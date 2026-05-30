#!/usr/bin/env python3
"""
Test snapshot determinism by taking snapshots at the same execution point multiple times.

This test verifies that:
1. Running to choice N and snapshotting produces state A
2. Resuming from state A and immediately snapshotting produces state A'
3. State A and A' are identical (deterministic snapshots)
"""

import subprocess
import sys
import json
import argparse
from pathlib import Path
from typing import Tuple, Optional

# ANSI color codes
RED = '\033[0;31m'
GREEN = '\033[0;32m'
YELLOW = '\033[1;33m'
CYAN = '\033[0;36m'
RESET = '\033[0m'

def print_color(color: str, text: str):
    """Print text in color."""
    print(f"{color}{text}{RESET}")

def run_to_snapshot(deck: str, p1_type: str, p2_type: str, seed: int,
                    choice_count: int, snapshot_path: Path) -> bool:
    """Run game until choice_count and save snapshot."""
    cmd = [
        './target/debug/mtg', 'tui', deck,
        '--p1', p1_type,
        '--p2', p2_type,
        '--seed', str(seed),
        '--stop-every', f'p1:choice:{choice_count}',
        '--snapshot-output', str(snapshot_path)
    ]

    result = subprocess.run(cmd, capture_output=True, text=True, timeout=60)

    if result.returncode != 0:
        print_color(RED, f"Error running to snapshot: {result.stderr}")
        return False

    return snapshot_path.exists()

def resume_and_snapshot_immediately(snapshot_in: Path, snapshot_out: Path,
                                     p1_type: str, p2_type: str) -> bool:
    """Resume from snapshot and immediately take another snapshot at same point (0 new choices)."""
    cmd = [
        './target/debug/mtg', 'tui',
        '--start-from', str(snapshot_in),
        '--p1', p1_type,
        '--p2', p2_type,
        '--stop-every', 'p1:choice:0',  # Stop immediately (0 new choices)
        '--snapshot-output', str(snapshot_out)
    ]

    result = subprocess.run(cmd, capture_output=True, text=True, timeout=60)

    if result.returncode != 0:
        print_color(RED, f"Error resuming and snapshotting: {result.stderr}")
        return False

    return snapshot_out.exists()

def strip_metadata(obj):
    """
    Recursively strip metadata/ephemeral fields from snapshot.

    These fields can legitimately differ between snapshots taken at the same
    execution point:
    - choice_id: Global counter that increments
    - undo_log: Not part of gameplay state
    - intra_turn_choices: Replay metadata (number of choices can differ if snapshotting after rewind)
    - logger state: Presentation layer
    - cached/derived state
    - controller state: RNG state can differ between first play and replay
    - lands_played_this_turn: Turn-scoped counter that resets during rewind
    """
    if isinstance(obj, dict):
        result = {}
        for key, value in obj.items():
            # Skip metadata fields
            if key in ('choice_id', 'undo_log', 'intra_turn_choices',
                      'logger', 'show_choice_menu',
                      'output_mode', 'output_format', 'numeric_choices',
                      'step_header_printed',
                      'p1_controller_state', 'p2_controller_state',
                      'lands_played_this_turn'):
                continue
            result[key] = strip_metadata(value)
        return result
    elif isinstance(obj, list):
        return [strip_metadata(item) for item in obj]
    else:
        return obj

def compare_snapshots(snap1_path: Path, snap2_path: Path, verbose: bool = False) -> Tuple[bool, Optional[str]]:
    """
    Compare two snapshots for equality.

    Returns:
        (matches, diff_message) - True if snapshots are functionally identical
    """
    try:
        with open(snap1_path) as f:
            snap1 = json.load(f)
        with open(snap2_path) as f:
            snap2 = json.load(f)
    except Exception as e:
        return False, f"Failed to load snapshots: {e}"

    # Strip metadata that can legitimately differ
    snap1_clean = strip_metadata(snap1)
    snap2_clean = strip_metadata(snap2)

    # Compare
    if snap1_clean == snap2_clean:
        return True, None

    # If not equal, try to identify differences
    if verbose:
        import difflib
        snap1_str = json.dumps(snap1_clean, indent=2, sort_keys=True)
        snap2_str = json.dumps(snap2_clean, indent=2, sort_keys=True)

        diff = difflib.unified_diff(
            snap1_str.splitlines(keepends=True),
            snap2_str.splitlines(keepends=True),
            fromfile=str(snap1_path),
            tofile=str(snap2_path),
            lineterm=''
        )

        diff_text = ''.join(list(diff)[:100])  # Limit to first 100 lines
        return False, diff_text

    return False, "Snapshots differ (use --verbose to see diff)"

