#!/usr/bin/env python3
"""CLI entry point for agent-driven MTG replay games."""

from __future__ import annotations

import argparse
from datetime import datetime
import random
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
    from agentplay.engine import GameEngine
    from agentplay.prompts import build_choice_prompt, parse_agent_response
else:
    from .engine import GameEngine
    from .prompts import build_choice_prompt, parse_agent_response


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
        help="Stop the game when an agent emits a BUG_REPORT section (default: continue playing).",
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

    engine = GameEngine(seed=args.seed, game_dir=args.game_dir, verbose=args.verbose)
    engine.set_initial_args(mtg_args)
    if args.puzzle:
        engine.set_command("puzzle")
    rng = random.Random(args.seed)

    try:
        snapshot = engine.start_game()
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    choice_count = 0

    # Game log file for clean output (no agent commentary)
    game_log_path = engine.game_dir / "game.log"
    prev_output_line_count = 0
    # Interleaved log: game events + agent reasoning for prompt context
    interleaved_log_parts: list[str] = []

    while True:
        if engine.is_game_over(snapshot):
            # Print final new log lines and write to file
            new_lines = _extract_new_output(snapshot, prev_output_line_count)
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

        # Show new game log lines since last choice (deduped by line count)
        new_lines = _extract_new_output(snapshot, prev_output_line_count)
        if new_lines:
            print(new_lines, file=sys.stderr, flush=True)
            _append_to_file(game_log_path, new_lines)
            interleaved_log_parts.append(new_lines)
        prev_output_line_count = _output_line_count(snapshot)

        # Build prompt with interleaved log (game events + prior agent reasoning)
        interleaved_log_text = "\n".join(interleaved_log_parts[-20:])  # Last 20 chunks
        prompt_text = build_choice_prompt(
            snapshot.get("game_state", {}),
            choices,
            interleaved_log_text,
            goal=args.goal,
        )
        before_snapshot = snapshot
        game_state_summary = _extract_game_state_summary(prompt_text)
        player = _player_name(snapshot.get("active_player"))
        controller_kind = _controller_for_player(args.mode, player)

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
            choice_number, raw_response = _choose_for_player(
                mode=args.mode,
                player=player,
                prompt_text=prompt_text,
                choice_count=len(choices),
                rng=rng,
                verbose=args.verbose,
            )
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 1
        elapsed = time.time() - t0
        choice_text = "pass" if choice_number == 0 else choices[choice_number - 1]

        # Show decision
        if controller_kind == "agent":
            print(f"  => Agent chose [{choice_number}] {choice_text}", file=sys.stderr, flush=True)
            print(
                f"========== Agent responded in {elapsed:.1f}s ==========",
                file=sys.stderr,
                flush=True,
            )
            if args.verbose:
                print(f"  [reasoning] {raw_response.strip()}", file=sys.stderr)
            # Add agent reasoning to interleaved log for future prompts
            reasoning_summary = raw_response.strip().split("\n")[0][:200]  # First line, truncated
            interleaved_log_parts.append(f"[Agent chose: {choice_text}. Reasoning: {reasoning_summary}]")
        else:
            print(f"  => Random chose [{choice_number}] {choice_text}", file=sys.stderr, flush=True)
            interleaved_log_parts.append(f"[Random chose: {choice_text}]")

        bug_report_text = _extract_bug_report(raw_response)
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


def _query_agent(prompt_text: str, choice_count: int, verbose: bool) -> tuple[int, str]:
    last_error = "no agent attempts made"
    for attempt in range(1, 4):
        completed = subprocess.run(
            ["claude", "-p", prompt_text],
            capture_output=True,
            text=True,
            check=False,
        )
        response = completed.stdout.strip() or completed.stderr.strip()
        if completed.returncode != 0:
            last_error = f"claude exited with code {completed.returncode}: {response}"
            if verbose:
                print(f"[retry {attempt}/3] {last_error}", file=sys.stderr)
            continue
        try:
            choice_number = parse_agent_response(response)
        except ValueError as exc:
            last_error = str(exc)
            if verbose:
                print(f"[retry {attempt}/3] {last_error}", file=sys.stderr)
            continue
        if 0 <= choice_number <= choice_count:
            return choice_number, response
        last_error = f"parsed choice {choice_number} is outside valid range 0..{choice_count}"
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
) -> tuple[int, str]:
    controller_kind = _controller_for_player(mode, player)
    if controller_kind == "agent":
        return _query_agent(prompt_text, choice_count, verbose)
    # Random/heuristic: pick locally, no subprocess call
    # Clamp to valid range: 0=pass, 1..choice_count=actions
    choice_number = rng.randint(0, choice_count)
    return choice_number, f"{controller_kind} choice\n{choice_number}"


def _controller_for_player(mode: str, player: str) -> str:
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
    end_marker = "\n\nAvailable choices:\n"
    if start_marker not in prompt_text or end_marker not in prompt_text:
        return prompt_text.strip()
    start = prompt_text.index(start_marker) + len(start_marker)
    end = prompt_text.index(end_marker, start)
    return prompt_text[start:end].strip()


def _extract_bug_report(response: str) -> str | None:
    marker = "BUG_REPORT"
    if marker not in response:
        return None
    _, bug_report = response.split(marker, 1)
    return bug_report.lstrip(" :\n\t").strip() or "(BUG_REPORT marker present, but no details were provided)"


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


def _extract_new_output(snapshot: dict, prev_line_count: int) -> str:
    """Extract new output lines from engine stdout since last check."""
    raw = snapshot.get("raw_output", "")
    if not raw:
        return ""
    all_lines = raw.splitlines()
    if prev_line_count >= len(all_lines):
        return ""
    return "\n".join(all_lines[prev_line_count:])


def _output_line_count(snapshot: dict) -> int:
    """Count total lines in engine raw output."""
    raw = snapshot.get("raw_output", "")
    return len(raw.splitlines()) if raw else 0


def _append_to_file(path: Path, text: str) -> None:
    """Append text to a file."""
    with path.open("a", encoding="utf-8") as f:
        f.write(text)
        f.write("\n")


if __name__ == "__main__":
    raise SystemExit(main())
