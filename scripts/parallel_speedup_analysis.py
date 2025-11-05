#!/usr/bin/env python3
"""
Parallel Speedup Analysis Script for MTG Forge-rs Benchmarks

This script analyzes parallel scaling behavior across different allocators
and thread counts for the pinned thread pool benchmark.

## Usage

### Quick Mode (recommended for testing - completes in minutes)
```bash
# Dry-run to see what would execute (15 runs: 3 allocators × 5 thread counts)
python3 scripts/parallel_speedup_analysis.py --dry-run --quick

# Run quick benchmarks with plotting (1s per benchmark, tests 1/25%/50%/75%/100% threads)
python3 scripts/parallel_speedup_analysis.py --run-benchmarks --quick --plot
```

### Full Analysis (comprehensive - may take hours)
```bash
# Dry-run to see what would execute (96 runs: 3 allocators × 32 thread counts)
python3 scripts/parallel_speedup_analysis.py --dry-run

# Run all benchmarks with plotting (10s per benchmark, tests all thread counts)
python3 scripts/parallel_speedup_analysis.py --run-benchmarks --plot
```

### Plot Only (from existing data)
```bash
python3 scripts/parallel_speedup_analysis.py --input experiment_results/parallel_speedup_*.csv --plot
```

## Output

- CSV data: experiment_results/parallel_speedup_YYYY-MM-DD.csv
- Plot: experiment_results/plots/parallel_speedup_YYYY-MM-DD.png

## Benchmark Configuration

The script collects data for:
- Allocators: system (default), mimalloc, jemalloc
- Thread counts:
  - Quick mode: 1, 25%, 50%, 75%, 100% of physical cores
  - Full mode: 1 to num_physical_cores (all thread counts)
- Benchmark: pinned_par_rewind_play_again
- Measurement time:
  - Quick mode: 1 second per benchmark
  - Full mode: 10 seconds per benchmark

## Implementation

The benchmark supports parameterization via environment variables:
- BENCH_NUM_THREADS: Number of threads to use
- BENCH_MEASUREMENT_TIME_SECS: Criterion measurement duration

Timing data is extracted from Criterion's JSON output:
- Mean estimate and standard deviation
- Confidence intervals
- Throughput calculations (turns/sec)

## Metrics Calculated

- Turns/sec at each thread count
- Speedup relative to single-threaded
- Parallel efficiency = speedup / num_threads
- Perfect linear speedup reference line
"""

import argparse
import csv
import json
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import List, Optional, Tuple

# Try to import matplotlib
try:
    import matplotlib.pyplot as plt
    import matplotlib
    matplotlib.use('Agg')  # Non-interactive backend
    HAS_MATPLOTLIB = True
except ImportError:
    HAS_MATPLOTLIB = False
    print("Warning: matplotlib not available. Plotting will be disabled.", file=sys.stderr)


@dataclass
class BenchmarkResult:
    """Results from a single benchmark run"""
    allocator: str
    num_threads: int
    mean_time_ns: float
    std_dev_ns: float
    turns_per_sec: float
    bytes_per_turn: float
    timestamp: str
    git_commit: str