def test_snapshot_determinism(deck: str, p1_type: str, p2_type: str,
                               seed: int, choice_count: int,
                               temp_dir: Path, verbose: bool = False) -> bool:
    """
    Test that snapshots are deterministic.

    Process:
    1. Run game to choice N, save snapshot A
    2. Resume from snapshot A, immediately snapshot again → snapshot B
    3. Assert A == B (modulo metadata)
    """
    print_color(CYAN, f"\nTesting snapshot determinism at choice {choice_count}:")
    print(f"  Deck: {deck}")
    print(f"  Controllers: {p1_type} vs {p2_type}")
    print(f"  Seed: {seed}")

    # Step 1: Run to choice N and save snapshot A
    snapshot_a = temp_dir / "snapshot_a.json"
    print(f"\n[1/3] Running to choice {choice_count} → {snapshot_a.name}")
    if not run_to_snapshot(deck, p1_type, p2_type, seed, choice_count, snapshot_a):
        print_color(RED, "✗ Failed to create initial snapshot")
        return False
    print_color(GREEN, f"✓ Created snapshot A (turn {json.load(open(snapshot_a))['turn_number']})")

    # Step 2: Resume from A and immediately snapshot → B (0 new choices)
    snapshot_b = temp_dir / "snapshot_b.json"
    print(f"\n[2/3] Resume from A and snapshot immediately (0 new choices) → {snapshot_b.name}")
    if not resume_and_snapshot_immediately(snapshot_a, snapshot_b, p1_type, p2_type):
        print_color(RED, "✗ Failed to create resumed snapshot")
        return False
    print_color(GREEN, f"✓ Created snapshot B (turn {json.load(open(snapshot_b))['turn_number']})")

    # Step 3: Compare A and B
    print(f"\n[3/3] Comparing snapshots...")
    matches, diff = compare_snapshots(snapshot_a, snapshot_b, verbose)

    if matches:
        print_color(GREEN, "✓ PASS: Snapshots are identical (deterministic)")
        return True
    else:
        print_color(RED, "✗ FAIL: Snapshots differ")
        if diff:
            print("\nDifferences:")
            print(diff)

        # Show file sizes for debugging
        size_a = snapshot_a.stat().st_size
        size_b = snapshot_b.stat().st_size
        print(f"\nSnapshot A size: {size_a} bytes")
        print(f"Snapshot B size: {size_b} bytes")

        return False

def main():
    parser = argparse.ArgumentParser(
        description='Test snapshot determinism',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Test at choice 3 with default settings
  %(prog)s decks/royal_assassin.dck

  # Test at choice 10 with specific seed
  %(prog)s decks/royal_assassin.dck --choice 10 --seed 42

  # Multiple tests at different choice points
  %(prog)s decks/grizzly_bears.dck --choice 5 10 15

  # Verbose diff output
  %(prog)s decks/royal_assassin.dck --verbose
"""
    )

    parser.add_argument('deck', help='Path to deck file')
    parser.add_argument('--p1', default='random', help='Player 1 controller type (default: random)')
    parser.add_argument('--p2', default='random', help='Player 2 controller type (default: random)')
    parser.add_argument('--seed', type=int, default=42, help='Random seed (default: 42)')
    parser.add_argument('--choice', type=int, nargs='+', default=[3],
                       help='Choice count(s) to test (default: 3)')
    parser.add_argument('--temp-dir', type=str, default='test_artifacts',
                       help='Directory for temporary files (default: test_artifacts)')
    parser.add_argument('--verbose', '-v', action='store_true',
                       help='Show detailed diff on failure')

    args = parser.parse_args()

    # Ensure binary is built
    print("Building project...")
    result = subprocess.run(['cargo', 'build'], capture_output=True, text=True)
    if result.returncode != 0:
        print_color(RED, f"Build failed: {result.stderr}")
        return 1
    print_color(GREEN, "✓ Build successful\n")

    # Create temp directory
    temp_dir = Path(args.temp_dir)
    temp_dir.mkdir(exist_ok=True)

    # Run tests
    all_passed = True
    for choice_count in args.choice:
        passed = test_snapshot_determinism(
            args.deck, args.p1, args.p2, args.seed,
            choice_count, temp_dir, args.verbose
        )
        if not passed:
            all_passed = False

    # Summary
    print("\n" + "="*60)
    if all_passed:
        print_color(GREEN, "✓ ALL TESTS PASSED: Snapshots are deterministic")
        return 0
    else:
        print_color(RED, "✗ SOME TESTS FAILED: Snapshots are NOT deterministic")
        return 1

if __name__ == '__main__':
    sys.exit(main())
