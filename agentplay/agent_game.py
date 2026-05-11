#!/usr/bin/env python3
"""CLI entry point for agent-driven MTG replay games."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime
import random
import shlex
import subprocess
import sys
import time
from pathlib import Path
from typing import Sequence

DEFAULT_MTG_ARGS = ["decks/simple_bolt.dck", "decks/simple_bolt.dck"]
MODE_AGENT_VS_HEURISTIC = "agent-vs-heuristic"
MODE_AGENT_VS_RANDOM = "agent-vs-random"
MODE_AGENT_VS_AGENT = "agent-vs-agent"
MODE_RANDOM_VS_RANDOM = "random-vs-random"

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    from agentplay.lib.engine import GameEngine
    from agentplay.lib.prompts import (
        AgentDecision,
        build_choice_prompt,
        extract_bug_report,
        parse_agent_decision,
    )
    from agentplay.lib.card_defs import CardDatabase, find_mentioned_cards
else:
    from .lib.engine import GameEngine
    from .lib.prompts import (
        AgentDecision,
        build_choice_prompt,
        extract_bug_report,
        parse_agent_decision,
    )
    from .lib.card_defs import CardDatabase, find_mentioned_cards


@dataclass(frozen=True)
class HistoryEntry:
    """One decision plus the game log that led into it."""

    decision_number: int
    player: str
    controller_kind: str
    turn_number: int | None
    log_since_last_decision: str
    chosen_action: str
    reasoning: str


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Run a deterministic MTG game by asking Claude to choose each action.",
    )
    parser.add_argument("--seed", type=int, default=42, help="Deterministic game seed.")
    parser.add_argument(
        "--mode",
        choices=(
            MODE_AGENT_VS_HEURISTIC,
            MODE_AGENT_VS_RANDOM,
            MODE_AGENT_VS_AGENT,
            MODE_RANDOM_VS_RANDOM,
        ),
        default=MODE_AGENT_VS_HEURISTIC,
        help="Choose how each player's decisions are produced.",
    )
    parser.add_argument(
        "--game-dir",
        default=None,
        help="Game session directory under agentplay/ (default: next numbered directory).",
    )
    parser.add_argument(
        "--puzzle",
        default=None,
        help="Run `mtg puzzle <file>` instead of `mtg tui`.",
    )
    parser.add_argument(
        "--goal",
        default=None,
        help="Optional goal text passed into the choice prompt for directed play.",
    )
    parser.add_argument(
        "--scenario",
        default=None,
        help="English description of the gameplay scenario the agent should try to reproduce.",
    )
    parser.add_argument(
        "--bug-detection",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Prompt agents to stop and report gameplay bugs at each decision point (default: enabled).",
    )
    parser.add_argument(
        "--pure-play",
        dest="bug_detection",
        action="store_false",
        help="Disable bug-detection prompting and only ask agents to make gameplay choices.",
    )
    parser.add_argument("--verbose", "-v", action="store_true", help="Print replay and agent details.")
    parser.add_argument(
        "--max-turns",
        type=int,
        default=200,
        help="Safety limit on game turn number before aborting.",
    )
    parser.add_argument(
        "--stop-on-bug",
        action="store_true",
        help="Legacy alias: stop when an agent emits a BUG_REPORT section. Bug-detection mode already stops by default.",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Replace agent controllers with random choice selection (no API tokens burned).",
    )
    # Legacy alias
    parser.add_argument(
        "--continue-past-bug-reports",
        action="store_true",
        default=True,
        help=argparse.SUPPRESS,
    )
    parser.add_argument(
        "mtg_args",
        nargs=argparse.REMAINDER,
        help=(
            "Arguments passed to `mtg tui`; supply them after `--`, for example "
            "`-- decks/a.dck decks/b.dck`. Defaults to a simple bolt mirror match."
        ),
    )
    parser.add_argument(
        "--claude-args",
        action="append",
        default=[],
        help=(
            "Extra arguments to pass to claude CLI. May be repeated or given as a quoted "
            "string which will be shlex-split, e.g. --claude-args='--model sonnet'."
        ),
    )
    parser.add_argument(
        "--p1-draw",
        default=None,
        help="Override P1 initial hand (semicolon-separated card names, passed to mtg tui --p1-draw).",
    )
    parser.add_argument(
        "--p2-draw",
        default=None,
        help="Override P2 initial hand (semicolon-separated card names, passed to mtg tui --p2-draw).",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    mtg_args = list(args.mtg_args)
    if mtg_args and mtg_args[0] == "--":
        mtg_args = mtg_args[1:]
    if not mtg_args:
        mtg_args = list(DEFAULT_MTG_ARGS)
    if args.puzzle:
        mtg_args = [args.puzzle]

    # Pass through draw overrides to mtg tui
    if args.p1_draw:
        mtg_args.extend(["--p1-draw", args.p1_draw])
    if args.p2_draw:
        mtg_args.extend(["--p2-draw", args.p2_draw])

    # Flatten --claude-args: each entry may be a quoted string containing multiple tokens
    # (e.g. --claude-args='--model sonnet'), so shlex-split each one.
    claude_args: list[str] = []
    for raw in args.claude_args:
        claude_args.extend(shlex.split(raw))
    args.claude_args = claude_args
    if args.verbose and claude_args:
        print(f"[verbose] claude extra args: {claude_args}", file=sys.stderr)

    engine = GameEngine(seed=args.seed, game_dir=args.game_dir, verbose=args.verbose)
    engine.set_initial_args(mtg_args)
    if args.puzzle:
        engine.set_command("puzzle")
    rng = random.Random(args.seed)

    # Load card definitions from deck files
    repo_root = Path(__file__).resolve().parent.parent
    cardsfolder = repo_root / "forge-java" / "forge-gui" / "res" / "cardsfolder"
    card_db = CardDatabase(cardsfolder)
    for arg in mtg_args:
        deck_path = repo_root / arg
        if deck_path.suffix == ".dck" and deck_path.exists():
            card_db.load_deck(deck_path)
    all_card_names = card_db.all_names()
    seen_card_names: set[str] = set()

    # Rules references
    rules_dir = repo_root / "rules"
    rules_paths: list[str] = []
    if rules_dir.exists():
        for rf in sorted(rules_dir.iterdir()):
            if rf.is_file() and rf.suffix in (".txt", ".md"):
                rules_paths.append(str(rf))

    try:
        snapshot = engine.start_game()
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    choice_count = 0

    # Game log file for clean output (no agent commentary)
    game_log_path = engine.game_dir / "game.log"
    prev_log_lines: list[str] = []
    history_entries: list[HistoryEntry] = []

    while True:
        if engine.is_game_over(snapshot):
            new_lines = _new_log_tail_lines(snapshot.get("log_tail", ""), prev_log_lines)
            if new_lines:
                print(new_lines, file=sys.stderr, flush=True)
                _append_to_file(game_log_path, new_lines)
            print("Game over.", file=sys.stderr)
            return 0

        turn_number = _turn_number(snapshot)
        if turn_number is not None and turn_number > args.max_turns:
            print(f"Stopped: reached turn limit {args.max_turns}", file=sys.stderr)
            return 2

        choices = snapshot.get("choices", [])
        if not choices:
            print("Stopped: no available choices found in engine output", file=sys.stderr)
            return 1

        # Show new game log lines since last choice (deduped via log_tail comparison)
        new_lines = _new_log_tail_lines(snapshot.get("log_tail", ""), prev_log_lines)
        if new_lines:
            print(new_lines, file=sys.stderr, flush=True)
            _append_to_file(game_log_path, new_lines)
        prev_log_lines = snapshot.get("log_tail", "").splitlines()

        # Track which cards have been mentioned in the game
        log_text = snapshot.get("log_tail", "")
        newly_seen = find_mentioned_cards(log_text, all_card_names) - seen_card_names
        seen_card_names.update(newly_seen)

        # Build prompt with the full game log interleaved with prior choices and rationale.
        interleaved_history = _format_history(history_entries)
        previous_decision = _format_previous_decision(history_entries)
        card_defs_text = card_db.format_definitions(seen_card_names) if seen_card_names else None
        prompt_text = build_choice_prompt(
            snapshot.get("game_state", {}),
            choices,
            new_lines,
            goal=args.goal,
            scenario=args.scenario,
            interleaved_history=interleaved_history,
            previous_decision=previous_decision,
            card_definitions=card_defs_text,
            rules_paths=rules_paths if rules_paths else None,
            bug_detection=args.bug_detection,
        )
        before_snapshot = snapshot
        game_state_summary = _extract_game_state_summary(prompt_text)
        player = _player_name(snapshot.get("active_player"))
        controller_kind = _controller_for_player(args.mode, player, mock=args.mock)

        # Show choice point
        choice_display = "\n".join(
            f"  [{i}] {c}" for i, c in enumerate(["pass"] + list(choices))
        )
        print(
            f"--- {player} ({controller_kind}) | Turn {turn_number or '?'} | {len(choices)} choices ---",
            file=sys.stderr,
            flush=True,
        )
        print(choice_display, file=sys.stderr, flush=True)

        if controller_kind == "agent":
            print(
                f"========== Agent invoked for choice #{choice_count + 1} ==========",
                file=sys.stderr,
                flush=True,
            )

        t0 = time.time()
        try:
            decision = _choose_for_player(
                mode=args.mode,
                player=player,
                prompt_text=prompt_text,
                choice_count=len(choices),
                rng=rng,
                verbose=args.verbose,
                claude_args=args.claude_args,
                mock=args.mock,
                bug_detection=args.bug_detection,
            )
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 1
        elapsed = time.time() - t0
        raw_response = decision.raw_response

        if decision.stopped_for_bug:
            bug_report_path = _append_bug_report(
                engine.game_dir,
                player=player,
                turn_number=turn_number,
                bug_report_text=decision.bug_report
                or "(agent stopped for a suspected gameplay bug, but no details were provided)",
                raw_response=raw_response,
            )
            print(f"  [bug-report] logged to {bug_report_path}", file=sys.stderr)
            print(f"Stopped: {player} reported a suspected gameplay bug.", file=sys.stderr)
            return 0

        if decision.choice_number is None:
            print("Stopped: agent decision did not include a choice number", file=sys.stderr)
            return 1
        choice_number = decision.choice_number
        choice_text = "pass" if choice_number == 0 else choices[choice_number - 1]

        # Show decision
        if controller_kind == "agent":
            print(f"  => Agent chose [{choice_number}] {choice_text}", file=sys.stderr, flush=True)
            print(
                f"========== Agent responded in {elapsed:.1f}s ==========",
                file=sys.stderr,
                flush=True,
            )
            # Always show agent reasoning
            print(f"  [reasoning] {raw_response.strip()}", file=sys.stderr)
        else:
            print(f"  => {controller_kind.title()} chose [{choice_number}] {choice_text}", file=sys.stderr, flush=True)

        history_entries.append(
            HistoryEntry(
                decision_number=choice_count + 1,
                player=player,
                controller_kind=controller_kind,
                turn_number=turn_number,
                log_since_last_decision=new_lines,
                chosen_action=choice_text,
                reasoning=raw_response,
            )
        )

        bug_report_text = extract_bug_report(raw_response)
        if bug_report_text is not None:
            bug_report_path = _append_bug_report(
                engine.game_dir,
                player=player,
                turn_number=turn_number,
                bug_report_text=bug_report_text,
                raw_response=raw_response,
            )
            print(f"  [bug-report] logged to {bug_report_path}", file=sys.stderr)
            if args.stop_on_bug:
                print(
                    f"Stopped: BUG_REPORT detected in {player} response. Logged to {bug_report_path}",
                    file=sys.stderr,
                )
                return 0

        # Use text command (not numeric index) with wildcard prefix for replay resilience.
        # Text commands like "play Mountain" or "cast Lightning Bolt" match the right
        # choice point during replay even when priority auto-passes shift the sequence.
        engine.append_choice(player, choice_text)
        try:
            snapshot = engine.continue_game()
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 1

        engine.append_enriched_log(
            before_snapshot=before_snapshot,
            game_state_summary=game_state_summary,
            available_choices=choices,
            agent_response=raw_response,
            chosen_action=choice_text,
            after_snapshot=snapshot,
        )
        choice_count += 1

        if choice_count > 10000:
            print("Stopped: exceeded internal choice safety limit", file=sys.stderr)
            return 2


def _query_agent(
    prompt_text: str,
    choice_count: int,
    verbose: bool,
    claude_args: list[str] | None = None,
    *,
    bug_detection: bool = True,
) -> AgentDecision:
    extra_args = claude_args or []
    last_error = "no agent attempts made"
    for attempt in range(1, 4):
        retry_prompt = prompt_text
        if attempt > 1:
            valid_response = (
                f"Valid responses are either STOP with a BUG_REPORT, or a choice number from 0 to {choice_count}."
                if bug_detection
                else f"Valid choices are 0 to {choice_count}."
            )
            final_line = (
                "If choosing, the final line MUST be only a single number. If stopping, write STOP and BUG_REPORT instead."
                if bug_detection
                else f"You MUST respond with ONLY a single number between 0 and {choice_count} on the final line."
            )
            retry_prompt = (
                prompt_text
                + f"\n\nWARNING: Your previous response was invalid ({last_error}). "
                + valid_response
                + " "
                + final_line
            )
        cmd = ["claude"] + extra_args + ["-p", retry_prompt]
        if verbose:
            # Print command with the (potentially huge) prompt elided so the actual
            # argv layout is easy to eyeball without flooding the terminal.
            display_cmd = ["claude"] + extra_args + ["-p", f"<prompt {len(retry_prompt)} chars>"]
            print(
                f"[verbose] attempt {attempt}/3: $ {shlex.join(display_cmd)}",
                file=sys.stderr,
                flush=True,
            )
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=False,
        )
        response = completed.stdout.strip() or completed.stderr.strip()
        if verbose:
            print(
                f"[verbose] claude exit={completed.returncode} "
                f"stdout={len(completed.stdout)}B stderr={len(completed.stderr)}B",
                file=sys.stderr,
                flush=True,
            )
        if completed.returncode != 0:
            last_error = f"claude exited with code {completed.returncode}: {response}"
            if verbose:
                print(f"[retry {attempt}/3] {last_error}", file=sys.stderr)
            continue
        try:
            decision = parse_agent_decision(response, bug_detection=bug_detection)
        except ValueError as exc:
            last_error = str(exc)
            if verbose:
                print(f"[retry {attempt}/3] {last_error}", file=sys.stderr)
            continue
        if decision.stopped_for_bug:
            return decision
        if decision.choice_number is not None and 0 <= decision.choice_number <= choice_count:
            return decision
        last_error = f"parsed choice {decision.choice_number} is outside valid range 0..{choice_count}"
        if verbose:
            print(f"[retry {attempt}/3] {last_error}", file=sys.stderr)
    raise RuntimeError(f"failed to get a valid Claude choice after 3 attempts: {last_error}")


def _choose_for_player(
    mode: str,
    player: str,
    prompt_text: str,
    choice_count: int,
    rng: random.Random,
    verbose: bool,
    claude_args: list[str] | None = None,
    mock: bool = False,
    bug_detection: bool = True,
) -> AgentDecision:
    controller_kind = _controller_for_player(mode, player, mock=mock)
    if controller_kind == "agent":
        return _query_agent(
            prompt_text,
            choice_count,
            verbose,
            claude_args or [],
            bug_detection=bug_detection,
        )
    # Mock/random: pick locally, no subprocess call
    choice_number = rng.randint(0, choice_count)
    return AgentDecision(choice_number=choice_number, raw_response=f"{controller_kind} choice\n{choice_number}")


def _controller_for_player(mode: str, player: str, mock: bool = False) -> str:
    if mock:
        return "mock"
    if mode == MODE_AGENT_VS_AGENT:
        return "agent"
    if mode == MODE_RANDOM_VS_RANDOM:
        return "random"
    if player == "p1":
        return "agent"
    if mode in (MODE_AGENT_VS_HEURISTIC, MODE_AGENT_VS_RANDOM):
        return "random"
    raise ValueError(f"unsupported mode/player combination: {mode} {player}")


def _player_name(active_player: object) -> str:
    player_id = str(active_player)
    if player_id == "0":
        return "p1"
    if player_id == "1":
        return "p2"
    raise ValueError(f"unsupported active_player value in snapshot: {active_player!r}")


def _turn_number(snapshot: dict[str, object]) -> int | None:
    game_state = snapshot.get("game_state")
    if not isinstance(game_state, dict):
        return None
    root = game_state.get("game_state")
    if isinstance(root, dict):
        game_state = root
    turn = game_state.get("turn")
    if not isinstance(turn, dict):
        return None
    value = turn.get("turn_number")
    return value if isinstance(value, int) else None


def _extract_game_state_summary(prompt_text: str) -> str:
    start_marker = "Current game state:\n"
    end_marker = "\n\nInterleaved history so far:\n"
    if start_marker not in prompt_text or end_marker not in prompt_text:
        return prompt_text.strip()
    start = prompt_text.index(start_marker) + len(start_marker)
    end = prompt_text.index(end_marker, start)
    return prompt_text[start:end].strip()


def _format_history(history_entries: Sequence[HistoryEntry]) -> str:
    if not history_entries:
        return ""
    return "\n\n".join(_format_history_entry(entry) for entry in history_entries)


def _format_previous_decision(history_entries: Sequence[HistoryEntry]) -> str:
    if not history_entries:
        return ""
    entry = history_entries[-1]
    return "\n".join(
        [
            f"Decision #{entry.decision_number}: {entry.player} ({entry.controller_kind}) on turn {_format_turn(entry.turn_number)}",
            f"Chose: {entry.chosen_action}",
            "Reasoning:",
            _indent_text(entry.reasoning.strip() or "(no reasoning provided)"),
        ]
    )


def _format_history_entry(entry: HistoryEntry) -> str:
    return "\n".join(
        [
            f"## Decision #{entry.decision_number}: {entry.player} ({entry.controller_kind}) on turn {_format_turn(entry.turn_number)}",
            "Game log since previous decision:",
            _indent_text(entry.log_since_last_decision.strip() or "(no new game log lines)"),
            "Choice and rationale:",
            _indent_text(f"Chose: {entry.chosen_action}\n{entry.reasoning.strip() or '(no reasoning provided)'}"),
        ]
    )


def _format_turn(turn_number: int | None) -> str:
    return str(turn_number) if turn_number is not None else "?"


def _indent_text(text: str) -> str:
    return "\n".join(f"  {line}" if line else "" for line in text.splitlines())


def _append_bug_report(
    game_dir: Path,
    *,
    player: str,
    turn_number: int | None,
    bug_report_text: str,
    raw_response: str,
) -> Path:
    bug_report_path = game_dir / "bug_reports.log"
    timestamp = datetime.now().isoformat(timespec="seconds")
    with bug_report_path.open("a", encoding="utf-8") as handle:
        handle.write(f"[{timestamp}] player={player} turn={turn_number if turn_number is not None else '?'}\n")
        handle.write(bug_report_text.strip())
        handle.write("\n\n--- RAW RESPONSE ---\n")
        handle.write(raw_response.strip())
        handle.write("\n\n")
    return bug_report_path


def _new_log_tail_lines(current_tail: str, prev_lines: list[str]) -> str:
    """Extract lines from current log_tail that weren't in prev_lines.

    The engine replays from scratch and the mtg binary returns a bounded log
    tail, so older lines may roll off. Match the largest overlap between the
    previous tail suffix and current tail prefix, then return the new suffix.
    """
    if not current_tail:
        return ""
    curr = current_tail.splitlines()
    if not prev_lines:
        return current_tail

    if len(curr) <= len(prev_lines) and prev_lines[-len(curr) :] == curr:
        return ""

    max_overlap = min(len(prev_lines), len(curr))
    overlap = 0
    for size in range(max_overlap, 0, -1):
        if prev_lines[-size:] == curr[:size]:
            overlap = size
            break
    new = curr[overlap:]
    return "\n".join(new) if new else ""


def _append_to_file(path: Path, text: str) -> None:
    """Append text to a file."""
    with path.open("a", encoding="utf-8") as f:
        f.write(text)
        f.write("\n")


if __name__ == "__main__":
    raise SystemExit(main())
