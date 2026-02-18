#!/usr/bin/env python3
"""
Network Fuzz Test - Find bugs by testing various configurations

This is a BUG FINDING script, NOT a regression test.
It runs for extended periods to discover new bugs through randomized testing.

Tests the network implementation with different:
- Controller types (heuristic, random, zero)
- Seeds
- Deck combinations
- Player orderings

Modes:
- Default: Network-only testing (check for crashes/errors)
- --local-equivalence: Also run a local game and compare gamelogs

Reports on exit (or Ctrl-C):
- Pass/fail rates per configuration
- Error categorization by last ERROR lines in logs
- Determinism testing (re-running failures)
- Reproducer commands for debugging
"""

import os
import sys
import shutil
import signal
import time
import random
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import List, Dict, Tuple

# Import shared library
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from network_test_lib import (
    MTG_BIN, DECKS, CONTROLLERS, CLIENT_MODES,
    TestConfig, TestResult,
    run_network_test, run_equivalence_test,
)

# Seeds to test
SEEDS = [1, 2, 3, 5, 7, 11, 13, 17, 42, 100]

# Global for graceful shutdown
shutdown_requested = False
results_collected: List[TestResult] = []
error_buckets_collected: Dict[str, List[TestResult]] = defaultdict(list)


def generate_configs(num_configs: int = 50,
                     controller_filter: str = None,
                     client_filter: str = None) -> List[TestConfig]:
    """Generate diverse test configurations.

    Args:
        num_configs: Maximum number of configs to generate
        controller_filter: If set, only generate configs where BOTH players
                          use this controller type
        client_filter: If set, use this client mode for all players
                       ("native", "wasm", or "mixed" for p1=native/p2=wasm)
    """
    configs = []

    # Determine which controllers to use
    if controller_filter:
        controllers_to_test = [controller_filter]
    else:
        controllers_to_test = CONTROLLERS

    # Determine client modes
    def get_client_modes():
        if client_filter == "wasm":
            return [("wasm", "wasm")]
        elif client_filter == "mixed":
            return [("native", "wasm"), ("wasm", "native")]
        else:  # native (default)
            return [("native", "native")]

    client_mode_pairs = get_client_modes()

    # Test all controller combinations with first 5 seeds
    for c1 in controllers_to_test:
        for c2 in controllers_to_test:
            for seed in SEEDS[:5]:
                for cl1, cl2 in client_mode_pairs:
                    configs.append(TestConfig(
                        seed=seed,
                        controller_p1=c1,
                        controller_p2=c2,
                        deck1=DECKS[0],
                        deck2=DECKS[1],
                        client_p1=cl1,
                        client_p2=cl2,
                    ))

    # Add random configs to fill up to num_configs
    while len(configs) < num_configs:
        cl1, cl2 = random.choice(client_mode_pairs)
        configs.append(TestConfig(
            seed=random.randint(1, 1000),
            controller_p1=random.choice(controllers_to_test),
            controller_p2=random.choice(controllers_to_test),
            deck1=random.choice(DECKS),
            deck2=random.choice(DECKS),
            client_p1=cl1,
            client_p2=cl2,
        ))

    return configs[:num_configs]


def test_determinism(config: TestConfig, num_runs: int = 3,
                     use_equivalence: bool = False) -> Tuple[int, int]:
    """Test if a configuration fails deterministically."""
    passes = 0
    fails = 0
    runner = run_equivalence_test if use_equivalence else run_network_test
    for _ in range(num_runs):
        result = runner(config)
        if result.passed:
            passes += 1
        else:
            fails += 1
        if result.output_dir and os.path.exists(result.output_dir):
            shutil.rmtree(result.output_dir, ignore_errors=True)
    return passes, fails


def format_result_status(result: TestResult, use_equivalence: bool) -> str:
    """Format a result line for display."""
    if result.passed:
        return "PASS"
    parts = [f"FAIL ({result.error_signature})"]
    if use_equivalence and result.gamelog_diff_lines > 0:
        parts.append(f"  gamelog: {result.gamelog_diff_lines} lines differ")
    return "\n    ".join(parts)


