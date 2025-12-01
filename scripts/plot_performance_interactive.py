#!/usr/bin/env python3
"""
Generate interactive performance dashboard using Plotly.

Creates a standalone HTML dashboard with interactive plots showing:
- Performance trends over git history
- Hover tooltips with exact values
- Toggleable benchmark series
- Zoom and pan capabilities
- Regression markers

Usage: ./scripts/plot_performance_interactive.py [options]

Options:
  --input FILE      Input CSV file (default: experiment_results/AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores/perf_history.csv)
  --output FILE     Output HTML file (default: experiment_results/performance_dashboard.html)
  --benchmark NAME  Filter to specific benchmark (default: all)

Examples:
  ./scripts/plot_performance_interactive.py
  ./scripts/plot_performance_interactive.py --input custom.csv --output dashboard.html
"""

import argparse
import sys
from pathlib import Path
from datetime import datetime

try:
    import pandas as pd
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
except ImportError as e:
    print(f"Error: Missing required Python package: {e}", file=sys.stderr)
    print("Install with: pip install pandas plotly", file=sys.stderr)
    sys.exit(1)


def parse_args():
    """Parse command line arguments."""
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        '--input',
        default='experiment_results/AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores/perf_history.csv',
        help='Input CSV file'
    )
    parser.add_argument(
        '--output',
        default='experiment_results/AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores/performance_dashboard.html',
        help='Output HTML file'
    )
    parser.add_argument(
        '--benchmark',
        default=None,
        help='Filter to specific benchmark'
    )
    return parser.parse_args()


def load_data(csv_file):
    """Load and preprocess performance history data."""
    csv_path = Path(csv_file)
    if not csv_path.exists():
        print(f"Error: File not found: {csv_file}", file=sys.stderr)
        sys.exit(1)

    df = pd.read_csv(csv_file)

    # Convert timestamp to datetime
    df['timestamp'] = pd.to_datetime(df['timestamp'])

    # Sort by git_depth for chronological ordering
    df = df.sort_values('git_depth')

    return df


def detect_regressions(series, threshold=0.05):
    """
    Detect immediate performance regressions (commit-to-commit drops).

    Returns list of (index, value, regression_pct) tuples where regression > threshold.
    Only detects regressions from one commit to the next, not vs historical best.

    threshold: fraction of performance drop to consider a regression (default: 5%)
    """
    regressions = []
    if len(series) < 2:
        return regressions

    # Check each commit against the immediate previous commit
    for i in range(1, len(series)):
        if pd.isna(series.iloc[i]) or pd.isna(series.iloc[i-1]):
            continue

        current = series.iloc[i]
        previous = series.iloc[i-1]

        if previous > 0:
            regression_pct = (previous - current) / previous
            if regression_pct > threshold:
                regressions.append((i, current, regression_pct))

    return regressions


def parse_deck_variant(benchmark_name):
    """
    Parse benchmark name into deck and variant components.

    Args:
        benchmark_name: String like "robots_mirror/rewind_play_again" or "rewind"

    Returns:
        Tuple of (deck, variant)
    """
    if '/' in benchmark_name:
        parts = benchmark_name.split('/', 1)
        return (parts[0], parts[1])
    else:
        # Edge case: "rewind" has no '/'
        return (benchmark_name, "(standalone)")


def extract_decks_variants(df):
    """
    Extract sorted lists of unique decks and variants from benchmark names.

    Args:
        df: DataFrame with 'benchmark_name' column

    Returns:
        Tuple of (sorted list of decks, sorted list of variants)
    """
    decks = set()
    variants = set()

    for benchmark_name in df['benchmark_name'].unique():
        deck, variant = parse_deck_variant(benchmark_name)
        decks.add(deck)
        variants.add(variant)

    return sorted(decks), sorted(variants)


