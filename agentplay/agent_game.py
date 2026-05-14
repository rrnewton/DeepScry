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

# Default LLM. Haiku is the cheapest Claude model and typically suffices
# for the structured "pick a number from a menu" decisions agentplay
# makes; users can opt into sonnet/opus for harder bug-detection runs
# via `--model`.
DEFAULT_MODEL = "haiku"
# Convenience aliases that map to Anthropic model identifiers. Anything
# not listed here is passed through to `claude --model <value>` verbatim
# so future Claude releases (or full model IDs like
# `claude-3-5-sonnet-20241022`) work without code changes.
MODEL_ALIASES = {
    "haiku": "haiku",
    "sonnet": "sonnet",
    "opus": "opus",
}

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    from agentplay.lib.engine import GameEngine, new_log_tail_lines
    from agentplay.lib.prompts import (
        AgentDecision,
        build_choice_prompt,
        build_intro_section,
        extract_bug_report,
        format_deck_preamble,
        parse_agent_decision,
    )
    from agentplay.lib.card_defs import CardDatabase, find_mentioned_cards
    from agentplay.lib.game_process import ChoicePoint, GameOver, NativeTuiProcess
    from agentplay.lib.agent_session import (
        AgentSession,
        ClaudeOneShotSession,
        ClaudeResumeSession,
        MockSession,
    )
else:
    from .lib.engine import GameEngine, new_log_tail_lines
    from .lib.prompts import (
        AgentDecision,
        build_choice_prompt,
        build_intro_section,
        extract_bug_report,
        format_deck_preamble,
        parse_agent_decision,
    )
    from .lib.card_defs import CardDatabase, find_mentioned_cards
    from .lib.game_process import ChoicePoint, GameOver, NativeTuiProcess
    from .lib.agent_session import (
        AgentSession,
        ClaudeOneShotSession,
        ClaudeResumeSession,
        MockSession,
    )


# Engine driver modes selected via `--driver`. The persistent mode keeps a
# single `mtg tui --p1=tui` subprocess alive and feeds choices via stdin; the
# legacy stop-and-go mode re-runs the entire game from scratch on every
# decision via `--p1=fixed --p2=fixed`. Both produce the same on-disk
# artefacts (pN_choices.txt, snapshot.json, game.log, enriched_log.md).
DRIVER_PERSISTENT = "persistent"
DRIVER_STOP_AND_GO = "stop-and-go"