def print_summary(results: List[TestResult],
                  error_buckets: Dict[str, List[TestResult]],
                  determinism_runs: int = 3,
                  interrupted: bool = False,
                  use_equivalence: bool = False):
    """Print the test summary."""
    print()
    if interrupted:
        print("=" * 50)
        print("INTERRUPTED - Printing summary of completed tests")
        print("=" * 50)
    print()

    if not results:
        print("No tests completed.")
        return

    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed
    mode = "EQUIVALENCE (local+network)" if use_equivalence else "NETWORK-ONLY"

    print(f"=== Summary ({mode}) ===")
    print(f"Total:  {len(results)}")
    print(f"Passed: {passed} ({100*passed/len(results):.1f}%)")
    print(f"Failed: {failed} ({100*failed/len(results):.1f}%)")
    print()

    # Error breakdown
    if error_buckets:
        print("=== Error Categories ===")
        for error_sig, error_results in sorted(
                error_buckets.items(), key=lambda x: -len(x[1])):
            print(f"\n{error_sig}: {len(error_results)} occurrences")
            ex = error_results[0]
            print(f"  Example: {ex.config}")
            if ex.server_errors:
                print(f"  Server: {ex.server_errors[0][:80]}")
            if ex.client1_errors:
                print(f"  Client1: {ex.client1_errors[0][:80]}")
            if ex.client2_errors:
                print(f"  Client2: {ex.client2_errors[0][:80]}")
            if use_equivalence:
                if ex.local_errors:
                    print(f"  Local: {ex.local_errors[0][:80]}")
                if ex.gamelog_diff_lines > 0:
                    print(f"  Gamelog diff: {ex.gamelog_diff_lines} lines")
                    if ex.gamelog_diff_sample:
                        for line in ex.gamelog_diff_sample.split('\n')[:6]:
                            print(f"    {line}")

        # Determinism testing (skip if interrupted)
        if not interrupted:
            print()
            print("=== Determinism Test ===")
            for error_sig, error_results in error_buckets.items():
                config = error_results[0].config
                passes, fails = test_determinism(
                    config, determinism_runs, use_equivalence)
                det = ("DETERMINISTIC" if fails == determinism_runs
                       else f"FLAKY ({passes}/{determinism_runs} pass)")
                print(f"{error_sig}: {det}")

    print()
    print("=== Controller Matrix ===")
    matrix = defaultdict(lambda: {"passed": 0, "total": 0})
    for r in results:
        key = f"{r.config.controller_p1} vs {r.config.controller_p2}"
        matrix[key]["total"] += 1
        if r.passed:
            matrix[key]["passed"] += 1

    for combo, stats in sorted(matrix.items()):
        pct = 100 * stats["passed"] / stats["total"] if stats["total"] > 0 else 0
        print(f"  {combo}: {stats['passed']}/{stats['total']} ({pct:.0f}%)")

    # Reproducer commands
    if error_buckets:
        print()
        print("=== Reproducer Commands ===")
        for error_sig, error_results in error_buckets.items():
            ex = error_results[0]
            print(f"\n--- {error_sig} ---")
            print(ex.config.reproducer_command())

    # Failure logs
    if error_buckets:
        print()
        print("=== Failure Logs ===")
        for error_sig, error_results in error_buckets.items():
            if (error_results[0].output_dir
                    and os.path.exists(error_results[0].output_dir)):
                print(f"{error_sig}: {error_results[0].output_dir}")


def signal_handler(signum, frame):
    """Handle Ctrl-C gracefully."""
    global shutdown_requested
    if shutdown_requested:
        print("\nForce quit...")
        sys.exit(1)
    print("\n\nShutdown requested... waiting for current tests to complete...")
    shutdown_requested = True