def create_metric_plot(df, metric_col, ylabel, title, min_depth=100, max_depth=None):
    """
    Create an interactive Plotly figure for a single metric.

    Args:
        df: DataFrame with performance data
        metric_col: Column name for the metric to plot
        ylabel: Y-axis label
        title: Plot title
        min_depth: Minimum git depth to display (default: 100)
        max_depth: Maximum git depth (auto-detected if None)

    Returns a Plotly figure object.
    """
    fig = go.Figure()

    benchmarks = sorted(df['benchmark_name'].unique())
    trace_count = 0
    regression_trace_indices = []  # Track which traces are regressions

    # Determine the full range of git depths for slider
    if max_depth is None:
        max_depth = int(df['git_depth'].max())
    abs_min_depth = int(df['git_depth'].min())

    for benchmark in benchmarks:
        bench_df = df[df['benchmark_name'] == benchmark].copy()

        if bench_df.empty or metric_col not in bench_df.columns:
            continue

        # Skip if all values are NaN
        if bench_df[metric_col].isna().all():
            continue

        # Parse deck and variant for filtering
        deck, variant = parse_deck_variant(benchmark)

        # Add main line trace
        trace_count += 1
        hover_text = []
        for _, row in bench_df.iterrows():
            hover_text.append(
                f"<b>{benchmark}</b><br>"
                f"Deck: {deck}<br>"
                f"Variant: {variant}<br>"
                f"Git Depth: {row['git_depth']}<br>"
                f"Commit: {row['git_commit'][:8]}<br>"
                f"Branch: {row.get('git_branch', 'N/A')}<br>"
                f"Date: {row['timestamp'].strftime('%Y-%m-%d')}<br>"
                f"{ylabel}: {row[metric_col]:,.2f}<br>"
                f"Games: {row.get('num_games', 'N/A')}"
            )

        fig.add_trace(go.Scatter(
            x=bench_df['git_depth'].tolist(),  # Convert to list for better compatibility
            y=bench_df[metric_col].tolist(),
            mode='lines+markers',
            name=benchmark,
            hovertext=hover_text,
            hoverinfo='text',
            line=dict(width=2),
            marker=dict(size=6),
            visible=True,  # Explicitly set visibility
            legendgroup=benchmark,  # Group with regression markers
            customdata=[[deck, variant]] * len(bench_df)  # Add deck/variant metadata
        ))

        # Detect and mark regressions
        regressions = detect_regressions(bench_df[metric_col])
        if regressions:
            reg_indices, reg_values, reg_pcts = zip(*regressions)
            reg_depths = bench_df.iloc[list(reg_indices)]['git_depth'].values
            reg_commits = bench_df.iloc[list(reg_indices)]['git_commit'].values

            reg_hover = [
                f"<b>REGRESSION</b><br>"
                f"{benchmark}<br>"
                f"Commit: {commit[:8]}<br>"
                f"Drop: -{pct*100:.1f}%<br>"
                f"{ylabel}: {val:,.2f}"
                for commit, pct, val in zip(reg_commits, reg_pcts, reg_values)
            ]

            # Track this regression trace index
            regression_trace_indices.append(len(fig.data))

            fig.add_trace(go.Scatter(
                x=reg_depths.tolist(),
                y=list(reg_values),
                mode='markers',
                name=f'{benchmark} (regressions)',
                marker=dict(
                    symbol='x',
                    size=12,
                    color='red',
                    line=dict(width=2)
                ),
                hovertext=reg_hover,
                hoverinfo='text',
                showlegend=False,
                visible=True,
                legendgroup=benchmark,  # Link to parent benchmark trace
                customdata=[[deck, variant]] * len(reg_depths)  # Add deck/variant metadata
            ))

    # Determine if log scale is appropriate (need positive values)
    all_values = df[metric_col].dropna()
    use_log = len(all_values) > 0 and (all_values > 0).all()

    # Note: Regression visibility now controlled by global HTML buttons
    # (removed per-plot buttons for cleaner UX)

    # Don't create sliders here - we'll use a global slider instead
    # (individual sliders removed to use global HTML slider)

    # Add 2% right-edge padding to prevent datapoints from being cut off
    depth_range = max_depth - abs_min_depth
    right_padding = int(depth_range * 0.02) or 10  # At least 10 units padding
    padded_max_depth = max_depth + right_padding

    # Update layout with buttons (no individual slider)
    fig.update_layout(
        title=dict(
            text=title,
            font=dict(size=20)
        ),
        xaxis=dict(
            title='Git Depth (commit count)',
            range=[min_depth, padded_max_depth],  # Set initial range with right padding
            gridcolor='lightgray',
            showgrid=True
        ),
        yaxis=dict(
            title=ylabel,
            type='log' if use_log else 'linear',  # Logarithmic scale only for positive values
            gridcolor='lightgray',
            showgrid=True
        ),
        hovermode='closest',
        legend=dict(
            orientation='v',
            yanchor='top',
            y=1,
            xanchor='left',
            x=1.05
        ),
        updatemenus=[
            dict(
                type='buttons',
                direction='left',
                buttons=[
                    dict(
                        label='Show All Lines',
                        method='update',
                        args=[{'visible': True}]
                    ),
                    dict(
                        label='Hide All Lines',
                        method='update',
                        args=[{'visible': 'legendonly'}]
                    )
                ],
                pad={'r': 10, 't': 10},
                showactive=False,
                x=0.0,
                xanchor='left',
                y=1.15,
                yanchor='top'
            )
        ],
        # No individual sliders - using global slider instead
        template='plotly_dark',  # Dark mode by default
        height=500,
        showlegend=True
    )

    print(f"  - {trace_count} traces, {len(regression_trace_indices)} regression overlays, log scale: {use_log}")

    # Return figure and metadata needed for global slider
    return fig, {'abs_min_depth': abs_min_depth, 'max_depth': max_depth, 'padded_max_depth': padded_max_depth}


