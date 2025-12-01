#!/usr/bin/env python3
"""
Backfill benchmark results for historical commits - simplified version.

Key insight: Keep CSV completely outside git tree during backfill process.
This avoids all git conflicts and makes restoration trivial.
"""

import argparse
import subprocess
import sys
from pathlib import Path
from dataclasses import dataclass
from typing import List, Optional


@dataclass
class CommitInfo:
    hash: str
    short_hash: str
    depth: int
    message: str


PATCH_INTRODUCED_DEPTH = 823

BENCHMARK_PATCH = '''--- a/mtg-benchmarks/benches/game_benchmark.rs
+++ b/mtg-benchmarks/benches/game_benchmark.rs
@@ -150,7 +150,9 @@ fn bench_rewind_play_again<B, C, F, P>(
         let total_games = bench.total_games();
         if total_games > 0 {
             let aggregated_metrics = bench.get_metrics();
-            print_aggregated_metrics(bench_name, bench.orig_seed(), &aggregated_metrics, total_games);
+            // Use full group/bench name to distinguish different deck matchups
+            let full_bench_name = format!("{}/{}", group_name, bench_name);
+            print_aggregated_metrics(&full_bench_name, bench.orig_seed(), &aggregated_metrics, total_games);
             print_fn(bench);
         }
     }
'''

UPDATED_GITDEPTH = '''#!/bin/bash
# Count commits in first-parent (main branch) history only
git rev-list --count --first-parent HEAD
'''


def run_cmd(cmd: List[str], **kwargs):
    """Run command, return result."""
    return subprocess.run(cmd, capture_output=True, text=True, **kwargs)


def get_cpu_name() -> str:
    """Get CPU name."""
    result = run_cmd(['lscpu'])
    for line in result.stdout.splitlines():
        if line.startswith('Model name:'):
            return line.split(':', 1)[1].strip().replace(' ', '_')
    raise RuntimeError("Could not determine CPU name")


def get_first_parent_depth(commit='HEAD') -> int:
    """Get first-parent depth."""
    result = run_cmd(['git', 'rev-list', '--count', '--first-parent', commit])
    return int(result.stdout.strip())


def get_commit_at_depth(depth: int) -> Optional[CommitInfo]:
    """Get commit at specific first-parent depth."""
    result = run_cmd(['git', 'rev-list', '--first-parent', '--reverse', 'HEAD'])
    commits = result.stdout.strip().split('\n')
    if depth < 1 or depth > len(commits):
        return None
    commit_hash = commits[depth - 1]
    result = run_cmd(['git', 'log', '-1', '--oneline', commit_hash])
    return CommitInfo(
        hash=commit_hash,
        short_hash=commit_hash[:8],
        depth=depth,
        message=result.stdout.strip()
    )


def is_docs_only(commit_hash: str) -> bool:
    """Check if commit is docs-only."""
    result = run_cmd(['git', 'diff-tree', '--no-commit-id', '--name-only', '-r', commit_hash])
    files = [f for f in result.stdout.strip().split('\n') if f]
    if not files:
        return True
    docs_patterns = ['.beads/', 'docs/', '.md', 'experiment_results/', 'scripts/plot_performance', '.gitignore']
    return all(any(p in f for p in docs_patterns) for f in files)


