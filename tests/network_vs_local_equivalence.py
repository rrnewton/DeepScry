#!/usr/bin/env python3
"""
Network vs Local Game Equivalence Test

Thin CLI wrapper around the shared network_test_lib infrastructure.
Replaces tests/network_vs_local_equivalence_e2e.sh with Python.

Usage:
    python3 tests/network_vs_local_equivalence.py [SEED] [CONTROLLER_P1] [CONTROLLER_P2]

Examples:
    python3 tests/network_vs_local_equivalence.py              # seed=3, both zero
    python3 tests/network_vs_local_equivalence.py 5            # seed=5, both zero
    python3 tests/network_vs_local_equivalence.py 5 random     # seed=5, both random
    python3 tests/network_vs_local_equivalence.py 5 random heuristic  # seed=5, p1=random, p2=heuristic

Exit codes:
    0 - PASS: gamelogs are identical
    1 - FAIL: gamelogs differ or error occurred
"""

import os
import sys
import shutil

# Import shared library
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'bug_finding'))
from network_test_lib import (
    MTG_BIN, DECKS, CONTROLLERS,
    TestConfig, run_equivalence_test,
)


def main():
    # Parse arguments with defaults
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    controller_p1 = sys.argv[2] if len(sys.argv) > 2 else "zero"
    controller_p2 = sys.argv[3] if len(sys.argv) > 3 else controller_p1

    # Validate controllers
    for ctrl in [controller_p1, controller_p2]:
        if ctrl not in CONTROLLERS:
            print(f"Error: Invalid controller '{ctrl}'. Must be: {', '.join(CONTROLLERS)}")
            sys.exit(1)

    # Check binary exists
    if not os.path.exists(MTG_BIN):
        print(f"Error: Binary not found: {MTG_BIN}")
        print("Run: cargo build --release --features network")
        sys.exit(1)

    print("=== Network vs Local Game Equivalence Test ===")
    print()
    print("Configuration:")
    print(f"  Seed: {seed}")
    print(f"  Controller P1: {controller_p1}")
    print(f"  Controller P2: {controller_p2}")
    print(f"  Deck 1: {os.path.basename(DECKS[0])}")
    print(f"  Deck 2: {os.path.basename(DECKS[1])}")
    print()

    config = TestConfig(
        seed=seed,
        controller_p1=controller_p1,
        controller_p2=controller_p2,
        deck1=DECKS[0],
        deck2=DECKS[1],
    )

    print("Running LOCAL and NETWORK games...")
    result = run_equivalence_test(config, timeout=180)

    print()
    print("=== Results ===")
    print(f"Duration: {result.duration:.1f}s")

    if result.passed:
        print("\033[32m✓ PASS: LOCAL and NETWORK gamelogs are IDENTICAL\033[0m")
        # Clean up on pass
        if result.output_dir and os.path.exists(result.output_dir):
            shutil.rmtree(result.output_dir, ignore_errors=True)
        sys.exit(0)
    else:
        print(f"\033[31m✗ FAIL: {result.error_signature}\033[0m")

        if result.gamelog_diff_lines > 0:
            print(f"\nGamelog differences: {result.gamelog_diff_lines} lines")
            if result.gamelog_diff_sample:
                print(result.gamelog_diff_sample)

        if result.local_errors:
            print(f"\nLocal errors: {result.local_errors}")
        if result.server_errors:
            print(f"\nServer errors: {result.server_errors}")
        if result.client1_errors:
            print(f"\nClient1 errors: {result.client1_errors}")
        if result.client2_errors:
            print(f"\nClient2 errors: {result.client2_errors}")

        if result.output_dir:
            print(f"\nLogs preserved at: {result.output_dir}")

        # Print reproducer command
        print(f"\nReproducer: {config.reproducer_command()}")

        sys.exit(1)


if __name__ == "__main__":
    main()
