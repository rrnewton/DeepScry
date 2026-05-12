#!/usr/bin/env python3
"""Continue an agentplay game session with one choice.

Thin CLI wrapper around GameEngine.continue_game().
Replaces the former continue_game.sh shell script.

Usage:
    ./agentplay/continue_game.py p1 "play Mountain"
    ./agentplay/continue_game.py p2 "0"
    ./agentplay/continue_game.py --game-dir=042.game p1 "cast Lightning Bolt"

Output:
    Prints only the NEW game log lines emitted since the previous
    choice point (everything we've already shown the agent is recorded
    in `<game-dir>/game.log` and is suppressed from this output), then
    the upcoming choice menu with turn / step / choice-context context.
    The full game-state JSON is NOT printed -- it is saved to
    `<game-dir>/snapshot.json` if you need it.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agentplay.lib.engine import GameEngine, new_log_tail_lines
from agentplay.lib.session_io import (
    print_choice_block,
    print_log_segment,
    record_log_segment,
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Continue an agentplay game with one choice.")
    parser.add_argument("--game-dir", default=None, help="Game directory (default: latest numbered dir).")
    parser.add_argument("--seed", type=int, default=42, help="Deterministic game seed (default: 42).")
    parser.add_argument("--verbose", "-v", action="store_true", help="Print replay command.")
    parser.add_argument("player", choices=["p1", "p2"], help="Which player is making the choice.")
    parser.add_argument("choice", help="The choice text or index (e.g. 'play Mountain' or '1').")
    args = parser.parse_args(argv)

    # Find an existing game dir if none specified
    game_dir = args.game_dir
    if game_dir is None:
        agentplay_dir = Path(__file__).resolve().parent
        game_dirs = sorted(agentplay_dir.glob("*.game"))
        game_dirs = [d for d in game_dirs if d.is_dir() and not d.is_symlink()]
        if not game_dirs:
            print("Error: no game directory found. Run start_game.py first.", file=sys.stderr)
            return 1
        game_dir = str(game_dirs[-1])

    engine = GameEngine(seed=args.seed, game_dir=game_dir, verbose=args.verbose)
    engine.append_choice(args.player, args.choice)

    print(f"Continuing game in {engine.game_dir.name}: {args.player} chose '{args.choice}'")
    try:
        snapshot = engine.continue_game()
    except RuntimeError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    log_path = engine.game_dir / "game.log"
    already_printed: list[str] = []
    if log_path.exists():
        already_printed = log_path.read_text(encoding="utf-8").splitlines()

    new_lines = new_log_tail_lines(snapshot.get("log_tail", ""), already_printed)
    print_log_segment(new_lines)
    record_log_segment(log_path, new_lines)

    if engine.is_game_over(snapshot):
        print("Game over.")
        return 0

    print_choice_block(
        snapshot,
        choice_number=engine.total_choices_made() + 1,
        game_dir_name=engine.game_dir.name,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