def main():
    parser = argparse.ArgumentParser(description='Backfill benchmark results')
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--depth-range', nargs=2, type=int)
    group.add_argument('--last-n', type=int)
    parser.add_argument('--cadence', type=int, default=1)
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()

    # Determine range
    current_depth = get_first_parent_depth('HEAD')
    if args.depth_range:
        start_depth, end_depth = args.depth_range
    else:
        start_depth = max(1, current_depth - args.last_n + 1)
        end_depth = current_depth

    # Setup paths
    cpu_name = get_cpu_name()
    csv_in_tree = Path(f'experiment_results/{cpu_name}/perf_history.csv')
    csv_external = Path('/tmp/backfill_perf_history.csv')
    patch_file = Path('/tmp/benchmark_naming_fix.patch')

    print("=== Backfilling Benchmarks ===")
    print(f"CPU: {cpu_name}")
    print(f"Depth range: {start_depth} to {end_depth}")
    print(f"Cadence: every {args.cadence}")
    print(f"Patch introduced at: depth {PATCH_INTRODUCED_DEPTH}")
    print()

    # Analyze commits
    print("=== Analyzing commits ===")
    commits_to_bench: List[CommitInfo] = []
    for depth in range(start_depth, end_depth + 1, args.cadence):
        commit = get_commit_at_depth(depth)
        if not commit:
            continue
        if is_docs_only(commit.hash):
            print(f"Skip depth {depth} (docs-only)")
            continue
        commits_to_bench.append(commit)
        print(f"Need depth {depth}: {commit.message[:60]}")

    print(f"\nCommits to benchmark: {len(commits_to_bench)}")

    if args.dry_run:
        print("\n=== Dry run complete ===")
        return 0

    # Save state
    original_branch = run_cmd(['git', 'branch', '--show-current']).stdout.strip() or None
    original_head = run_cmd(['git', 'rev-parse', 'HEAD']).stdout.strip()

    # Move CSV out of git tree completely
    import shutil
    if csv_in_tree.exists():
        shutil.copy(csv_in_tree, csv_external)
        print(f"\nCopied CSV to external location: {csv_external}")
        # Reset CSV in tree to clean state for git operations
        run_cmd(['git', 'checkout', 'HEAD', '--', str(csv_in_tree)], check=False)
    else:
        csv_external.write_text('timestamp,git_commit,git_depth,git_branch,git_dirty,benchmark_name,seed,num_threads,num_games,total_turns,total_actions,total_duration_ms,avg_turns_per_game,avg_actions_per_game,avg_duration_ms_per_game,games_per_sec,actions_per_sec,turns_per_sec,actions_per_turn,total_bytes_allocated,total_bytes_deallocated,net_bytes,avg_bytes_per_game,bytes_per_turn,bytes_per_sec\n')

    # Write files
    patch_file.write_text(BENCHMARK_PATCH)

    try:
        # Benchmark each commit
        for i, commit in enumerate(commits_to_bench, 1):
            print(f"\n[{i}/{len(commits_to_bench)}] Benchmarking depth {commit.depth}")

            # Checkout
            result = run_cmd(['git', 'checkout', commit.hash], check=False)
            if result.returncode != 0:
                print(f"✗ Checkout failed")
                continue

            # Verify depth
            actual = get_first_parent_depth('HEAD')
            if actual != commit.depth:
                print(f"✗ Depth mismatch: expected {commit.depth}, got {actual}")
                continue

            # Install updated gitdepth.sh
            Path('scripts/gitdepth.sh').write_text(UPDATED_GITDEPTH)
            Path('scripts/gitdepth.sh').chmod(0o755)

            # Apply patch if needed
            if commit.depth < PATCH_INTRODUCED_DEPTH:
                run_cmd(['patch', '-p1'], input=BENCHMARK_PATCH, check=False)

            # Copy external CSV into tree so benchmarks can append to it
            shutil.copy(csv_external, csv_in_tree)

            # Run benchmarks
            run_cmd(['cargo', 'clean'], check=False)
            result = run_cmd(['./scripts/run_benchmark.sh'], check=False)

            # Copy updated CSV back to external location
            shutil.copy(csv_in_tree, csv_external)

            if result.returncode == 0:
                print(f"✓ Benchmarked depth {commit.depth}")
            else:
                print(f"✗ Benchmark failed")

            # Cleanup - restore files to original state
            if commit.depth < PATCH_INTRODUCED_DEPTH:
                run_cmd(['git', 'checkout', 'mtg-benchmarks/benches/game_benchmark.rs'], check=False)
            run_cmd(['git', 'checkout', 'scripts/gitdepth.sh'], check=False)
            run_cmd(['git', 'checkout', str(csv_in_tree)], check=False)  # Restore CSV to clean state

    finally:
        # Restore git state
        print("\n=== Restoring original state ===")

        # Checkout original (CSV should already be clean from last iteration)
        if original_branch:
            run_cmd(['git', 'checkout', original_branch], check=False)
        else:
            run_cmd(['git', 'checkout', original_head], check=False)

        # Copy final CSV back into tree
        if csv_external.exists():
            shutil.copy(csv_external, csv_in_tree)
            print(f"✓ Restored CSV to git tree")

        # Cleanup temp files
        patch_file.unlink(missing_ok=True)
        csv_external.unlink(missing_ok=True)

    lines = len(open(csv_in_tree).readlines())
    print(f"\n✓ Complete! CSV has {lines} lines")
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\nInterrupted")
        sys.exit(130)
