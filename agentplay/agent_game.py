"""CLI entry point for agent-driven MTG replay games."""

from __future__ import annotations

import argparse
import random
import subprocess
import sys
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

    while True:
        if engine.is_game_over(snapshot):
            if args.verbose:
                print(snapshot.get("log_tail", ""))
            return 0

        turn_number = _turn_number(snapshot)
        if turn_number is not None and turn_number > args.max_turns:
            print(f"Stopped: reached turn limit {args.max_turns}", file=sys.stderr)
            return 2

        choices = snapshot.get("choices", [])
        if not choices:
            print("Stopped: no available choices found in engine output", file=sys.stderr)
            return 1

        prompt_text = build_choice_prompt(
            snapshot.get("game_state", {}),
            choices,
            snapshot.get("log_tail", ""),
            goal=args.goal,
        )
        before_snapshot = snapshot
        game_state_summary = _extract_game_state_summary(prompt_text)
        try:
            choice_number, raw_response = _choose_for_player(
                mode=args.mode,
                player=_player_name(snapshot.get("active_player")),
                prompt_text=prompt_text,
                choice_count=len(choices),
                rng=rng,
                verbose=args.verbose,
            )
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 1
        player = _player_name(snapshot.get("active_player"))
        choice_text = "pass" if choice_number == 0 else choices[choice_number - 1]

        if args.verbose:
            print(f"[turn {turn_number if turn_number is not None else '?'}] {player} -> {choice_number}: {choice_text}")
            print(f"[claude] {raw_response.strip()}")

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
            ["with-proxy", "claude", "-p", prompt_text],
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


if __name__ == "__main__":
    raise SystemExit(main())