class ParallelSpeedupAnalyzer:
    """Analyze parallel speedup across allocators and thread counts"""

    def __init__(self, workspace_root: Path, quick_mode: bool = False, hyperthreads: bool = False):
        self.workspace_root = workspace_root
        self.results_dir = workspace_root / "experiment_results"
        self.plots_dir = self.results_dir / "plots"
        self.plots_dir.mkdir(parents=True, exist_ok=True)

        # Get number of physical cores
        self.num_physical_cores = self._get_physical_cores()
        self.quick_mode = quick_mode
        self.hyperthreads = hyperthreads

        # Allocator configurations
        self.allocators = [
            ("system", None),  # Default allocator
            ("mimalloc", "bench-mimalloc"),
            ("jemalloc", "bench-jemalloc"),
        ]

    def _get_physical_cores(self) -> int:
        """Get number of physical CPU cores"""
        try:
            import os
            # This works on Linux
            cores = len(set(
                int(open(f"/sys/devices/system/cpu/cpu{i}/topology/core_id").read())
                for i in range(os.cpu_count() or 1)
                if Path(f"/sys/devices/system/cpu/cpu{i}/topology/core_id").exists()
            ))
            return cores if cores > 0 else (os.cpu_count() or 1) // 2
        except:
            # Fallback: assume half of logical cores are physical
            import os
            return (os.cpu_count() or 1) // 2

    def _get_thread_counts(self) -> List[int]:
        """Get list of thread counts to test"""
        if self.quick_mode:
            # Quick mode: 1, 25%, 50%, 75%, 100% of available threads
            counts = [
                1,
                max(1, self.num_physical_cores // 4),  # 25%
                max(1, self.num_physical_cores // 2),  # 50%
                max(1, 3 * self.num_physical_cores // 4),  # 75%
                self.num_physical_cores  # 100%
            ]
        else:
            # Full mode: all thread counts from 1 to num_physical_cores
            counts = list(range(1, self.num_physical_cores + 1))

        # Add hyperthreading test points if requested
        if self.hyperthreads:
            # Add 1.5x and 2x physical cores
            counts.extend([
                int(1.5 * self.num_physical_cores),  # 1.5x
                2 * self.num_physical_cores  # 2x
            ])

        # Remove duplicates and sort
        return sorted(set(counts))

    def _get_measurement_time(self) -> int:
        """Get measurement time in seconds for Criterion"""
        return 1 if self.quick_mode else 10

    def _get_git_info(self) -> Tuple[str, str]:
        """Get current git commit hash and depth"""
        try:
            commit = subprocess.check_output(
                ["git", "rev-parse", "--short", "HEAD"],
                cwd=self.workspace_root,
                text=True
            ).strip()

            depth = subprocess.check_output(
                ["git", "rev-list", "--count", "HEAD"],
                cwd=self.workspace_root,
                text=True
            ).strip()

            return commit, depth
        except:
            return "unknown", "0"

    def run_benchmark(self, allocator_name: str, feature: Optional[str],
                     num_threads: int, dry_run: bool = False) -> Optional[BenchmarkResult]:
        """
        Run benchmark with specific allocator and thread count

        NOTE: Currently the benchmark is hardcoded to use all physical cores.
        This function shows the INTENDED workflow once the benchmark supports
        parameterized thread counts.
        """
        print(f"\n{'[DRY-RUN] ' if dry_run else ''}Running benchmark: "
              f"{allocator_name} with {num_threads} threads")

        # Build command
        cmd = ["cargo", "bench", "--bench", "game_benchmark"]

        if feature:
            cmd.extend(["--features", feature])

        # Add filter to run only the pinned parallel benchmark
        cmd.append("pinned_par_rewind_play_again")

        # Set thread count and measurement time via environment variables
        measurement_time = self._get_measurement_time()
        env = {
            "BENCH_NUM_THREADS": str(num_threads),
            "BENCH_MEASUREMENT_TIME_SECS": str(measurement_time),
            **subprocess.os.environ.copy()
        }

        print(f"  Command: {' '.join(cmd)}")
        print(f"  Env: BENCH_NUM_THREADS={num_threads}, BENCH_MEASUREMENT_TIME_SECS={measurement_time}s")

        if dry_run:
            print("  [Would run benchmark here]")
            return None

        try:
            # Run benchmark
            result = subprocess.run(
                cmd,
                cwd=self.workspace_root / "mtg-benchmarks",
                env=env,
                capture_output=True,
                text=True,
                timeout=600  # 10 minute timeout
            )

            if result.returncode != 0:
                print(f"  ERROR: Benchmark failed")
                print(f"  stderr: {result.stderr}")
                return None

            # Parse Criterion JSON output
            # The results are in target/criterion/game_execution/pinned_par_rewind_play_again/
            criterion_dir = self.workspace_root / "target" / "criterion" / \
                           "game_execution" / "pinned_par_rewind_play_again"

            estimates_file = criterion_dir / "base" / "estimates.json"

            if not estimates_file.exists():
                print(f"  ERROR: Could not find estimates file: {estimates_file}")
                return None

            with open(estimates_file) as f:
                estimates = json.load(f)

            mean_ns = estimates["mean"]["point_estimate"]
            std_dev_ns = estimates["std_dev"]["point_estimate"]

            # Calculate turns/sec
            # NOTE: This is a placeholder - actual calculation depends on
            # how many iterations were run and game state
            # For now, use inverse of mean time as proxy
            turns_per_sec = 1e9 / mean_ns  # Placeholder

            # Get bytes/turn from allocator stats
            # This would need to be extracted from benchmark output
            bytes_per_turn = 0.0  # Placeholder

            commit, _depth = self._get_git_info()
            timestamp = datetime.now().isoformat()

            return BenchmarkResult(
                allocator=allocator_name,
                num_threads=num_threads,
                mean_time_ns=mean_ns,
                std_dev_ns=std_dev_ns,
                turns_per_sec=turns_per_sec,
                bytes_per_turn=bytes_per_turn,
                timestamp=timestamp,
                git_commit=commit,
            )

        except subprocess.TimeoutExpired:
            print(f"  ERROR: Benchmark timed out")
            return None
        except Exception as e:
            print(f"  ERROR: {e}")
            return None

    def run_full_analysis(self, dry_run: bool = False) -> List[BenchmarkResult]:
        """Run benchmarks for all allocators and thread counts"""
        results = []

        thread_counts = self._get_thread_counts()
        mode_str = "Quick Mode" if self.quick_mode else "Full Analysis"
        measurement_time = self._get_measurement_time()

        print(f"\n{'='*70}")
        print(f"Parallel Speedup Analysis - {mode_str}")
        print(f"{'='*70}")
        print(f"Physical cores: {self.num_physical_cores}")
        print(f"Thread counts to test: {thread_counts}")
        print(f"Measurement time: {measurement_time}s per benchmark")
        print(f"Allocators: {', '.join(a[0] for a in self.allocators)}")
        print(f"Total runs: {len(self.allocators)} allocators × {len(thread_counts)} thread counts = {len(self.allocators) * len(thread_counts)}")
        print(f"{'='*70}\n")

        for allocator_name, feature in self.allocators:
            print(f"\n{'='*70}")
            print(f"Allocator: {allocator_name}")
            print(f"{'='*70}")

            for num_threads in thread_counts:
                result = self.run_benchmark(
                    allocator_name,
                    feature,
                    num_threads,
                    dry_run
                )

                if result:
                    results.append(result)
                    print(f"  ✓ {num_threads} threads: {result.mean_time_ns/1e6:.2f}ms")

        return results

    def save_results(self, results: List[BenchmarkResult], output_file: Optional[Path] = None):
        """Save results to CSV file"""
        if not results:
            print("No results to save")
            return

        if output_file is None:
            date_str = datetime.now().strftime("%Y-%m-%d")
            output_file = self.results_dir / f"parallel_speedup_{date_str}.csv"

        print(f"\nSaving results to {output_file}")

        with open(output_file, 'w', newline='') as f:
            writer = csv.writer(f)
            writer.writerow([
                "timestamp", "git_commit", "allocator", "num_threads",
                "mean_time_ns", "std_dev_ns", "turns_per_sec", "bytes_per_turn"
            ])

            for r in results:
                writer.writerow([
                    r.timestamp, r.git_commit, r.allocator, r.num_threads,
                    r.mean_time_ns, r.std_dev_ns, r.turns_per_sec, r.bytes_per_turn
                ])

        print(f"✓ Saved {len(results)} results")

    def load_results(self, input_file: Path) -> List[BenchmarkResult]:
        """Load results from CSV file"""
        results = []

        with open(input_file, 'r') as f:
            reader = csv.DictReader(f)
            for row in reader:
                results.append(BenchmarkResult(
                    allocator=row["allocator"],
                    num_threads=int(row["num_threads"]),
                    mean_time_ns=float(row["mean_time_ns"]),
                    std_dev_ns=float(row["std_dev_ns"]),
                    turns_per_sec=float(row["turns_per_sec"]),
                    bytes_per_turn=float(row["bytes_per_turn"]),
                    timestamp=row["timestamp"],
                    git_commit=row["git_commit"],
                ))

        return results

    def analyze_max_throughput(self, results: List[BenchmarkResult]):
        """Analyze and print maximum throughput for each allocator"""
        if not results:
            print("\nNo results to analyze")
            return

        print(f"\n{'='*70}")
        print("MAXIMUM THROUGHPUT ANALYSIS")
        print(f"{'='*70}\n")

        # Group results by allocator
        allocator_results = {}
        for r in results:
            if r.allocator not in allocator_results:
                allocator_results[r.allocator] = []
            allocator_results[r.allocator].append(r)

        # Find single-threaded and parallel max for each allocator
        single_threaded_results = []
        max_parallel_results = []

        for allocator in sorted(allocator_results.keys()):
            alloc_results = allocator_results[allocator]

            # Get single-threaded result
            single_threaded = [r for r in alloc_results if r.num_threads == 1]
            if single_threaded:
                single_threaded_results.append((allocator, single_threaded[0]))

            # Get max parallel result
            max_result = max(alloc_results, key=lambda r: r.turns_per_sec)
            max_parallel_results.append((allocator, max_result))

        # Find winners
        best_single_allocator, best_single = max(single_threaded_results, key=lambda x: x[1].turns_per_sec)
        best_parallel_allocator, best_parallel = max(max_parallel_results, key=lambda x: x[1].turns_per_sec)

        # Print single-threaded comparison
        print("SINGLE-THREADED PERFORMANCE")
        print("-" * 70)
        for allocator, result in single_threaded_results:
            is_winner = (allocator == best_single_allocator)
            pct_diff = ((best_single.turns_per_sec - result.turns_per_sec) / best_single.turns_per_sec) * 100

            winner_mark = " 👑 WINNER" if is_winner else f" ({pct_diff:.1f}% slower)"
            print(f"  {allocator:12} {result.turns_per_sec:15,.2f} turns/sec{winner_mark}")
        print()

        # Print parallel performance comparison
        print("PARALLEL PERFORMANCE (Maximum Throughput)")
        print("-" * 70)
        for allocator, max_result in max_parallel_results:
            is_winner = (allocator == best_parallel_allocator)
            pct_diff = ((best_parallel.turns_per_sec - max_result.turns_per_sec) / best_parallel.turns_per_sec) * 100

            winner_mark = " 👑 WINNER" if is_winner else f" ({pct_diff:.1f}% slower)"
            print(f"  {allocator:12} {max_result.turns_per_sec:15,.2f} turns/sec at {max_result.num_threads:2} threads{winner_mark}")
        print()

        # Detailed results for each allocator
        print("DETAILED RESULTS BY ALLOCATOR")
        print("-" * 70)
        for allocator, max_result in max_parallel_results:
            print(f"\n{allocator}:")
            print(f"  Maximum throughput: {max_result.turns_per_sec:,.2f} turns/sec")
            print(f"  Achieved at: {max_result.num_threads} threads")
            print(f"  Mean time: {max_result.mean_time_ns/1e6:.4f} ms")
            print(f"  Std dev: ±{max_result.std_dev_ns/1e6:.4f} ms")

            # Calculate speedup relative to single-threaded
            single_threaded = [r for r in allocator_results[allocator] if r.num_threads == 1]
            if single_threaded:
                speedup = max_result.turns_per_sec / single_threaded[0].turns_per_sec
                efficiency = (speedup / max_result.num_threads) * 100
                print(f"  Speedup vs 1 thread: {speedup:.2f}x")
                print(f"  Parallel efficiency: {efficiency:.1f}%")

        print(f"\n{'='*70}")
        print(f"SUMMARY")
        print(f"{'='*70}")
        print(f"Single-threaded winner: {best_single_allocator} ({best_single.turns_per_sec:,.2f} turns/sec)")
        print(f"Parallel winner: {best_parallel_allocator} ({best_parallel.turns_per_sec:,.2f} turns/sec at {best_parallel.num_threads} threads)")
        print(f"{'='*70}\n")

    def plot_speedup(self, results: List[BenchmarkResult], output_file: Optional[Path] = None):
        """Generate speedup plot"""
        if not HAS_MATPLOTLIB:
            print("\nERROR: matplotlib not available. Cannot generate plot.", file=sys.stderr)
            print("Install with: pip install matplotlib", file=sys.stderr)
            return

        if not results:
            print("No results to plot")
            return

        if output_file is None:
            date_str = datetime.now().strftime("%Y-%m-%d")
            output_file = self.plots_dir / f"parallel_speedup_{date_str}.png"

        print(f"\nGenerating plot: {output_file}")

        # Organize results by allocator
        allocator_results = {}
        for r in results:
            if r.allocator not in allocator_results:
                allocator_results[r.allocator] = []
            allocator_results[r.allocator].append((r.num_threads, r.turns_per_sec))

        # Sort by thread count
        for alloc in allocator_results:
            allocator_results[alloc].sort(key=lambda x: x[0])

        # Create plot
        fig, ax = plt.subplots(figsize=(12, 8))

        # Plot each allocator
        colors = {'system': 'blue', 'mimalloc': 'red', 'jemalloc': 'green'}
        markers = {'system': 'o', 'mimalloc': 's', 'jemalloc': '^'}

        for allocator, data in allocator_results.items():
            threads = [d[0] for d in data]
            throughput = [d[1] for d in data]

            ax.plot(threads, throughput,
                   label=allocator,
                   color=colors.get(allocator, 'black'),
                   marker=markers.get(allocator, 'o'),
                   linewidth=2,
                   markersize=8)

        # Add perfect linear speedup reference
        if results:
            baseline = min(r.turns_per_sec for r in results if r.num_threads == 1)
            max_threads = max(r.num_threads for r in results)
            perfect_speedup = [baseline * i for i in range(1, max_threads + 1)]
            ax.plot(range(1, max_threads + 1), perfect_speedup,
                   'k--', alpha=0.3, linewidth=1, label='Perfect Linear Speedup')

        ax.set_xlabel('Number of Threads (Pinned to Physical Cores)', fontsize=12)
        ax.set_ylabel('Throughput (turns/sec)', fontsize=12)
        ax.set_title('Parallel Speedup Analysis: MTG Forge-rs Benchmark\n'
                    f'Pinned Thread Pool (Physical Cores: {self.num_physical_cores})',
                    fontsize=14)
        ax.legend(fontsize=10)
        ax.grid(True, alpha=0.3)

        # Add annotation with bytes/turn if available
        if results and any(r.bytes_per_turn > 0 for r in results):
            avg_bytes = sum(r.bytes_per_turn for r in results) / len(results)
            ax.text(0.02, 0.98, f'Avg bytes allocated/turn: {avg_bytes:.0f}',
                   transform=ax.transAxes, fontsize=10,
                   verticalalignment='top',
                   bbox=dict(boxstyle='round', facecolor='wheat', alpha=0.5))

        plt.tight_layout()
        plt.savefig(output_file, dpi=300)
        print(f"✓ Plot saved: {output_file}")


def main():
    parser = argparse.ArgumentParser(
        description="Analyze parallel speedup for MTG Forge-rs benchmarks",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__
    )

    parser.add_argument("--run-benchmarks", action="store_true",
                       help="Run benchmarks for all allocators and thread counts")
    parser.add_argument("--dry-run", action="store_true",
                       help="Show what would be run without actually running benchmarks")
    parser.add_argument("--quick", action="store_true",
                       help="Quick mode: 1s per benchmark, test only 1/25%%/50%%/75%%/100%% thread counts")
    parser.add_argument("--hyperthreads", action="store_true",
                       help="Also test at 1.5x and 2x physical cores to observe hyperthreading effects")
    parser.add_argument("--plot", action="store_true",
                       help="Generate speedup plot (requires matplotlib)")
    parser.add_argument("--input", type=Path,
                       help="Input CSV file to load results from")
    parser.add_argument("--output-data", type=Path,
                       help="Output CSV file for benchmark results")
    parser.add_argument("--output-plot", type=Path,
                       help="Output PNG file for speedup plot")

    args = parser.parse_args()

    # Find workspace root
    script_dir = Path(__file__).parent
    workspace_root = script_dir.parent

    analyzer = ParallelSpeedupAnalyzer(workspace_root, quick_mode=args.quick, hyperthreads=args.hyperthreads)

    results = []

    # Run benchmarks if requested
    if args.run_benchmarks or args.dry_run:
        results = analyzer.run_full_analysis(dry_run=args.dry_run)

        if results and not args.dry_run:
            analyzer.save_results(results, args.output_data)
            # Analyze and print maximum throughput for each allocator
            analyzer.analyze_max_throughput(results)

    # Load results from file if specified
    elif args.input:
        results = analyzer.load_results(args.input)
        print(f"Loaded {len(results)} results from {args.input}")
        # Analyze loaded results
        analyzer.analyze_max_throughput(results)

    # Generate plot if requested
    if args.plot and results:
        analyzer.plot_speedup(results, args.output_plot)

    # Show usage if no action specified
    if not (args.run_benchmarks or args.dry_run or args.plot or args.input):
        print("\nNo action specified. Use --help for usage information.\n")
        print("Common workflows:")
        print("  1. Dry run (show commands):  python3 scripts/parallel_speedup_analysis.py --dry-run")
        print("  2. Run and plot:             python3 scripts/parallel_speedup_analysis.py --run-benchmarks --plot")
        print("  3. Plot from existing data:  python3 scripts/parallel_speedup_analysis.py --input FILE --plot")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