def create_dashboard(df, output_file, filter_benchmark=None):
    """Generate complete interactive HTML dashboard."""

    if filter_benchmark:
        df = df[df['benchmark_name'] == filter_benchmark]
        if df.empty:
            print(f"Warning: No data for benchmark '{filter_benchmark}'", file=sys.stderr)
            return

    # Define metrics to plot (actions/sec is primary, bytes/turn is secondary)
    metrics = [
        ('actions_per_sec', 'Actions per Second', 'Throughput: Actions per Second'),
        ('bytes_per_turn', 'Bytes per Turn', 'Memory: Bytes per Turn'),
        ('games_per_sec', 'Games per Second', 'Throughput: Games per Second'),
        ('avg_duration_ms_per_game', 'Duration (ms)', 'Average Game Duration'),
        ('avg_bytes_per_game', 'Bytes', 'Memory: Average Bytes per Game'),
        ('actions_per_turn', 'Actions per Turn', 'Game Complexity: Actions per Turn'),
        ('avg_turns_per_game', 'Turns per Game', 'Game Length: Average Turns'),
    ]

    # Create HTML header
    html_parts = [
        '<!DOCTYPE html>',
        '<html>',
        '<head>',
        '    <meta charset="utf-8">',
        '    <title>MTG Forge-rs Performance Dashboard</title>',
        '    <script src="https://cdn.plot.ly/plotly-2.27.0.min.js"></script>',
        '    <style>',
        '        body {',
        '            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;',
        '            margin: 20px;',
        '            background-color: #f5f5f5;',
        '        }',
        '        .header {',
        '            background-color: white;',
        '            padding: 20px;',
        '            border-radius: 8px;',
        '            box-shadow: 0 2px 4px rgba(0,0,0,0.1);',
        '            margin-bottom: 20px;',
        '        }',
        '        h1 {',
        '            margin: 0 0 10px 0;',
        '            color: #333;',
        '        }',
        '        .subtitle {',
        '            color: #666;',
        '            font-size: 14px;',
        '        }',
        '        .plot-container {',
        '            background-color: white;',
        '            padding: 20px;',
        '            border-radius: 8px;',
        '            box-shadow: 0 2px 4px rgba(0,0,0,0.1);',
        '            margin-bottom: 20px;',
        '        }',
        '        .stats {',
        '            display: grid;',
        '            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));',
        '            gap: 15px;',
        '            margin-top: 20px;',
        '        }',
        '        .stat-card {',
        '            background-color: #f8f9fa;',
        '            padding: 15px;',
        '            border-radius: 6px;',
        '            border-left: 4px solid #007bff;',
        '        }',
        '        .stat-label {',
        '            font-size: 12px;',
        '            color: #666;',
        '            margin-bottom: 5px;',
        '        }',
        '        .stat-value {',
        '            font-size: 24px;',
        '            font-weight: bold;',
        '            color: #333;',
        '        }',
        '        .global-controls {',
        '            display: flex;',
        '            gap: 10px;',
        '            margin-top: 15px;',
        '            flex-wrap: wrap;',
        '        }',
        '        .control-btn {',
        '            padding: 10px 20px;',
        '            border: none;',
        '            border-radius: 6px;',
        '            cursor: pointer;',
        '            font-size: 14px;',
        '            font-weight: 500;',
        '            transition: all 0.3s;',
        '        }',
        '        .control-btn:hover {',
        '            transform: translateY(-1px);',
        '            box-shadow: 0 2px 8px rgba(0,0,0,0.2);',
        '        }',
        '        .theme-btn {',
        '            background-color: #007bff;',
        '            color: white;',
        '        }',
        '        .regression-btn {',
        '            background-color: #6c757d;',
        '            color: white;',
        '        }',
        '        body.dark-mode {',
        '            background-color: #1a1a1a;',
        '            color: #e0e0e0;',
        '        }',
        '        body.dark-mode .header {',
        '            background-color: #2d2d2d;',
        '            color: #e0e0e0;',
        '        }',
        '        body.dark-mode .subtitle {',
        '            color: #b0b0b0;',
        '        }',
        '        body.dark-mode .plot-container {',
        '            background-color: #2d2d2d;',
        '        }',
        '        body.dark-mode .stat-card {',
        '            background-color: #3d3d3d;',
        '            color: #e0e0e0;',
        '        }',
        '        body.dark-mode .stat-label {',
        '            color: #b0b0b0;',
        '        }',
        '        body.dark-mode .stat-value {',
        '            color: #e0e0e0;',
        '        }',
        '        .slider-container {',
        '            background-color: #e8f4f8;',
        '            border-left: 4px solid #007bff;',
        '        }',
        '        body.dark-mode .slider-container {',
        '            background-color: #1e3a4a;',
        '            border-left: 4px solid #4a9eff;',
        '        }',
        '        .filter-section {',
        '            display: grid;',
        '            grid-template-columns: 1fr 1fr;',
        '            gap: 20px;',
        '            margin-top: 20px;',
        '        }',
        '        .filter-column {',
        '            background-color: #f8f9fa;',
        '            padding: 15px;',
        '            border-radius: 6px;',
        '            border-left: 4px solid #28a745;',
        '        }',
        '        body.dark-mode .filter-column {',
        '            background-color: #3d3d3d;',
        '        }',
        '        .filter-column h4 {',
        '            margin-top: 0;',
        '            margin-bottom: 10px;',
        '        }',
        '        .checkbox-group {',
        '            display: flex;',
        '            flex-direction: column;',
        '            gap: 8px;',
        '            margin-top: 10px;',
        '            max-height: 300px;',
        '            overflow-y: auto;',
        '        }',
        '        .checkbox-label {',
        '            display: flex;',
        '            align-items: center;',
        '            cursor: pointer;',
        '            padding: 5px;',
        '            border-radius: 4px;',
        '            transition: background-color 0.2s;',
        '        }',
        '        .checkbox-label:hover {',
        '            background-color: rgba(0, 123, 255, 0.1);',
        '        }',
        '        .checkbox-label input {',
        '            margin-right: 8px;',
        '            cursor: pointer;',
        '            width: 18px;',
        '            height: 18px;',
        '        }',
        '        .filter-buttons {',
        '            display: flex;',
        '            gap: 10px;',
        '            margin-top: 10px;',
        '        }',
        '        .filter-btn {',
        '            padding: 8px 16px;',
        '            border: none;',
        '            border-radius: 4px;',
        '            cursor: pointer;',
        '            font-size: 12px;',
        '            font-weight: 500;',
        '            background-color: #28a745;',
        '            color: white;',
        '            transition: all 0.2s;',
        '        }',
        '        .filter-btn:hover {',
        '            background-color: #218838;',
        '            transform: translateY(-1px);',
        '        }',
        '        .filter-btn.deselect {',
        '            background-color: #dc3545;',
        '        }',
        '        .filter-btn.deselect:hover {',
        '            background-color: #c82333;',
        '        }',
        '        .warning-message {',
        '            background-color: #fff3cd;',
        '            border: 1px solid #ffc107;',
        '            border-radius: 6px;',
        '            padding: 12px;',
        '            margin-top: 15px;',
        '            color: #856404;',
        '            display: none;',
        '        }',
        '        body.dark-mode .warning-message {',
        '            background-color: #664d03;',
        '            border-color: #ffc107;',
        '            color: #ffecb5;',
        '        }',
        '    </style>',
        '</head>',
        '<body class="dark-mode">',  # Start in dark mode
        '    <div class="header">',
        '        <h1>🚀 MTG Forge-rs Performance Dashboard</h1>',
        f'        <div class="subtitle">Generated: {datetime.now().strftime("%Y-%m-%d %H:%M:%S UTC")}</div>',
    ]

    # Detect typical thread count for this machine from parallel benchmarks
    cpu_threads = "unknown"
    if 'num_threads' in df.columns:
        # Get the thread count from parallel benchmarks
        par_df = df[df['benchmark_name'].str.contains('par_', na=False)]
        if not par_df.empty:
            cpu_threads = str(int(par_df['num_threads'].mode().iloc[0]))

    # Add benchmark documentation
    benchmark_docs = {
        # Robots mirror matchup (baseline)
        'robots_mirror/fresh_games': 'Robots Mirror: Fresh Games - Play complete games from start to finish without rewind. (1 thread)',
        'robots_mirror/mem_logging_rewind_play_again': 'Robots Mirror: Memory Logging + Rewind - Tests undo system with realistic logging. (1 thread)',
        'robots_mirror/stdout_logging_rewind_play_again': 'Robots Mirror: Stdout Logging + Rewind - Tests worst-case logging overhead. (1 thread)',
        'robots_mirror/snapshot_games': 'Robots Mirror: Snapshot Games - Uses Clone-based snapshots instead of undo log. (1 thread)',
        'robots_mirror/rewind': 'Robots Mirror: Rewind Only - Pure rewind speed measurement. (1 thread)',
        'robots_mirror/rewind_play_again': 'Robots Mirror: Rewind + Play Again (Sequential) - Core rewind benchmark. (1 thread)',
        'robots_mirror/4x_par_rewind_play_again': 'Robots Mirror: Parallel (Rayon, 4 threads) - Tests multi-core scaling.',
        'robots_mirror/4x_pinned_par_rewind_play_again': 'Robots Mirror: Parallel (Pinned, 4 threads) - CPU affinity pinning for NUMA awareness.',
        'robots_mirror/32x_par_rewind_play_again': f'Robots Mirror: Parallel (Rayon, 32 threads) - High thread count scaling test.',
        'robots_mirror/32x_pinned_par_rewind_play_again': f'Robots Mirror: Parallel (Pinned, 32 threads) - High thread count with pinning.',

        # Other deck matchups
        'monoblack_thedeck/rewind_play_again': 'Mono Black vs The Deck: Rewind + Play Again - Control vs aggro matchup. (1 thread)',
        'whiteweenie_mirror/rewind_play_again': 'White Weenie Mirror: Rewind + Play Again - Creature aggro mirror. (1 thread)',
        'jeskai_trolldisk/rewind_play_again': 'Jeskai vs Troll Disk: Rewind + Play Again - Tempo vs control. (1 thread)',
        'simple_bolt/rewind_play_again': 'Simple Bolt Mirror: Rewind + Play Again - Minimal deck for fast iteration. (1 thread)',

        # Snapshot serialization
        'snapshot_serialization/save_to_file': 'Snapshot Serialization: Save to File - Tests bincode serialization speed. (1 thread)',
    }

    deck_info = [
        ('Robots Mirror', 'decks/old_school/03_robots_jesseisbak.dck', 'Artifact aggro mirror match - baseline benchmark'),
        ('Mono Black vs The Deck', 'decks/old_school/05_mono_black_rogerbrand.dck vs decks/old_school/02_thedeck_peterschnidrig.dck', 'Control vs aggro matchup'),
        ('White Weenie Mirror', 'decks/old_school2/white_weenie_classic.dck', 'Creature aggro mirror'),
        ('Jeskai Aggro vs Troll Disk', 'decks/old_school/06_jeskai_aggro_joseantonioprieto.dck vs decks/old_school/06_troll_disk_daniellebrunazzo.dck', 'Tempo vs control'),
        ('Simple Bolt Mirror', 'decks/simple_bolt.dck', 'Minimal deck for fast iteration'),
    ]

    # Add summary statistics
    latest = df.sort_values('git_depth').groupby('benchmark_name').last()
    total_entries = len(df)
    num_benchmarks = len(df['benchmark_name'].unique())
    commit_range = f"{df['git_depth'].min()} - {df['git_depth'].max()}"
    date_range = f"{df['timestamp'].min().strftime('%Y-%m-%d')} to {df['timestamp'].max().strftime('%Y-%m-%d')}"

    html_parts.extend([
        '        <div class="stats">',
        '            <div class="stat-card">',
        '                <div class="stat-label">Total Measurements</div>',
        f'                <div class="stat-value">{total_entries}</div>',
        '            </div>',
        '            <div class="stat-card">',
        '                <div class="stat-label">Benchmark Types</div>',
        f'                <div class="stat-value">{num_benchmarks}</div>',
        '            </div>',
        '            <div class="stat-card">',
        '                <div class="stat-label">Git Depth Range</div>',
        f'                <div class="stat-value">{commit_range}</div>',
        '            </div>',
        '            <div class="stat-card">',
        '                <div class="stat-label">Date Range</div>',
        f'                <div class="stat-value" style="font-size: 16px;">{date_range}</div>',
        '            </div>',
        '        </div>',
        '    </div>',
        '',
        '    <div class="header">',
        '        <h2 style="margin-top: 0;">📊 About This Dashboard</h2>',
        '        <p>',
        '        This dashboard tracks the performance of the MTG Forge-rs game engine over time. ',
        '        Each data point represents a benchmark run at a specific git commit depth.',
        '        </p>',
        '        <p>',
        '        <strong>Interactive Controls:</strong>',
        '        <ul>',
        '            <li><strong>🎯 Deck/Variant Filters:</strong> Checkboxes to filter which benchmark traces appear on plots. Filter by deck (left column) AND variant (right column). All checked by default. Use "Select All" / "Deselect All" buttons for quick control.</li>',
        '            <li><strong>🌙 Dark/Light Mode Toggle:</strong> Global button switches all plots between dark and light themes. Defaults to dark mode.</li>',
        '            <li><strong>Hide/Show Regressions:</strong> Global button toggles red X regression markers across all plots. Independent from line visibility.</li>',
        '            <li><strong>Global Git Depth Slider:</strong> Single slider controls all plots simultaneously. Default is 900 (recent commits). Slide left to see full history.</li>',
        '            <li><strong>Show/Hide All Lines:</strong> Per-plot buttons to quickly toggle visibility of all benchmark series</li>',
        '            <li><strong>Single click legend:</strong> Show/hide individual benchmarks (regression markers linked to their lines)</li>',
        '            <li><strong>Double click legend:</strong> Isolate a single benchmark (hides all others)</li>',
        '            <li><strong>Hover over points:</strong> See detailed information (deck, variant, commit, date, exact values)</li>',
        '            <li><strong>Toolbar:</strong> Zoom, pan, box select, reset axes, download plot as PNG</li>',
        '        </ul>',
        '        </p>',
        '        <p>',
        '        <strong>Key Metrics:</strong>',
        '        <ul>',
        '            <li><strong>Games/sec:</strong> Number of complete game simulations per second</li>',
        '            <li><strong>Actions/sec:</strong> Total game actions processed per second (higher = better throughput)</li>',
        '            <li><strong>Bytes/game:</strong> Memory allocated per game (lower = better efficiency)</li>',
        '            <li><strong>Actions/turn:</strong> Game complexity measure (varies by deck matchup)</li>',
        '        </ul>',
        '        </p>',
        '        <p>',
        '        <strong>Red X markers</strong> indicate immediate performance regressions >5% from the previous commit (not historical best).',
        '        This filters out noise from measurement variance while highlighting real commit-to-commit drops.',
        '        All Y-axes use logarithmic scale for better visibility across different benchmark magnitudes.',
        '        </p>',
        '    </div>',
        '',
        '    <div class="header">',
        '        <h2 style="margin-top: 0;">🎮 Benchmark Modes</h2>',
        '        <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(400px, 1fr)); gap: 15px;">',
    ])

    # Add benchmark documentation cards
    for bench_name, description in benchmark_docs.items():
        if bench_name in df['benchmark_name'].values:
            html_parts.extend([
                '            <div style="background-color: #f8f9fa; padding: 12px; border-radius: 6px; border-left: 4px solid #28a745;">',
                f'                <div style="font-weight: bold; margin-bottom: 5px;">{bench_name}</div>',
                f'                <div style="font-size: 14px; color: #555;">{description}</div>',
                '            </div>',
            ])

    html_parts.extend([
        '        </div>',
        '    </div>',
        '',
        '    <div class="header">',
        '        <h2 style="margin-top: 0;">🃏 Deck Configurations</h2>',
        '        <p>Different deck matchups test various aspects of the game engine:</p>',
        '        <table style="width: 100%; border-collapse: collapse; margin-top: 10px;">',
        '            <thead>',
        '                <tr style="background-color: #f8f9fa; text-align: left;">',
        '                    <th style="padding: 10px; border-bottom: 2px solid #dee2e6;">Matchup</th>',
        '                    <th style="padding: 10px; border-bottom: 2px solid #dee2e6;">Deck Path(s)</th>',
        '                    <th style="padding: 10px; border-bottom: 2px solid #dee2e6;">Description</th>',
        '                </tr>',
        '            </thead>',
        '            <tbody>',
    ])

    for name, path, desc in deck_info:
        html_parts.extend([
            '                <tr style="border-bottom: 1px solid #dee2e6;">',
            f'                    <td style="padding: 10px;"><strong>{name}</strong></td>',
            f'                    <td style="padding: 10px; font-family: monospace; font-size: 12px;">{path}</td>',
            f'                    <td style="padding: 10px;">{desc}</td>',
            '                </tr>',
        ])

    html_parts.extend([
        '            </tbody>',
        '        </table>',
        '    </div>',
    ])

    # Generate and embed each plot
    # Default to showing only recent data (git depth >= 900)
    default_min_depth = 900

    # Collect plot metadata for global slider
    plot_metadata = None
    plot_htmls = []

    for metric_col, ylabel, plot_title in metrics:
        if metric_col not in df.columns:
            print(f"Warning: Metric '{metric_col}' not found in data", file=sys.stderr)
            continue

        print(f"Generating plot: {plot_title}")
        fig, metadata = create_metric_plot(df, metric_col, ylabel, plot_title, min_depth=default_min_depth)

        # Store metadata from first plot (all plots have same depth range)
        if plot_metadata is None:
            plot_metadata = metadata

        # Convert to HTML div
        plot_html = fig.to_html(
            include_plotlyjs=False,
            div_id=f"plot_{metric_col}",
            config={'responsive': True},
            include_mathjax=False
        )

        plot_htmls.append((metric_col, plot_html))

    # Extract decks and variants for filter UI
    decks, variants = extract_decks_variants(df)

    # Add deck/variant filter controls before plots
    html_parts.extend([
        '    <div class="plot-container">',
        '        <h3 style="margin-top: 0;">🎯 Deck/Variant Filters</h3>',
        '        <p style="margin-bottom: 15px;">',
        '            Filter which benchmark traces appear on the plots. All traces are shown by default.',
        '        </p>',
        '        <div class="filter-section">',
        '            <!-- Deck filters -->',
        '            <div class="filter-column">',
        '                <h4>📦 Decks</h4>',
        '                <div class="filter-buttons">',
        '                    <button id="selectAllDecks" class="filter-btn">Select All</button>',
        '                    <button id="deselectAllDecks" class="filter-btn deselect">Deselect All</button>',
        '                </div>',
        '                <div class="checkbox-group" id="deckCheckboxes">',
    ])

    # Add deck checkboxes
    for deck in decks:
        deck_id = deck.replace('_', '-').replace('/', '-')
        html_parts.extend([
            f'                    <label class="checkbox-label">',
            f'                        <input type="checkbox" class="deck-checkbox" data-deck="{deck}" checked>',
            f'                        <span>{deck}</span>',
            f'                    </label>',
        ])

    html_parts.extend([
        '                </div>',
        '            </div>',
        '            <!-- Variant filters -->',
        '            <div class="filter-column">',
        '                <h4>🔧 Variants</h4>',
        '                <div class="filter-buttons">',
        '                    <button id="selectAllVariants" class="filter-btn">Select All</button>',
        '                    <button id="deselectAllVariants" class="filter-btn deselect">Deselect All</button>',
        '                </div>',
        '                <div class="checkbox-group" id="variantCheckboxes">',
    ])

    # Add variant checkboxes
    for variant in variants:
        variant_id = variant.replace('_', '-').replace('/', '-').replace('(', '').replace(')', '')
        html_parts.extend([
            f'                    <label class="checkbox-label">',
            f'                        <input type="checkbox" class="variant-checkbox" data-variant="{variant}" checked>',
            f'                        <span>{variant}</span>',
            f'                    </label>',
        ])

    html_parts.extend([
        '                </div>',
        '            </div>',
        '        </div>',
        '        <div id="filterWarning" class="warning-message">',
        '            ⚠️ Warning: No decks or variants selected. All traces are hidden.',
        '        </div>',
        '    </div>',
        '',
    ])

    # Add global slider control before plots
    if plot_metadata:
        abs_min = plot_metadata['abs_min_depth']
        max_depth = plot_metadata['max_depth']
        padded_max = plot_metadata['padded_max_depth']

        html_parts.extend([
            '    <div class="plot-container slider-container">',
            '        <h3 style="margin-top: 0;">⚙️ Global Controls</h3>',
            '        ',
            '        <!-- Theme and Regression Controls -->',
            '        <div class="global-controls" style="margin-bottom: 20px;">',
            '            <button id="themeToggle" class="control-btn theme-btn">☀️ Light Mode</button>',
            '            <button id="regressionToggle" class="control-btn regression-btn">Hide Regressions</button>',
            '        </div>',
            '        ',
            '        <!-- Git Depth Slider -->',
            '        <h4 style="margin-top: 20px; margin-bottom: 10px;">🎚️ Git Depth Filter</h4>',
            '        <p style="margin-bottom: 15px;">',
            '            Adjust the slider to filter all plots by minimum git depth. ',
            '            Default is 900 (recent commits). Slide left to see full history.',
            '        </p>',
            f'        <label for="gitDepthSlider" style="display: block; margin-bottom: 5px; font-weight: bold;">',
            f'            Min Git Depth: <span id="gitDepthValue">{default_min_depth}</span>',
            '        </label>',
            f'        <input type="range" id="gitDepthSlider" min="{abs_min}" max="{max_depth}" value="{default_min_depth}" step="10" ',
            '               style="width: 100%; height: 30px; cursor: pointer;">',
            '        <div style="display: flex; justify-content: space-between; font-size: 12px; color: #666; margin-top: 5px;">',
            f'            <span>Full History ({abs_min})</span>',
            f'            <span>Latest ({max_depth})</span>',
            '        </div>',
            '    </div>',
            '',
            '    <script>',
            '    // Global slider control for all plots',
            '    (function() {',
            '        const slider = document.getElementById("gitDepthSlider");',
            '        const valueDisplay = document.getElementById("gitDepthValue");',
            f'        const paddedMax = {padded_max};',
            '        ',
            '        // Get all plot divs',
            f'        const plotIds = {[f"plot_{m[0]}" for m in metrics]};',
            '        ',
            '        slider.addEventListener("input", function() {',
            '            const minDepth = parseInt(this.value);',
            '            valueDisplay.textContent = minDepth;',
            '            ',
            '            // Update all plots',
            '            plotIds.forEach(plotId => {',
            '                const plotDiv = document.getElementById(plotId);',
            '                if (plotDiv && plotDiv.layout) {',
            '                    Plotly.relayout(plotId, {',
            '                        "xaxis.range": [minDepth, paddedMax]',
            '                    });',
            '                }',
            '            });',
            '        });',
            '    })();',
            '    </script>',
            '',
        ])

    # Add all the plot HTMLs
    for metric_col, plot_html in plot_htmls:
        html_parts.extend([
            '    <div class="plot-container">',
            plot_html,
            '    </div>',
        ])

    # Add global control JavaScript
    html_parts.extend([
        '    <script>',
        '    // Theme toggle',
        '    (function() {',
        '        const themeBtn = document.getElementById("themeToggle");',
        f'        const plotIds = {[f"plot_{m[0]}" for m in metrics]};',
        '        let isDark = true;  // Start in dark mode',
        '        ',
        '        themeBtn.addEventListener("click", function() {',
        '            isDark = !isDark;',
        '            document.body.classList.toggle("dark-mode");',
        '            const newTemplate = isDark ? "plotly_dark" : "plotly_white";',
        '            themeBtn.textContent = isDark ? "☀️ Light Mode" : "🌙 Dark Mode";',
        '            ',
        '            // Update all plots',
        '            plotIds.forEach(plotId => {',
        '                const plotDiv = document.getElementById(plotId);',
        '                if (plotDiv && plotDiv.layout) {',
        '                    Plotly.relayout(plotId, {',
        '                        template: newTemplate',
        '                    });',
        '                }',
        '            });',
        '        });',
        '    })();',
        '    ',
        '    // Regression toggle - only affects regression markers (showlegend=false traces)',
        '    (function() {',
        '        const regressionBtn = document.getElementById("regressionToggle");',
        f'        const plotIds = {[f"plot_{m[0]}" for m in metrics]};',
        '        let regressionsVisible = true;',
        '        ',
        '        regressionBtn.addEventListener("click", function() {',
        '            regressionsVisible = !regressionsVisible;',
        '            regressionBtn.textContent = regressionsVisible ? "Hide Regressions" : "Show Regressions";',
        '            ',
        '            // Update all plots - only toggle traces with showlegend=false',
        '            plotIds.forEach(plotId => {',
        '                const plotDiv = document.getElementById(plotId);',
        '                if (plotDiv && plotDiv.data) {',
        '                    // Build array of visibility for each trace',
        '                    const visibilityArray = plotDiv.data.map((trace, idx) => {',
        '                        // Only affect regression markers (showlegend=false)',
        '                        if (trace.showlegend === false) {',
        '                            return regressionsVisible;',
        '                        }',
        '                        // Keep other traces as they are',
        '                        return trace.visible === "legendonly" ? "legendonly" : true;',
        '                    });',
        '                    ',
        '                    Plotly.restyle(plotId, {visible: visibilityArray});',
        '                }',
        '            });',
        '        });',
        '    })();',
        '    ',
        '    // Deck/Variant filter control',
        '    (function() {',
        f'        const plotIds = {[f"plot_{m[0]}" for m in metrics]};',
        '        ',
        '        // Get all checkboxes',
        '        const deckCheckboxes = document.querySelectorAll(".deck-checkbox");',
        '        const variantCheckboxes = document.querySelectorAll(".variant-checkbox");',
        '        const warningDiv = document.getElementById("filterWarning");',
        '        ',
        '        // Select/Deselect All buttons',
        '        document.getElementById("selectAllDecks").addEventListener("click", function() {',
        '            deckCheckboxes.forEach(cb => cb.checked = true);',
        '            applyFilters();',
        '        });',
        '        ',
        '        document.getElementById("deselectAllDecks").addEventListener("click", function() {',
        '            deckCheckboxes.forEach(cb => cb.checked = false);',
        '            applyFilters();',
        '        });',
        '        ',
        '        document.getElementById("selectAllVariants").addEventListener("click", function() {',
        '            variantCheckboxes.forEach(cb => cb.checked = true);',
        '            applyFilters();',
        '        });',
        '        ',
        '        document.getElementById("deselectAllVariants").addEventListener("click", function() {',
        '            variantCheckboxes.forEach(cb => cb.checked = false);',
        '            applyFilters();',
        '        });',
        '        ',
        '        // Apply filters when checkboxes change',
        '        deckCheckboxes.forEach(cb => {',
        '            cb.addEventListener("change", applyFilters);',
        '        });',
        '        ',
        '        variantCheckboxes.forEach(cb => {',
        '            cb.addEventListener("change", applyFilters);',
        '        });',
        '        ',
        '        function applyFilters() {',
        '            // Get selected decks and variants',
        '            const selectedDecks = new Set();',
        '            deckCheckboxes.forEach(cb => {',
        '                if (cb.checked) {',
        '                    selectedDecks.add(cb.dataset.deck);',
        '                }',
        '            });',
        '            ',
        '            const selectedVariants = new Set();',
        '            variantCheckboxes.forEach(cb => {',
        '                if (cb.checked) {',
        '                    selectedVariants.add(cb.dataset.variant);',
        '                }',
        '            });',
        '            ',
        '            // Show warning if no decks or variants selected',
        '            if (selectedDecks.size === 0 || selectedVariants.size === 0) {',
        '                warningDiv.style.display = "block";',
        '            } else {',
        '                warningDiv.style.display = "none";',
        '            }',
        '            ',
        '            // Update all plots',
        '            plotIds.forEach(plotId => {',
        '                const plotDiv = document.getElementById(plotId);',
        '                if (plotDiv && plotDiv.data) {',
        '                    // Build visibility array for each trace',
        '                    const visibilityArray = plotDiv.data.map((trace, idx) => {',
        '                        // Extract deck and variant from customdata',
        '                        if (trace.customdata && trace.customdata.length > 0) {',
        '                            const deck = trace.customdata[0][0];',
        '                            const variant = trace.customdata[0][1];',
        '                            ',
        '                            // Check if both deck and variant match',
        '                            const deckMatch = selectedDecks.has(deck);',
        '                            const variantMatch = selectedVariants.has(variant);',
        '                            ',
        '                            if (deckMatch && variantMatch) {',
        '                                // Check if trace was set to legendonly by user',
        '                                return trace.visible === "legendonly" ? "legendonly" : true;',
        '                            } else {',
        '                                return false;',
        '                            }',
        '                        }',
        '                        ',
        '                        // Keep trace as-is if no customdata (shouldn\'t happen)',
        '                        return trace.visible === "legendonly" ? "legendonly" : true;',
        '                    });',
        '                    ',
        '                    Plotly.restyle(plotId, {visible: visibilityArray});',
        '                }',
        '            });',
        '        }',
        '    })();',
        '    </script>',
    ])

    # Close HTML
    html_parts.extend([
        '</body>',
        '</html>'
    ])

    # Write to file
    output_path = Path(output_file)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with open(output_path, 'w') as f:
        f.write('\n'.join(html_parts))

    print(f"\n✓ Dashboard saved to: {output_path}")
    print(f"  File size: {output_path.stat().st_size / 1024:.1f} KB")
    print(f"  Open in browser: file://{output_path.absolute()}")


def main():
    args = parse_args()

    # Load data
    print(f"Loading performance history from: {args.input}")
    df = load_data(args.input)

    print(f"Found {len(df)} entries across {len(df['benchmark_name'].unique())} benchmark types")

    # Generate dashboard
    print(f"\nGenerating interactive dashboard...")
    create_dashboard(df, args.output, filter_benchmark=args.benchmark)


if __name__ == '__main__':
    main()