# Re-exported here so existing callers (and tests) can keep importing
# `_new_log_tail_lines` from this module while the implementation lives
# alongside the engine helpers.
_new_log_tail_lines = new_log_tail_lines


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
        help="Run `mtg tui --start-state PUZZLE.pzl` to load a puzzle file as the starting state.",
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
    parser.add_argument(
        "--decklists",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Include both players' full deck lists as a preamble in the agent prompt (default: enabled).",
    )
    parser.add_argument(
        "--model",
        default=DEFAULT_MODEL,
        help=(
            "LLM to use for agent decisions. Recognised aliases: "
            f"{', '.join(sorted(MODEL_ALIASES))}. Any other value is passed "
            f"through to `claude --model <value>` unchanged. Default: {DEFAULT_MODEL}."
        ),
    )
    parser.add_argument(
        "--driver",
        choices=(DRIVER_PERSISTENT, DRIVER_STOP_AND_GO),
        default=DRIVER_PERSISTENT,
        help=(
            "Engine driver. `persistent` keeps one `mtg tui --p1=tui` process "
            "alive and pipes choices via stdin; `stop-and-go` re-runs the engine "
            "from scratch on every decision (legacy default before persistent "
            "mode existed). Both produce identical on-disk artefacts."
        ),
    )
    parser.add_argument(
        "--persistent-claude",
        choices=("resume", "oneshot"),
        default="resume",
        help=(
            "How the persistent driver talks to Claude. `resume` (default) "
            "keeps one `claude --resume <session>` conversation per player so "
            "follow-up turns reuse context; `oneshot` re-invokes "
            "`claude -p` per turn (matching the stop-and-go cost profile). "
            "Has no effect when --driver=stop-and-go."
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
        mtg_args = mtg_args + ["--start-state", args.puzzle]

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

    # Prepend the resolved --model selection unless the user already set
    # one explicitly via --claude-args='--model X' (in which case we
    # respect that, since it's the more specific override).
    model_value = MODEL_ALIASES.get(args.model, args.model)
    if "--model" not in claude_args:
        claude_args = ["--model", model_value] + claude_args
    args.claude_args = claude_args
    if args.verbose:
        print(f"[verbose] claude extra args: {claude_args}", file=sys.stderr)

    engine = GameEngine(seed=args.seed, game_dir=args.game_dir, verbose=args.verbose)
    engine.set_initial_args(mtg_args)
    rng = random.Random(args.seed)

    # Load card definitions from deck files. Track each .dck path in the order
    # it appeared so we can label them as P1/P2 in the deck list preamble.
    repo_root = Path(__file__).resolve().parent.parent
    cardsfolder = repo_root / "forge-java" / "forge-gui" / "res" / "cardsfolder"
    card_db = CardDatabase(cardsfolder)
    deck_paths: list[Path] = []
    for arg in mtg_args:
        deck_path = repo_root / arg
        if deck_path.suffix == ".dck" and deck_path.exists():
            card_db.load_deck(deck_path)
            deck_paths.append(deck_path)
    all_card_names = card_db.all_names()
    seen_card_names: set[str] = set()

    # Build deck-list preamble (default on; opt out with --no-decklists). The
    # convention is that the first .dck arg is P1 and the second is P2.
    deck_preamble: str | None = None
    if args.decklists and deck_paths:
        labelled: list[tuple[str, Path]] = []
        for index, path in enumerate(deck_paths[:2]):
            labelled.append((f"P{index + 1}", path))
        deck_preamble = format_deck_preamble(labelled) or None

    # Rules references
    rules_dir = repo_root / "rules"
    rules_paths: list[str] = []
    if rules_dir.exists():
        for rf in sorted(rules_dir.iterdir()):
            if rf.is_file() and rf.suffix in (".txt", ".md"):
                rules_paths.append(str(rf))

    # Echo the static intro/system-prompt portion to stdout once at startup so
    # the human running the harness can see exactly what the agent has been
    # told (role, scenario, deck lists, rules references). Per-decision prompts
    # are NOT echoed because they are repetitive and noisy.
    intro_text = build_intro_section(
        scenario=args.scenario,
        goal=args.goal,
        bug_detection=args.bug_detection,
        deck_preamble=deck_preamble,
        rules_paths=rules_paths if rules_paths else None,
    )
    print("===== Agent intro prompt =====", flush=True)
    print(intro_text, flush=True)
    print("===== End agent intro prompt =====", flush=True)

    if args.driver == DRIVER_PERSISTENT:
        # Hand off to the persistent driver. It manages its own subprocess
        # lifecycle and on-disk artefacts, but reuses every prompt-building
        # helper above so the agent sees identical content.
        return _run_persistent(
            args=args,
            mtg_args=mtg_args,
            engine=engine,
            card_db=card_db,
            all_card_names=all_card_names,
            seen_card_names=seen_card_names,
            deck_preamble=deck_preamble,
            rules_paths=rules_paths,
            intro_text=intro_text,
            repo_root=repo_root,
        )

    try:
        snapshot = engine.start_game()
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    choice_count = 0

    # Game log file for clean output (no agent commentary)
    game_log_path = engine.game_dir / "game.log"
    # `printed_log_lines` is the cumulative record of every log line we've
    # already shown to the user. We dedup the engine's freshly-replayed
    # log_tail against THIS rather than the previous iteration's log_tail
    # alone, because diff'ing against the cumulative record is the only
    # way to suppress repeats when the engine re-emits an earlier-turn
    # block in a later snapshot (e.g. a discard event being re-logged
    # after a downstream replay diverges).
    game_log_path.touch()
    printed_log_lines: list[str] = []
    history_entries: list[HistoryEntry] = []

    while True:
        if engine.is_game_over(snapshot):
            new_lines = _new_log_tail_lines(snapshot.get("log_tail", ""), printed_log_lines)
            if new_lines:
                print(new_lines, file=sys.stderr, flush=True)
                _append_to_file(game_log_path, new_lines)
                printed_log_lines.extend(new_lines.splitlines())
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

        # Show new game log lines since last choice. Dedup against the full
        # cumulative printed log so that nothing we've already shown the
        # user gets re-emitted.
        new_lines = _new_log_tail_lines(snapshot.get("log_tail", ""), printed_log_lines)
        if new_lines:
            print(new_lines, file=sys.stderr, flush=True)
            _append_to_file(game_log_path, new_lines)
            printed_log_lines.extend(new_lines.splitlines())

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
            deck_preamble=deck_preamble,
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


# ---------------------------------------------------------------------------
# Persistent driver
# ---------------------------------------------------------------------------


def _run_persistent(
    *,
    args: argparse.Namespace,
    mtg_args: Sequence[str],
    engine: GameEngine,
    card_db: CardDatabase,
    all_card_names: set[str],
    seen_card_names: set[str],
    deck_preamble: str | None,
    rules_paths: list[str],
    intro_text: str,
    repo_root: Path,
) -> int:
    """Drive a game using ONE long-running `mtg tui --p1=tui` subprocess.

    Mirrors the per-decision artefacts and prompt structure of the legacy
    stop-and-go loop so a game played here can be inspected, replayed, or
    diffed against one played in the legacy mode. Differences:

    * Engine is started ONCE; choices are piped via stdin instead of being
      replayed via `--p1-fixed-inputs`.
    * The structured `game_state` JSON used by `build_choice_prompt` is read
      from a per-prompt snapshot the engine writes via `--tui-snapshot-path`
      (added in `mtg-engine/src/game/interactive_controller.rs`).
    * Each `agent` decision goes through an `AgentSession` that may keep a
      persistent `claude --resume <session>` conversation alive across turns.
    """

    # ------------------------------------------------------------------
    # Set up artefact paths (parity with stop-and-go mode)
    # ------------------------------------------------------------------
    engine.game_dir.mkdir(parents=True, exist_ok=True)
    # Touch the same files the legacy mode creates so downstream tooling
    # (e.g. continue_game.py) can find them.
    engine.p1_choices_path.touch()
    engine.p2_choices_path.touch()
    # initial_args.txt lets a follow-up stop-and-go run resume from this
    # exact game configuration.
    if not engine.initial_args_path.exists():
        engine.initial_args_path.write_text(
            "\n".join(str(a) for a in mtg_args) + "\n", encoding="utf-8"
        )
    game_log_path = engine.game_dir / "game.log"
    game_log_path.touch()
    printed_log_lines: list[str] = []

    rng = random.Random(args.seed)

    # ------------------------------------------------------------------
    # Resolve where the engine binary and cardsfolder live
    # ------------------------------------------------------------------
    binary_path = engine.binary_path
    if not binary_path.exists():
        print(
            f"Error: MTG engine binary not found at {binary_path}\n"
            "Build it with: cargo build --release",
            file=sys.stderr,
        )
        return 1
    cardsfolder: Path | None = None
    for candidate in (engine.cardsfolder_path, engine.forge_cardsfolder_path):
        if candidate.exists() and all((candidate / letter).is_dir() for letter in ("a", "b", "c")):
            cardsfolder = candidate
            break

    # ------------------------------------------------------------------
    # Pick controller types for each player based on --mode/--mock
    # ------------------------------------------------------------------
    p1_kind = _controller_for_player(args.mode, "p1", mock=args.mock)
    p2_kind = _controller_for_player(args.mode, "p2", mock=args.mock)
    # The engine needs exactly one of: tui|heuristic|random|fixed for each
    # player. "agent" is a Python-side concept implemented as an
    # InteractiveController on the engine side that we drive over stdin.
    p1_engine = _engine_controller_for_kind(p1_kind)
    p2_engine = _engine_controller_for_kind(p2_kind)

    # ------------------------------------------------------------------
    # Spawn the persistent subprocess
    # ------------------------------------------------------------------
    proc = NativeTuiProcess(
        binary=binary_path,
        mtg_args=mtg_args,
        game_dir=engine.game_dir,
        seed=args.seed,
        p1_controller=p1_engine,
        p2_controller=p2_engine,
        cardsfolder=cardsfolder,
        cwd=repo_root,
        verbose=args.verbose,
    )

    # ------------------------------------------------------------------
    # Build per-player AgentSession objects (only the ones we need)
    # ------------------------------------------------------------------
    sessions: dict[str, AgentSession] = {}
    if args.mock:
        sessions["p1"] = MockSession(seed=args.seed, label="p1-mock")
        sessions["p2"] = MockSession(seed=args.seed + 1, label="p2-mock")
    else:
        if p1_kind == "agent":
            sessions["p1"] = _build_agent_session(args, intro_text, label="p1")
        if p2_kind == "agent":
            sessions["p2"] = _build_agent_session(args, intro_text, label="p2")

    # ------------------------------------------------------------------
    # Run the loop
    # ------------------------------------------------------------------
    history_entries: list[HistoryEntry] = []
    choice_count = 0
    try:
        event = proc.start()
        while isinstance(event, ChoicePoint):
            player = event.player
            controller_kind = _controller_for_player(args.mode, player, mock=args.mock)
            turn_number = event.turn_number

            # Safety: turn limit
            if turn_number is not None and turn_number > args.max_turns:
                print(f"Stopped: reached turn limit {args.max_turns}", file=sys.stderr)
                return 2

            choices = list(event.choices)
            if not choices:
                # No menu items besides pass — the snapshot probably gives
                # us 0 actions; surface and stop, matching legacy behaviour.
                print("Stopped: no available choices found in engine output", file=sys.stderr)
                return 1

            # Cumulative dedup against everything we've already shown the
            # user — same scheme as stop-and-go mode (which uses
            # _new_log_tail_lines on the engine's log_tail). Here our
            # source of truth is `event.log_lines` (incremental, already
            # filtered by NativeTuiProcess._maybe_record_log_line).
            new_text_block = "\n".join(event.log_lines).strip()
            if new_text_block:
                print(new_text_block, file=sys.stderr, flush=True)
                _append_to_file(game_log_path, new_text_block)
                printed_log_lines.extend(new_text_block.splitlines())

            # Track which cards have been mentioned in the game so we can
            # show their definitions in the prompt.
            full_log_so_far = "\n".join(printed_log_lines)
            newly_seen = find_mentioned_cards(full_log_so_far, all_card_names) - seen_card_names
            seen_card_names.update(newly_seen)

            # Build the same prompt the stop-and-go loop builds.
            interleaved_history = _format_history(history_entries)
            previous_decision = _format_previous_decision(history_entries)
            card_defs_text = (
                card_db.format_definitions(seen_card_names) if seen_card_names else None
            )
            # `event.snapshot` is the full GameSnapshot dict (game_state +
            # turn_number + ...); build_choice_prompt's `_snapshot_root`
            # already handles either wrapping shape.
            prompt_text = build_choice_prompt(
                event.snapshot,
                choices,
                new_text_block,
                goal=args.goal,
                scenario=args.scenario,
                interleaved_history=interleaved_history,
                previous_decision=previous_decision,
                card_definitions=card_defs_text,
                rules_paths=rules_paths if rules_paths else None,
                bug_detection=args.bug_detection,
                deck_preamble=deck_preamble,
            )
            game_state_summary = _extract_game_state_summary(prompt_text)
            before_snapshot = event.snapshot

            # Show the choice point header (parity with legacy mode).
            choice_display = "\n".join(
                f"  [{i}] {c}" for i, c in enumerate(["pass"] + choices)
            )
            print(
                f"--- {player} ({controller_kind}) | Turn {turn_number or '?'} | "
                f"{len(choices)} choices ---",
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

            # Get a decision.
            t0 = time.time()
            try:
                decision = _persistent_choose(
                    controller_kind=controller_kind,
                    player=player,
                    sessions=sessions,
                    prompt_text=prompt_text,
                    choice_count=len(choices),
                    rng=rng,
                    bug_detection=args.bug_detection,
                )
            except RuntimeError as exc:
                print(str(exc), file=sys.stderr)
                return 1
            elapsed = time.time() - t0
            raw_response = decision.raw_response

            # Bug-report handling (mirrors legacy mode).
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
                print(
                    f"Stopped: {player} reported a suspected gameplay bug.",
                    file=sys.stderr,
                )
                return 0

            if decision.choice_number is None:
                print(
                    "Stopped: agent decision did not include a choice number",
                    file=sys.stderr,
                )
                return 1
            choice_number = decision.choice_number
            if choice_number == 0:
                choice_text = "pass"
            else:
                if choice_number > len(choices):
                    # Defensive guard; AgentSession should already filter this.
                    print(
                        f"Stopped: choice {choice_number} out of range (1..{len(choices)})",
                        file=sys.stderr,
                    )
                    return 1
                choice_text = choices[choice_number - 1]

            if controller_kind == "agent":
                print(
                    f"  => Agent chose [{choice_number}] {choice_text}",
                    file=sys.stderr,
                    flush=True,
                )
                print(
                    f"========== Agent responded in {elapsed:.1f}s ==========",
                    file=sys.stderr,
                    flush=True,
                )
                print(f"  [reasoning] {raw_response.strip()}", file=sys.stderr)
            else:
                print(
                    f"  => {controller_kind.title()} chose [{choice_number}] {choice_text}",
                    file=sys.stderr,
                    flush=True,
                )

            history_entries.append(
                HistoryEntry(
                    decision_number=choice_count + 1,
                    player=player,
                    controller_kind=controller_kind,
                    turn_number=turn_number,
                    log_since_last_decision=new_text_block,
                    chosen_action=choice_text,
                    reasoning=raw_response,
                )
            )

            # Inline bug-report extraction (same as legacy mode).
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

            # Persist the chosen action to pN_choices.txt — this gives us
            # cross-driver replay parity. A game played with --driver=
            # persistent can be replayed with --driver=stop-and-go from the
            # same game_dir.
            engine.append_choice(player, choice_text)

            # Send the choice to the engine. We send the TEXT command (e.g.
            # "play Mountain") rather than the index because:
            # (a) it matches what the legacy mode writes to pN_choices.txt
            #     (the engine.py `*;<text>` script form), and
            # (b) the InteractiveController accepts rich text commands
            #     (controller.rs:parse_spell_ability_choice).
            event = proc.send_choice(player, choice_text)

            engine.append_enriched_log(
                before_snapshot=before_snapshot,
                game_state_summary=game_state_summary,
                available_choices=choices,
                agent_response=raw_response,
                chosen_action=choice_text,
                after_snapshot=event.snapshot if isinstance(event, ChoicePoint) else {},
            )
            choice_count += 1

            if choice_count > 10000:
                print(
                    "Stopped: exceeded internal choice safety limit",
                    file=sys.stderr,
                )
                return 2

        # Drained out of the ChoicePoint loop ⇒ engine reported game over.
        assert isinstance(event, GameOver)
        if event.fresh_output.strip():
            print(event.fresh_output, file=sys.stderr, flush=True)
            _append_to_file(game_log_path, event.fresh_output)
        if event.log_lines:
            tail = "\n".join(event.log_lines)
            _append_to_file(game_log_path, tail)
        print("Game over.", file=sys.stderr)
        return 0
    finally:
        for sess in sessions.values():
            sess.close()
        proc.close()


def _engine_controller_for_kind(kind: str) -> str:
    """Map an agentplay 'controller_kind' to an `mtg tui --p?=` value.

    The persistent driver only knows two engine-side identities for a
    player: `tui` (Python pipes choices over stdin) or one of the engine's
    built-in controllers (heuristic/random/zero/etc.). `agent` and `mock`
    are Python-side concepts implemented as an InteractiveController.
    """

    if kind in ("agent", "mock"):
        return "tui"
    if kind in ("heuristic", "random", "zero"):
        return kind
    raise ValueError(f"unsupported controller kind for persistent driver: {kind!r}")


def _build_agent_session(
    args: argparse.Namespace, intro_text: str, *, label: str
) -> AgentSession:
    """Construct a per-player AgentSession according to --persistent-claude."""

    if args.persistent_claude == "oneshot":
        return ClaudeOneShotSession(
            claude_args=args.claude_args,
            verbose=args.verbose,
            label=label,
        )
    return ClaudeResumeSession(
        intro_text=intro_text,
        claude_args=args.claude_args,
        verbose=args.verbose,
        label=label,
    )


def _persistent_choose(
    *,
    controller_kind: str,
    player: str,
    sessions: dict[str, AgentSession],
    prompt_text: str,
    choice_count: int,
    rng: random.Random,
    bug_detection: bool,
) -> AgentDecision:
    """Resolve a single decision in persistent mode.

    For `agent`/`mock`, defer to the per-player AgentSession. For
    `heuristic`/`random`, the engine itself is making the decision and the
    Python harness should never get a ChoicePoint for that player — we
    raise loudly if we do.
    """

    if controller_kind in ("agent", "mock"):
        sess = sessions.get(player)
        if sess is None:
            raise RuntimeError(
                f"persistent driver: no AgentSession registered for {player!r}"
            )
        return sess.ask(
            prompt_text,
            choice_count,
            bug_detection=bug_detection,
        )
    # In random/heuristic engine-side modes the InteractiveController is
    # never spawned for that player, so the engine should never ask us for
    # a choice on their behalf. Pick locally as a defensive fallback (same
    # as legacy stop-and-go behaviour).
    choice_number = rng.randint(0, choice_count)
    return AgentDecision(
        choice_number=choice_number,
        raw_response=f"{controller_kind} choice\n{choice_number}",
    )


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


def _append_to_file(path: Path, text: str) -> None:
    """Append text to a file."""
    with path.open("a", encoding="utf-8") as f:
        f.write(text)
        f.write("\n")


if __name__ == "__main__":
    raise SystemExit(main())
