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


def create_metric_plot(df, metric_col, ylabel, title):
    """
    Create an interactive Plotly figure for a single metric.

    Returns a Plotly figure object.
    """
    fig = go.Figure()

    benchmarks = sorted(df['benchmark_name'].unique())
    trace_count = 0
    regression_trace_indices = []  # Track which traces are regressions

    for benchmark in benchmarks:
        bench_df = df[df['benchmark_name'] == benchmark].copy()

        if bench_df.empty or metric_col not in bench_df.columns:
            continue

        # Skip if all values are NaN
        if bench_df[metric_col].isna().all():
            continue

        # Add main line trace
        trace_count += 1
        hover_text = []
        for _, row in bench_df.iterrows():
            hover_text.append(
                f"<b>{benchmark}</b><br>"
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
            legendgroup=benchmark  # Group with regression markers
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
                legendgroup=benchmark  # Link to parent benchmark trace
            ))

    # Determine if log scale is appropriate (need positive values)
    all_values = df[metric_col].dropna()
    use_log = len(all_values) > 0 and (all_values > 0).all()

    # Create visibility arrays for toggling regressions
    total_traces = len(fig.data)
    show_regressions = [True] * total_traces
    hide_regressions = [True] * total_traces
    for idx in regression_trace_indices:
        hide_regressions[idx] = False

    # Update layout with buttons to show/hide all traces and toggle regressions
    fig.update_layout(
        title=dict(
            text=title,
            font=dict(size=20)
        ),
        xaxis=dict(
            title='Git Depth (commit count)',
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
                        label='Show All',
                        method='update',
                        args=[{'visible': True}]
                    ),
                    dict(
                        label='Hide All',
                        method='update',
                        args=[{'visible': 'legendonly'}]
                    ),
                    dict(
                        label='Hide Regressions',
                        method='update',
                        args=[{'visible': hide_regressions}]
                    ),
                    dict(
                        label='Show Regressions',
                        method='update',
                        args=[{'visible': show_regressions}]
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
        template='plotly_white',
        height=500,
        showlegend=True
    )

    print(f"  - {trace_count} traces, {len(regression_trace_indices)} regression overlays, log scale: {use_log}")

    return fig


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
        '    </style>',
        '</head>',
        '<body>',
        '    <div class="header">',
        '        <h1>🚀 MTG Forge-rs Performance Dashboard</h1>',
        f'        <div class="subtitle">Generated: {datetime.now().strftime("%Y-%m-%d %H:%M:%S UTC")}</div>',
    ]

    # Add benchmark documentation
    benchmark_docs = {
        'fresh_games': 'Fresh Games (Baseline) - Play complete games from start to finish without rewind. No logging overhead. Tests core game engine throughput.',
        'mem_logging_rewind_play_again': 'Memory Logging + Rewind - Play to midpoint, rewind, play forward. Logs stored in memory. Tests undo system with realistic logging.',
        'stdout_logging_rewind_play_again': 'Stdout Logging + Rewind - Same as memory logging but writes to stdout. Tests worst-case logging overhead.',
        'snapshot_games': 'Snapshot Games - Like fresh games but uses Clone-based snapshots instead of undo log. Tests memory cloning overhead.',
        'rewind': 'Rewind Only - Measures pure rewind speed by rewinding a completed game repeatedly. Tests undo system in isolation.',
        'rewind_play_again': 'Rewind + Play Again (Sequential) - Rewind to midpoint then play forward. Sequential execution. Core rewind benchmark.',
        'par_rewind_play_again': 'Rewind + Play Again (Parallel) - Same as sequential but uses Rayon parallel iterators. Tests multi-core scaling.',
        'pinned_par_rewind_play_again': 'Rewind + Play Again (Pinned-Parallel) - Parallel execution with CPU affinity pinning. Tests NUMA-aware performance.',
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
        '            <li><strong>Show/Hide All:</strong> Quickly toggle visibility of all benchmark series</li>',
        '            <li><strong>Hide/Show Regressions:</strong> Toggle regression markers on/off (red X markers)</li>',
        '            <li><strong>Single click legend:</strong> Show/hide individual benchmarks (regression markers follow their lines)</li>',
        '            <li><strong>Double click legend:</strong> Isolate a single benchmark (hides all others)</li>',
        '            <li><strong>Hover over points:</strong> See detailed information (commit, date, exact values)</li>',
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
    for metric_col, ylabel, plot_title in metrics:
        if metric_col not in df.columns:
            print(f"Warning: Metric '{metric_col}' not found in data", file=sys.stderr)
            continue

        print(f"Generating plot: {plot_title}")
        fig = create_metric_plot(df, metric_col, ylabel, plot_title)

        # Convert to HTML div
        plot_html = fig.to_html(
            include_plotlyjs=False,
            div_id=f"plot_{metric_col}",
            config={'responsive': True},
            include_mathjax=False
        )

        html_parts.extend([
            '    <div class="plot-container">',
            plot_html,
            '    </div>',
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
