#!/usr/bin/env python3
"""
Analyze benchmark variance from Criterion output

This script parses Criterion benchmark output and compares timing variance
between different implementations (e.g., Rayon vs pinned thread pool).

Usage:
    python3 scripts/analyze_benchmark_variance.py <benchmark_output.txt>

The script looks for lines like:
    time:   [1.2345 ms 1.2500 ms 1.2655 ms]

And extracts:
    - Lower bound (confidence interval)
    - Point estimate (mean)
    - Upper bound (confidence interval)
    - Variance = (upper - lower) / 2
    - Coefficient of variation = variance / mean
"""

import re
import sys
from pathlib import Path
from typing import Dict, List, Tuple


def parse_time_value(time_str: str) -> float:
    """Convert time string like '1.2345 ms' to nanoseconds"""
    parts = time_str.strip().split()
    if len(parts) != 2:
        return 0.0

    value = float(parts[0])
    unit = parts[1]

    # Convert to nanoseconds
    if unit == 'ns':
        return value
    elif unit == 'µs' or unit == 'us':
        return value * 1000
    elif unit == 'ms':
        return value * 1_000_000
    elif unit == 's':
        return value * 1_000_000_000
    else:
        return value


def parse_criterion_output(file_path: Path) -> Dict[str, List[Tuple[float, float, float]]]:
    """
    Parse Criterion output file and extract timing data

    Returns:
        Dict mapping benchmark name to list of (lower, mean, upper) tuples in nanoseconds
    """
    results = {}
    current_benchmark = None

    with open(file_path, 'r') as f:
        for line in f:
            # Look for benchmark name
            if line.startswith('Benchmarking'):
                # Extract benchmark name
                match = re.search(r'Benchmarking (.+?):', line)
                if match:
                    current_benchmark = match.group(1)
                    if current_benchmark not in results:
                        results[current_benchmark] = []

            # Look for timing line
            if 'time:' in line and '[' in line:
                # Extract times from: time:   [1.2345 ms 1.2500 ms 1.2655 ms]
                match = re.search(r'time:\s+\[(.+?)\s+(.+?)\s+(.+?)\]', line)
                if match and current_benchmark:
                    lower = parse_time_value(match.group(1))
                    mean = parse_time_value(match.group(2))
                    upper = parse_time_value(match.group(3))

                    if lower > 0 and mean > 0 and upper > 0:
                        results[current_benchmark].append((lower, mean, upper))

    return results


def analyze_variance(results: Dict[str, List[Tuple[float, float, float]]]):
    """Analyze and print variance statistics"""

    print("=" * 80)
    print("Benchmark Variance Analysis")
    print("=" * 80)
    print()

    for benchmark_name, timings in sorted(results.items()):
        if not timings:
            continue

        print(f"\n{benchmark_name}")
        print("-" * 80)

        # Calculate statistics across all samples
        means = [t[1] for t in timings]
        variances = [(t[2] - t[0]) / 2 for t in timings]

        avg_mean = sum(means) / len(means)
        avg_variance = sum(variances) / len(variances)

        # Coefficient of variation (std dev / mean, as percentage)
        cv = (avg_variance / avg_mean) * 100

        print(f"  Samples: {len(timings)}")
        print(f"  Mean time: {avg_mean / 1_000_000:.3f} ms")
        print(f"  Avg confidence interval width: ±{avg_variance / 1_000_000:.3f} ms")
        print(f"  Coefficient of variation: {cv:.2f}%")

        # Show individual samples
        print(f"\n  Individual samples:")
        for i, (lower, mean, upper) in enumerate(timings, 1):
            variance = (upper - lower) / 2
            sample_cv = (variance / mean) * 100
            print(f"    Sample {i}: {mean / 1_000_000:.3f} ms ± {variance / 1_000_000:.3f} ms (CV: {sample_cv:.2f}%)")

    print("\n" + "=" * 80)
    print("Comparison Summary")
    print("=" * 80)
    print()

    # Compare Rayon vs Pinned if both exist
    rayon_key = None
    pinned_key = None

    for key in results.keys():
        if 'par_rewind_play_again' in key and 'pinned' not in key:
            rayon_key = key
        elif 'pinned_par_rewind_play_again' in key:
            pinned_key = key

    if rayon_key and pinned_key:
        rayon_timings = results[rayon_key]
        pinned_timings = results[pinned_key]

        if rayon_timings and pinned_timings:
            rayon_means = [t[1] for t in rayon_timings]
            pinned_means = [t[1] for t in pinned_timings]

            rayon_variances = [(t[2] - t[0]) / 2 for t in rayon_timings]
            pinned_variances = [(t[2] - t[0]) / 2 for t in pinned_timings]

            rayon_avg_mean = sum(rayon_means) / len(rayon_means)
            pinned_avg_mean = sum(pinned_means) / len(pinned_means)

            rayon_avg_variance = sum(rayon_variances) / len(rayon_variances)
            pinned_avg_variance = sum(pinned_variances) / len(pinned_variances)

            rayon_cv = (rayon_avg_variance / rayon_avg_mean) * 100
            pinned_cv = (pinned_avg_variance / pinned_avg_mean) * 100

            print("Rayon (work-stealing thread pool):")
            print(f"  Mean: {rayon_avg_mean / 1_000_000:.3f} ms")
            print(f"  Variance: ±{rayon_avg_variance / 1_000_000:.3f} ms")
            print(f"  Coefficient of variation: {rayon_cv:.2f}%")
            print()

            print("Pinned (custom thread pool with core affinity):")
            print(f"  Mean: {pinned_avg_mean / 1_000_000:.3f} ms")
            print(f"  Variance: ±{pinned_avg_variance / 1_000_000:.3f} ms")
            print(f"  Coefficient of variation: {pinned_cv:.2f}%")
            print()

            # Calculate improvement
            variance_reduction = ((rayon_avg_variance - pinned_avg_variance) / rayon_avg_variance) * 100
            cv_reduction = ((rayon_cv - pinned_cv) / rayon_cv) * 100

            print("Variance Reduction:")
            print(f"  Absolute: {variance_reduction:+.1f}% (pinned vs Rayon)")
            print(f"  CV reduction: {cv_reduction:+.1f}%")
            print()

            if pinned_avg_variance < rayon_avg_variance:
                print(f"✓ Pinned thread pool shows {abs(variance_reduction):.1f}% LOWER variance")
                print(f"  This means MORE CONSISTENT timing measurements")
            else:
                print(f"✗ Pinned thread pool shows {abs(variance_reduction):.1f}% HIGHER variance")
                print(f"  This is unexpected - thread pinning should reduce variance")

            # Performance comparison
            speedup = rayon_avg_mean / pinned_avg_mean
            print()
            print("Performance Comparison:")
            if speedup > 1.0:
                print(f"  Pinned is {speedup:.2f}x FASTER than Rayon")
            else:
                print(f"  Rayon is {1/speedup:.2f}x FASTER than Pinned")


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <benchmark_output.txt>", file=sys.stderr)
        return 1

    input_file = Path(sys.argv[1])

    if not input_file.exists():
        print(f"Error: File not found: {input_file}", file=sys.stderr)
        return 1

    results = parse_criterion_output(input_file)

    if not results:
        print("No benchmark results found in file", file=sys.stderr)
        return 1

    analyze_variance(results)
    return 0


if __name__ == "__main__":
    sys.exit(main())
