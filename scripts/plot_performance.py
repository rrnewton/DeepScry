#!/usr/bin/env python3
"""
Plot performance metrics from benchmark history CSV.

Generates time-series plots showing performance trends and regressions.
Requires Python 3 with pandas and matplotlib installed.

Usage: ./scripts/plot_performance.py [options]

Options:
  --input FILE      Input CSV file (default: experiment_results/perf_history.csv)
  --output DIR      Output directory for plots (default: experiment_results/plots)
  --benchmark NAME  Filter to specific benchmark (default: all)
  --metric METRIC   Plot specific metric (default: all)
  --format FORMAT   Output format: png, svg, pdf (default: png)
  --show            Show plots interactively (default: save only)

Examples:
  ./scripts/plot_performance.py
  ./scripts/plot_performance.py --benchmark fresh --metric games_per_sec
  ./scripts/plot_performance.py --show
"""

import argparse
import sys
from pathlib import Path

try:
    import pandas as pd
    import matplotlib
    matplotlib.use('Agg')  # Non-interactive backend
    import matplotlib.pyplot as plt
    import matplotlib.dates as mdates
    from datetime import datetime
except ImportError as e:
    print(f"Error: Missing required Python package: {e}", file=sys.stderr)
    print("Install with: pip install pandas matplotlib", file=sys.stderr)
    sys.exit(1)


def parse_args():
    """Parse command line arguments."""
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--input', default='experiment_results/perf_history.csv',
                        help='Input CSV file')
    parser.add_argument('--output', default='experiment_results/plots',
                        help='Output directory for plots')
    parser.add_argument('--benchmark', default=None,
                        help='Filter to specific benchmark')
    parser.add_argument('--metric', default=None,
                        help='Plot specific metric only')
    parser.add_argument('--format', default='png', choices=['png', 'svg', 'pdf'],
                        help='Output format')
    parser.add_argument('--show', action='store_true',
                        help='Show plots interactively')
    return parser.parse_args()


def load_data(csv_file):
    """Load and preprocess performance history data."""
    if not Path(csv_file).exists():
        print(f"Error: File not found: {csv_file}", file=sys.stderr)
        sys.exit(1)

    df = pd.read_csv(csv_file)

    # Convert timestamp to datetime
    df['timestamp'] = pd.to_datetime(df['timestamp'])

    # Sort by git_depth for chronological ordering
    df = df.sort_values('git_depth')

    return df


def detect_regressions(series, threshold=0.15):
    """
    Detect performance regressions in a time series.

    Returns list of (index, value, regression_pct) tuples where regression > threshold.
    threshold: fraction of performance drop to consider a regression (default: 15%)
    """
    regressions = []
    if len(series) < 2:
        return regressions

    # Use rolling maximum as baseline (best performance so far)
    baseline = series.expanding().max()

    for i in range(1, len(series)):
        if pd.isna(series.iloc[i]) or pd.isna(baseline.iloc[i]):
            continue

        current = series.iloc[i]
        best = baseline.iloc[i-1]

        if best > 0:
            regression_pct = (best - current) / best
            if regression_pct > threshold:
                regressions.append((i, current, regression_pct))

    return regressions


def plot_metric(df, metric_col, ylabel, title, output_path, show=False):
    """
    Plot a single metric over time for all benchmarks.

    Creates a line plot with:
    - Separate lines for each benchmark type
    - Regression markers
    - Git depth on x-axis
    """
    fig, ax = plt.subplots(figsize=(12, 6))

    benchmarks = df['benchmark_name'].unique()
    colors = plt.cm.tab10(range(len(benchmarks)))

    for benchmark, color in zip(benchmarks, colors):
        bench_df = df[df['benchmark_name'] == benchmark].copy()

        if bench_df.empty or metric_col not in bench_df.columns:
            continue

        # Plot line
        ax.plot(bench_df['git_depth'], bench_df[metric_col],
                marker='o', label=benchmark, color=color, alpha=0.7)

        # Detect and mark regressions
        regressions = detect_regressions(bench_df[metric_col])
        if regressions:
            reg_indices, reg_values, reg_pcts = zip(*regressions)
            reg_depths = bench_df.iloc[list(reg_indices)]['git_depth'].values
            ax.scatter(reg_depths, reg_values, color='red', marker='x', s=100, zorder=5)

            # Annotate worst regression
            worst_idx = max(range(len(reg_pcts)), key=lambda i: reg_pcts[i])
            ax.annotate(f'-{reg_pcts[worst_idx]*100:.1f}%',
                       xy=(reg_depths[worst_idx], reg_values[worst_idx]),
                       xytext=(10, -10), textcoords='offset points',
                       bbox=dict(boxstyle='round,pad=0.5', fc='yellow', alpha=0.7),
                       arrowprops=dict(arrowstyle='->', connectionstyle='arc3,rad=0'))

    ax.set_xlabel('Git Depth (commit count)')
    ax.set_ylabel(ylabel)
    ax.set_title(title)
    ax.set_yscale('log')  # Use logarithmic scale for Y axis
    ax.legend(loc='best')
    ax.grid(True, alpha=0.3, which='both')  # Show grid for major and minor ticks

    plt.tight_layout()

    if show:
        plt.show()
    else:
        plt.savefig(output_path, dpi=150, bbox_inches='tight')
        print(f"Saved: {output_path}")

    plt.close()


