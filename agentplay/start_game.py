#!/usr/bin/env python3
"""Start a new agentplay game session.

Thin CLI wrapper around GameEngine.start_game().
Replaces the former start_game.sh shell script.

Usage:
    ./agentplay/start_game.py decks/simple_bolt.dck decks/simple_bolt.dck
    ./agentplay/start_game.py --game-dir=my_test.game decks/a.dck decks/b.dck
    ./agentplay/start_game.py --seed=7 decks/a.dck decks/b.dck

Output:
    The script prints the cumulative game log followed by the upcoming
    choice menu, with turn / step / choice-context context lines so an
    agent (or human) can immediately tell what is happening. The full
    game-state JSON is NOT echoed to stdout: it is already saved to
    `<game-dir>/snapshot.json` and re-reading the file is the supported
    way to inspect detailed state.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agentplay.lib.engine import GameEngine
from agentplay.lib.session_io import (
    print_choice_block,
    print_log_segment,
    record_log_segment,
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Start a new agentplay game session.")
    parser.add_argument("--game-dir", default=None, help="Game directory (default: next numbered dir).")
    parser.add_argument("--seed", type=int, default=42, help="Deterministic game seed (default: 42).")
    parser.add_argument("--verbose", "-v", action="store_true", help="Print replay command.")
    parser.add_argument("mtg_args", nargs=argparse.REMAINDER, help="Arguments for mtg tui.")
    args = parser.parse_args(argv)

    mtg_args = list(args.mtg_args)
    if mtg_args and mtg_args[0] == "--":
        mtg_args = mtg_args[1:]
    if not mtg_args:
        print("Error: provide deck files or mtg tui arguments.", file=sys.stderr)
        parser.print_usage(sys.stderr)
        return 1

    engine = GameEngine(seed=args.seed, game_dir=args.game_dir, verbose=args.verbose)
    engine.set_initial_args(mtg_args)

    print(f"Starting new game in {engine.game_dir.name}")
    try:
        snapshot = engine.start_game()
    except (RuntimeError, FileExistsError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    log_tail = snapshot.get("log_tail", "")
    print_log_segment(log_tail)
    record_log_segment(engine.game_dir / "game.log", log_tail)

    print_choice_block(
        snapshot,
        choice_number=engine.total_choices_made() + 1,
        game_dir_name=engine.game_dir.name,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
