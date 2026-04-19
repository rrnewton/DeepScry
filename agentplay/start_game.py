#!/usr/bin/env python3
"""Start a new agentplay game session.

Thin CLI wrapper around GameEngine.start_game().
Replaces the former start_game.sh shell script.

Usage:
    ./agentplay/start_game.py decks/simple_bolt.dck decks/simple_bolt.dck
    ./agentplay/start_game.py --game-dir=my_test.game decks/a.dck decks/b.dck
    ./agentplay/start_game.py --seed=7 decks/a.dck decks/b.dck
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agentplay.lib.engine import GameEngine


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

    print(f"Starting new game in {engine.game_dir.name}", file=sys.stderr)
    try:
        snapshot = engine.start_game()
    except (RuntimeError, FileExistsError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    choices = snapshot.get("choices", [])
    active = snapshot.get("active_player")
    player = "p1" if str(active) == "0" else "p2"
    print(f"Game started. {player}'s turn. {len(choices)} choice(s) available.", file=sys.stderr)
    if choices:
        print("[0] pass", file=sys.stderr)
        for i, c in enumerate(choices, 1):
            print(f"[{i}] {c}", file=sys.stderr)
    print(f"\nContinue with: ./agentplay/continue_game.py {player} <choice>", file=sys.stderr)

    json.dump(snapshot, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
