#!/usr/bin/env python3
"""
Network Fuzz Test - Find bugs by testing various configurations

Tests the network implementation with different:
- Controller types (heuristic, random, zero)
- Seeds
- Deck combinations
- Player orderings

Reports:
- Pass/fail rates per configuration
- Error categorization by last ERROR lines in logs
- Determinism testing (re-running failures)
"""

import subprocess
import os
import sys
import tempfile
import shutil
import re
import json
import time
from dataclasses import dataclass, field
from typing import Optional, List, Dict, Tuple
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
import random

# Configuration
WORKSPACE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MTG_BIN = os.path.join(WORKSPACE_ROOT, "target/release/mtg")

# Decks to test
DECKS = [
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/ryan_avatar_draft.dck"),
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/gabriel_avatar_draft.dck"),
]

# Controller types to test
CONTROLLERS = ["heuristic", "random", "zero"]

# Seeds to test
SEEDS = [1, 2, 3, 5, 7, 11, 13, 17, 42, 100]

@dataclass
class TestConfig:
    """Test configuration"""
    seed: int
    controller_p1: str
    controller_p2: str
    deck1: str
    deck2: str
    seed_p1: int = 3
    seed_p2: int = 3

    def __str__(self):
        return f"seed={self.seed} p1={self.controller_p1} p2={self.controller_p2}"

@dataclass
class TestResult:
    """Result of a single test run"""
    config: TestConfig
    passed: bool
    duration: float
    error_signature: Optional[str] = None
    server_errors: List[str] = field(default_factory=list)
    client1_errors: List[str] = field(default_factory=list)
    client2_errors: List[str] = field(default_factory=list)
    output_dir: Optional[str] = None

def extract_error_signature(log_path: str) -> List[str]:
    """Extract last few ERROR lines from a log file"""
    errors = []
    if os.path.exists(log_path):
        with open(log_path, 'r') as f:
            for line in f:
                if 'ERROR' in line.upper() or 'PANIC' in line.upper():
                    # Clean ANSI codes and timestamps
                    clean = re.sub(r'\x1b\[[0-9;]*m', '', line)
                    clean = re.sub(r'^\[.*?\] ', '', clean)
                    errors.append(clean.strip())
    return errors[-3:] if errors else []

def make_error_signature(server_errors: List[str], client1_errors: List[str], client2_errors: List[str]) -> str:
    """Create a signature from errors for bucketing"""
    all_errors = server_errors + client1_errors + client2_errors
    if not all_errors:
        return "unknown"

    # Take the most specific error (usually the first one that caused the cascade)
    for error in all_errors:
        if 'unexpected OpponentChoice' in error:
            return "unexpected_opponent_choice"
        if 'action_count mismatch' in error:
            return "action_count_mismatch"
        if 'Connection reset' in error:
            return "connection_reset"
        if 'REVEAL VALIDATION FAILED' in error:
            return "reveal_validation"
        if 'panic' in error.lower():
            return "panic"

    # Fallback: use first error line
    return all_errors[0][:50] if all_errors else "unknown"