def main():
    global shutdown_requested, results_collected, error_buckets_collected

    import argparse
    parser = argparse.ArgumentParser(
        description='Network fuzz tester - Bug finding through randomized testing',
        epilog='This is a bug finding tool, not a regression test.'
    )
    parser.add_argument('--configs', type=int, default=30,
                        help='Number of configs to test per batch')
    parser.add_argument('--parallel', type=int, default=3,
                        help='Parallel test count')
    parser.add_argument('--determinism-runs', type=int, default=3,
                        help='Runs for determinism test')
    parser.add_argument('--quick', action='store_true',
                        help='Quick mode: fewer configs (10)')
    parser.add_argument('--infinite', action='store_true',
                        help='Run forever until Ctrl-C')
    parser.add_argument('--timeout', type=int, default=120,
                        help='Timeout per test in seconds')
    parser.add_argument('--local-equivalence', action='store_true',
                        help='Also run local game and compare gamelogs '
                             '(slower but catches divergence bugs)')
    parser.add_argument('--duration', type=int, default=0,
                        help='Run for this many seconds then stop (0=unlimited)')
    parser.add_argument('--controller', type=str, default=None,
                        help='Filter to only test specific controller type '
                             '(heuristic, random, zero)')
    parser.add_argument('--client', type=str, default='native',
                        help='Client mode for network players: '
                             '"native" (default), "wasm" (both players use WASM), '
                             'or "mixed" (p1=native, p2=wasm and vice versa)')
    args = parser.parse_args()

    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    if args.quick:
        args.configs = 10

    mode_str = ("EQUIVALENCE (local+network)"
                if args.local_equivalence else "NETWORK-ONLY")

    print(f"=== Network Fuzz Test ({mode_str}) ===")
    print(f"Binary: {MTG_BIN}")
    print(f"Configs per batch: {args.configs}")
    print(f"Parallel: {args.parallel}")
    print(f"Timeout per test: {args.timeout}s")
    if args.local_equivalence:
        print(f"Local equivalence: ON (--network-debug, gamelog comparison)")
    if args.duration > 0:
        print(f"Duration limit: {args.duration}s")
    print(f"Infinite mode: {args.infinite}")
    print(f"Press Ctrl-C to stop and see summary")
    print()

    if not os.path.exists(MTG_BIN):
        print(f"ERROR: Binary not found: {MTG_BIN}")
        print("Run: cargo build --release --features network")
        sys.exit(1)

    # Choose runner based on mode
    if args.local_equivalence:
        runner = lambda config: run_equivalence_test(config, timeout=args.timeout)
    else:
        runner = lambda config: run_network_test(config, timeout=args.timeout)

    start_wall_time = time.time()
    batch_num = 0

    while not shutdown_requested:
        # Check duration limit
        if args.duration > 0:
            elapsed = time.time() - start_wall_time
            if elapsed >= args.duration:
                print(f"\nDuration limit reached ({args.duration}s)")
                break

        batch_num += 1
        if args.infinite or args.duration > 0:
            elapsed = time.time() - start_wall_time
            print(f"\n=== Batch {batch_num} (elapsed: {elapsed:.0f}s) ===")

        configs = generate_configs(args.configs, args.controller, args.client)
        if not args.infinite and args.duration == 0:
            filter_msg = f" (controller={args.controller})" if args.controller else ""
            print(f"Generated {len(configs)} test configurations{filter_msg}")
        print()

        print("Running tests...")
        with ThreadPoolExecutor(max_workers=args.parallel) as executor:
            futures = {
                executor.submit(runner, config): config
                for config in configs
            }

            for i, future in enumerate(as_completed(futures)):
                if shutdown_requested:
                    break

                # Check duration limit mid-batch
                if args.duration > 0:
                    elapsed = time.time() - start_wall_time
                    if elapsed >= args.duration:
                        shutdown_requested = True
                        break

                result = future.result()
                results_collected.append(result)

                status = format_result_status(result, args.local_equivalence)
                print(f"  [{i+1}/{len(configs)}] {result.config}: "
                      f"{status} ({result.duration:.1f}s)")

                if not result.passed:
                    error_buckets_collected[result.error_signature].append(result)

                # Clean up passing tests
                if (result.passed and result.output_dir
                        and os.path.exists(result.output_dir)):
                    shutil.rmtree(result.output_dir, ignore_errors=True)

        # Exit after one batch unless infinite/duration mode
        if not args.infinite and args.duration == 0:
            break

    print_summary(
        results_collected, error_buckets_collected,
        args.determinism_runs,
        interrupted=shutdown_requested,
        use_equivalence=args.local_equivalence
    )

    failed = sum(1 for r in results_collected if not r.passed)
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
