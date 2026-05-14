#!/usr/bin/env python3
"""
Compare two GameState JSON files.

Usage:
    diff_gamestate.py <normal_state> <stopgo_state> [--verbose]

The script compares game states after stripping metadata fields.
"""

import argparse
import sys
import json
from pathlib import Path

# ANSI color codes
RED = '\033[0;31m'
GREEN = '\033[0;32m'
YELLOW = '\033[1;33m'
CYAN = '\033[0;36m'
NC = '\033[0m'  # No Color

def print_color(color: str, message: str):
    """Print colored message"""
    print(f"{color}{message}{NC}")

def strip_metadata_fields(obj):
    """Recursively strip metadata fields from gamestate."""
    if isinstance(obj, dict):
        result = {}
        for key, value in obj.items():
            # Skip metadata/presentation fields that differ between normal and stop/go modes:
            # - choice_id: unique ID for each choice (increments differently based on stop points)
            # - undo_log: snapshot/replay metadata, not actual game state
            # - show_choice_menu: presentation flag (set true in stop/go mode)
            # - output_mode: presentation setting (Stdout vs Both)
            # - output_format: presentation setting (Text vs JSON)
            # - numeric_choices: presentation setting
            # - step_header_printed: transient UI state
            # - mana_state_version: bookkeeping counter for the ManaEngine
            #   incremental cache; gets one or two extra bumps after restoring
            #   from a snapshot because the cache is rebuilt from scratch.
            #   Not actual game state.
            if key in ("choice_id", "undo_log", "show_choice_menu", "output_mode",
                      "output_format", "numeric_choices", "step_header_printed",
                      "mana_state_version"):
                continue
            result[key] = strip_metadata_fields(value)
        return result
    elif isinstance(obj, list):
        return [strip_metadata_fields(item) for item in obj]
    else:
        return obj

def compare_gamestates(normal_state_file: Path, stopgo_state_file: Path, verbose: bool = False) -> bool:
    """Compare final GameState snapshots for differences."""
    try:
        with open(normal_state_file, 'r') as f:
            normal_state = json.load(f)
        with open(stopgo_state_file, 'r') as f:
            stopgo_state = json.load(f)
    except Exception as e:
        print_color(RED, f"Failed to load gamestate files: {e}")
        return False

    # Extract just the game_state portion
    normal_gs = normal_state.get("game_state", {})
    stopgo_gs = stopgo_state.get("game_state", {})

    # Strip metadata fields
    normal_gs = strip_metadata_fields(normal_gs)
    stopgo_gs = strip_metadata_fields(stopgo_gs)

    # Serialize both to canonical JSON for comparison
    normal_json = json.dumps(normal_gs, sort_keys=True, indent=2)
    stopgo_json = json.dumps(stopgo_gs, sort_keys=True, indent=2)

    if normal_json != stopgo_json:
        if verbose:
            # Show differences using unified diff
            import difflib
            diff = difflib.unified_diff(
                normal_json.splitlines(keepends=True),
                stopgo_json.splitlines(keepends=True),
                fromfile='normal_gamestate',
                tofile='stopgo_gamestate',
                lineterm=''
            )
            print_color(YELLOW, "GameState differences:")
            for i, line in enumerate(diff):
                if i < 100:  # Show first 100 lines of diff
                    print(f"  {line.rstrip()}")
                elif i == 100:
                    print("  ... (diff truncated)")
                    break
        return False

    return True

def main():
    parser = argparse.ArgumentParser(
        description='Compare two GameState JSON files',
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )

    parser.add_argument('normal_state', type=str, help='Path to normal game state JSON')
    parser.add_argument('stopgo_state', type=str, help='Path to stop-go game state JSON')
    parser.add_argument('--verbose', '-v', action='store_true', help='Show detailed differences')

    args = parser.parse_args()

    normal_path = Path(args.normal_state)
    stopgo_path = Path(args.stopgo_state)

    if not normal_path.exists():
        print_color(RED, f"Error: Normal state not found: {normal_path}")
        sys.exit(1)

    if not stopgo_path.exists():
        print_color(RED, f"Error: Stop-go state not found: {stopgo_path}")
        sys.exit(1)

    # Compare
    match = compare_gamestates(normal_path, stopgo_path, verbose=args.verbose)

    if match:
        print_color(GREEN, "✓ GameStates match")
        sys.exit(0)
    else:
        print_color(RED, "✗ GameStates differ")
        sys.exit(1)

if __name__ == "__main__":
    main()
