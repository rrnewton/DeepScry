#!/usr/bin/env python3
"""Continue an agentplay game session with one choice.

Thin CLI wrapper around GameEngine.continue_game().
Replaces the former continue_game.sh shell script.

Usage:
    ./agentplay/continue_game.py p1 "play Mountain"
    ./agentplay/continue_game.py p2 "0"
    ./agentplay/continue_game.py --game-dir=042.game p1 "cast Lightning Bolt"
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

    print(f"Continuing game in {engine.game_dir.name}: {args.player} chose '{args.choice}'", file=sys.stderr)
    try:
        snapshot = engine.continue_game()
    except RuntimeError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    if engine.is_game_over(snapshot):
        print("Game over.", file=sys.stderr)
    else:
        choices = snapshot.get("choices", [])
        active = snapshot.get("active_player")
        player = "p1" if str(active) == "0" else "p2"
        print(f"{player}'s turn. {len(choices)} choice(s) available.", file=sys.stderr)
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