def run_test(config: TestConfig, timeout: int = 120) -> TestResult:
    """Run a single network test"""
    start_time = time.time()

    # Create temp directory
    output_dir = tempfile.mkdtemp(prefix="network_fuzz_")

    # Random port
    port = random.randint(17800, 27800)

    # Paths
    server_log = os.path.join(output_dir, "server.log")
    client1_log = os.path.join(output_dir, "client1.log")
    client2_log = os.path.join(output_dir, "client2.log")

    try:
        # Start server
        server_proc = subprocess.Popen(
            [MTG_BIN, "server",
             "--port", str(port),
             "--seed", str(config.seed),
             "--network-debug",
             "--verbosity", "minimal",
             "--no-color-logs"],
            stdout=open(server_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        # Wait for server to start
        time.sleep(1.5)

        if server_proc.poll() is not None:
            return TestResult(
                config=config,
                passed=False,
                duration=time.time() - start_time,
                error_signature="server_startup_failed",
                output_dir=output_dir
            )

        # Start client 1
        client1_proc = subprocess.Popen(
            [MTG_BIN, "connect",
             config.deck1,
             "--server", f"localhost:{port}",
             "--controller", config.controller_p1,
             "--seed-player", str(config.seed_p1),
             "--name", "Ryan"],
            stdout=open(client1_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        time.sleep(0.5)

        # Start client 2
        client2_proc = subprocess.Popen(
            [MTG_BIN, "connect",
             config.deck2,
             "--server", f"localhost:{port}",
             "--controller", config.controller_p2,
             "--seed-player", str(config.seed_p2),
             "--name", "Gabriel"],
            stdout=open(client2_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        # Wait for completion
        try:
            server_proc.wait(timeout=timeout)
            client1_proc.wait(timeout=5)
            client2_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server_proc.kill()
            client1_proc.kill()
            client2_proc.kill()
            return TestResult(
                config=config,
                passed=False,
                duration=time.time() - start_time,
                error_signature="timeout",
                output_dir=output_dir
            )

        duration = time.time() - start_time

        # Check results
        server_errors = extract_error_signature(server_log)
        client1_errors = extract_error_signature(client1_log)
        client2_errors = extract_error_signature(client2_log)

        # Determine if passed (no errors and server exited cleanly)
        passed = (server_proc.returncode == 0 and
                  not server_errors and
                  not client1_errors and
                  not client2_errors)

        error_sig = None if passed else make_error_signature(server_errors, client1_errors, client2_errors)

        return TestResult(
            config=config,
            passed=passed,
            duration=duration,
            error_signature=error_sig,
            server_errors=server_errors,
            client1_errors=client1_errors,
            client2_errors=client2_errors,
            output_dir=output_dir
        )

    except Exception as e:
        return TestResult(
            config=config,
            passed=False,
            duration=time.time() - start_time,
            error_signature=f"exception:{str(e)[:30]}",
            output_dir=output_dir
        )

def test_determinism(config: TestConfig, num_runs: int = 3) -> Tuple[int, int]:
    """Test if a configuration fails deterministically"""
    passes = 0
    fails = 0
    for _ in range(num_runs):
        result = run_test(config)
        if result.passed:
            passes += 1
        else:
            fails += 1
        # Clean up
        if result.output_dir and os.path.exists(result.output_dir):
            shutil.rmtree(result.output_dir, ignore_errors=True)
    return passes, fails

def generate_configs(num_configs: int = 50) -> List[TestConfig]:
    """Generate diverse test configurations"""
    configs = []

    # Test all controller combinations
    for c1 in CONTROLLERS:
        for c2 in CONTROLLERS:
            for seed in SEEDS[:5]:  # First 5 seeds for each combo
                configs.append(TestConfig(
                    seed=seed,
                    controller_p1=c1,
                    controller_p2=c2,
                    deck1=DECKS[0],
                    deck2=DECKS[1]
                ))

    # Add some random configs
    while len(configs) < num_configs:
        configs.append(TestConfig(
            seed=random.randint(1, 1000),
            controller_p1=random.choice(CONTROLLERS),
            controller_p2=random.choice(CONTROLLERS),
            deck1=random.choice(DECKS),
            deck2=random.choice(DECKS)
        ))

    return configs[:num_configs]

def main():
    import argparse
    parser = argparse.ArgumentParser(description='Network fuzz tester')
    parser.add_argument('--configs', type=int, default=30, help='Number of configs to test')
    parser.add_argument('--parallel', type=int, default=3, help='Parallel test count')
    parser.add_argument('--determinism-runs', type=int, default=3, help='Runs for determinism test')
    parser.add_argument('--quick', action='store_true', help='Quick mode: fewer configs')
    args = parser.parse_args()

    if args.quick:
        args.configs = 10

    print(f"=== Network Fuzz Test ===")
    print(f"Binary: {MTG_BIN}")
    print(f"Configs: {args.configs}")
    print(f"Parallel: {args.parallel}")
    print()

    # Check binary exists
    if not os.path.exists(MTG_BIN):
        print(f"ERROR: Binary not found: {MTG_BIN}")
        print("Run: cargo build --release --features network")
        sys.exit(1)

    # Generate configs
    configs = generate_configs(args.configs)
    print(f"Generated {len(configs)} test configurations")
    print()

    # Run tests
    results: List[TestResult] = []
    error_buckets: Dict[str, List[TestResult]] = defaultdict(list)

    print("Running tests...")
    with ThreadPoolExecutor(max_workers=args.parallel) as executor:
        futures = {executor.submit(run_test, config): config for config in configs}

        for i, future in enumerate(as_completed(futures)):
            result = future.result()
            results.append(result)

            status = "PASS" if result.passed else f"FAIL ({result.error_signature})"
            print(f"  [{i+1}/{len(configs)}] {result.config}: {status} ({result.duration:.1f}s)")

            if not result.passed:
                error_buckets[result.error_signature].append(result)

            # Clean up passing tests
            if result.passed and result.output_dir and os.path.exists(result.output_dir):
                shutil.rmtree(result.output_dir, ignore_errors=True)

    print()

    # Summary
    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed

    print("=== Summary ===")
    print(f"Total:  {len(results)}")
    print(f"Passed: {passed} ({100*passed/len(results):.1f}%)")
    print(f"Failed: {failed} ({100*failed/len(results):.1f}%)")
    print()

    # Error breakdown
    if error_buckets:
        print("=== Error Categories ===")
        for error_sig, error_results in sorted(error_buckets.items(), key=lambda x: -len(x[1])):
            print(f"\n{error_sig}: {len(error_results)} occurrences")
            # Show example config
            ex = error_results[0]
            print(f"  Example: {ex.config}")
            if ex.server_errors:
                print(f"  Server: {ex.server_errors[0][:80]}...")
            if ex.client1_errors:
                print(f"  Client1: {ex.client1_errors[0][:80]}...")
            if ex.client2_errors:
                print(f"  Client2: {ex.client2_errors[0][:80]}...")

        # Test determinism of failures
        print()
        print("=== Determinism Test ===")
        for error_sig, error_results in error_buckets.items():
            config = error_results[0].config
            passes, fails = test_determinism(config, args.determinism_runs)
            det = "DETERMINISTIC" if fails == args.determinism_runs else f"FLAKY ({passes}/{args.determinism_runs} pass)"
            print(f"{error_sig}: {det}")

    print()
    print("=== Controller Matrix ===")
    # Show pass rate by controller combination
    matrix = defaultdict(lambda: {"passed": 0, "total": 0})
    for r in results:
        key = f"{r.config.controller_p1} vs {r.config.controller_p2}"
        matrix[key]["total"] += 1
        if r.passed:
            matrix[key]["passed"] += 1

    for combo, stats in sorted(matrix.items()):
        pct = 100 * stats["passed"] / stats["total"] if stats["total"] > 0 else 0
        print(f"  {combo}: {stats['passed']}/{stats['total']} ({pct:.0f}%)")

    # Keep failure logs
    if error_buckets:
        print()
        print("=== Failure Logs ===")
        for error_sig, error_results in error_buckets.items():
            if error_results[0].output_dir and os.path.exists(error_results[0].output_dir):
                print(f"{error_sig}: {error_results[0].output_dir}")

    sys.exit(0 if failed == 0 else 1)

if __name__ == "__main__":
    main()