def plot_all_metrics(df, output_dir, fmt='png', show=False, filter_benchmark=None, filter_metric=None):
    """Generate plots for all key metrics."""

    if filter_benchmark:
        df = df[df['benchmark_name'] == filter_benchmark]
        if df.empty:
            print(f"Warning: No data for benchmark '{filter_benchmark}'", file=sys.stderr)
            return

    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    # Define metrics to plot
    metrics = [
        ('games_per_sec', 'Games per Second', 'Throughput: Games per Second'),
        ('actions_per_sec', 'Actions per Second', 'Throughput: Actions per Second'),
        ('avg_duration_ms_per_game', 'Duration (ms)', 'Average Game Duration'),
        ('avg_bytes_per_game', 'Bytes', 'Memory: Average Bytes per Game'),
        ('bytes_per_turn', 'Bytes per Turn', 'Memory: Bytes per Turn'),
        ('actions_per_turn', 'Actions per Turn', 'Game Complexity: Actions per Turn'),
        ('avg_turns_per_game', 'Turns per Game', 'Game Length: Average Turns'),
    ]

    # Filter to specific metric if requested
    if filter_metric:
        metrics = [(col, ylabel, title) for col, ylabel, title in metrics if col == filter_metric]
        if not metrics:
            print(f"Error: Unknown metric '{filter_metric}'", file=sys.stderr)
            print(f"Available metrics: {', '.join([m[0] for m in metrics])}", file=sys.stderr)
            sys.exit(1)

    for metric_col, ylabel, title in metrics:
        if metric_col not in df.columns:
            print(f"Warning: Metric '{metric_col}' not found in data", file=sys.stderr)
            continue

        output_file = output_dir / f"{metric_col}.{fmt}"
        plot_metric(df, metric_col, ylabel, title, output_file, show)


def print_summary(df):
    """Print summary statistics of performance history."""
    print("\n=== Performance History Summary ===")
    print(f"Total entries: {len(df)}")
    print(f"Benchmarks: {', '.join(df['benchmark_name'].unique())}")
    print(f"Commit range: {df['git_depth'].min()} - {df['git_depth'].max()}")
    print(f"Time range: {df['timestamp'].min()} - {df['timestamp'].max()}")

    print("\n=== Latest Performance ===")
    latest = df.sort_values('git_depth').groupby('benchmark_name').last()
    for benchmark in latest.index:
        row = latest.loc[benchmark]
        print(f"\n{benchmark} (commit {row['git_commit']}, depth {row['git_depth']}):")
        print(f"  Games/sec: {row.get('games_per_sec', 'N/A'):.2f}")
        print(f"  Avg duration: {row.get('avg_duration_ms_per_game', 'N/A'):.2f} ms")
        print(f"  Avg bytes/game: {row.get('avg_bytes_per_game', 'N/A'):.0f}")
        print(f"  Actions/turn: {row.get('actions_per_turn', 'N/A'):.2f}")

    print("\n=== Regressions Detected ===")
    for benchmark in df['benchmark_name'].unique():
        bench_df = df[df['benchmark_name'] == benchmark].sort_values('git_depth')
        if 'games_per_sec' in bench_df.columns:
            regressions = detect_regressions(bench_df['games_per_sec'])
            if regressions:
                print(f"\n{benchmark} - games_per_sec:")
                for idx, val, pct in regressions[:3]:  # Show top 3
                    commit = bench_df.iloc[idx]['git_commit']
                    depth = bench_df.iloc[idx]['git_depth']
                    print(f"  Commit {commit} (depth {depth}): -{pct*100:.1f}% ({val:.2f} games/sec)")


def main():
    args = parse_args()

    # Load data
    print(f"Loading performance history from: {args.input}")
    df = load_data(args.input)

    # Print summary
    print_summary(df)

    # Generate plots
    print(f"\n=== Generating Plots ===")
    print(f"Output directory: {args.output}")
    print(f"Format: {args.format}")

    plot_all_metrics(df, args.output, fmt=args.format, show=args.show,
                     filter_benchmark=args.benchmark, filter_metric=args.metric)

    print(f"\n=== Done ===")


if __name__ == '__main__':
    main()
