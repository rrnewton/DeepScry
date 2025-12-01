#!/usr/bin/env python3
"""
Backfill benchmark results for historical commits.

Strategy:
- Use inclusive commit counting (all commits, not just first-parent)
- Build map of all commits with their depths
- Only benchmark commits on main branch (first-parent history)
- Keep CSV outside git tree during backfill to avoid conflicts
"""

import argparse
import subprocess
import sys
import logging
from datetime import datetime
from pathlib import Path
from dataclasses import dataclass
from typing import List, Optional, Set, Dict


@dataclass
class CommitInfo:
    hash: str
    short_hash: str
    depth: int
    message: str
    on_main_branch: bool


# Commit where benchmark naming patch was introduced (72d86871)
# Before this commit, we need to apply the patch
PATCH_COMMIT_HASH = '72d86871'

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
# Count total commits (inclusive of all history)
# Use --main-only flag to count only first-parent (main branch) commits

if [ "$1" = "--main-only" ]; then
    # Count only main-branch commits (linear history)
    git rev-list --count --first-parent HEAD
else
    # Count all commits (default)
    git rev-list --count HEAD
fi
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


def get_all_commits() -> List[str]:
    """Get all commits in chronological order."""
    result = run_cmd(['git', 'rev-list', '--reverse', 'HEAD'])
    return result.stdout.strip().split('\n')


def get_main_branch_commits() -> Set[str]:
    """Get set of commits on main branch (first-parent history)."""
    result = run_cmd(['git', 'rev-list', '--first-parent', 'HEAD'])
    return set(result.stdout.strip().split('\n'))


def build_commit_map() -> Dict[int, CommitInfo]:
    """Build map of depth -> CommitInfo for all commits."""
    all_commits = get_all_commits()
    main_commits = get_main_branch_commits()

    commit_map = {}
    for depth, commit_hash in enumerate(all_commits, start=1):
        result = run_cmd(['git', 'log', '-1', '--oneline', commit_hash])
        message = result.stdout.strip()

        commit_map[depth] = CommitInfo(
            hash=commit_hash,
            short_hash=commit_hash[:8],
            depth=depth,
            message=message,
            on_main_branch=(commit_hash in main_commits)
        )

    return commit_map


def is_docs_only(commit_hash: str) -> bool:
    """Check if commit is docs-only."""
    result = run_cmd(['git', 'diff-tree', '--no-commit-id', '--name-only', '-r', commit_hash])
    files = [f for f in result.stdout.strip().split('\n') if f]
    if not files:
        return True
    docs_patterns = ['.beads/', 'docs/', '.md', 'experiment_results/', 'scripts/plot_performance', '.gitignore']
    return all(any(p in f for p in docs_patterns) for f in files)


def get_benchmarked_commits(csv_file: Path) -> Set[str]:
    """Get set of commit hashes that already have benchmark results."""
    if not csv_file.exists():
        return set()

    benchmarked = set()
    with open(csv_file, 'r') as f:
        lines = f.readlines()
        # Skip header
        for line in lines[1:]:
            if line.strip():
                parts = line.split(',')
                if len(parts) > 1:
                    commit_hash = parts[1]  # git_commit is 2nd column
                    benchmarked.add(commit_hash)
    return benchmarked


def is_ancestor_of(commit1: str, commit2: str) -> bool:
    """Check if commit1 is an ancestor of commit2."""
    result = run_cmd(['git', 'merge-base', '--is-ancestor', commit1, commit2], check=False)
    return result.returncode == 0


def needs_patch(commit_hash: str) -> bool:
    """Check if commit needs the benchmark naming patch applied."""
    # If PATCH_COMMIT_HASH is ancestor of commit_hash, then commit_hash already has the patch
    if is_ancestor_of(PATCH_COMMIT_HASH, commit_hash):
        return False
    # Otherwise, commit is before the patch was introduced
    return True


def main():
    parser = argparse.ArgumentParser(description='Backfill benchmark results')
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--depth-range', nargs=2, type=int)
    group.add_argument('--last-n', type=int)
    parser.add_argument('--cadence', type=int, default=1)
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()

    # Setup logging
    log_file = Path('backfill_benchmarks.log')
    logging.basicConfig(
        level=logging.INFO,
        format='%(asctime)s - %(levelname)s - %(message)s',
        handlers=[
            logging.FileHandler(log_file),
            logging.StreamHandler(sys.stdout)
        ]
    )

    # Build commit map
    logging.info("=== Building commit map ===")
    commit_map = build_commit_map()
    total_commits = len(commit_map)
    main_commits = sum(1 for c in commit_map.values() if c.on_main_branch)

    logging.info(f"Total commits: {total_commits}")
    logging.info(f"Main branch commits: {main_commits}")
    logging.info(f"Non-main commits: {total_commits - main_commits}")

    # Determine range
    if args.depth_range:
        start_depth, end_depth = args.depth_range
    else:
        start_depth = max(1, total_commits - args.last_n + 1)
        end_depth = total_commits

    # Setup paths
    cpu_name = get_cpu_name()
    csv_in_tree = Path(f'experiment_results/{cpu_name}/perf_history.csv')
    csv_backup = csv_in_tree.with_suffix('.csv.backfill_backup')
    patch_file = Path('/tmp/benchmark_naming_fix.patch')

    logging.info("")
    logging.info("=== Backfilling Benchmarks ===")
    logging.info(f"CPU: {cpu_name}")
    logging.info(f"Depth range: {start_depth} to {end_depth}")
    logging.info(f"Cadence: every {args.cadence}")
    logging.info(f"Patch commit: {PATCH_COMMIT_HASH}")
    logging.info(f"Log file: {log_file.absolute()}")
    logging.info(f"CSV backup location: {csv_backup.absolute()}")
    logging.info("")

    # Get already-benchmarked commits for idempotency
    benchmarked_commits = get_benchmarked_commits(csv_backup if csv_backup.exists() else csv_in_tree)
    logging.info(f"Already benchmarked commits: {len(benchmarked_commits)}")

    # Analyze commits
    logging.info("")
    logging.info("=== Analyzing commits ===")
    commits_to_bench: List[CommitInfo] = []
    for depth in range(start_depth, end_depth + 1, args.cadence):
        if depth not in commit_map:
            continue

        commit = commit_map[depth]

        # Skip non-main-branch commits
        if not commit.on_main_branch:
            logging.info(f"Skip depth {depth} (not on main branch)")
            continue

        # Skip already-benchmarked commits (idempotency)
        if commit.short_hash in benchmarked_commits:
            logging.info(f"Skip depth {depth} (already benchmarked)")
            continue

        # Skip docs-only commits
        if is_docs_only(commit.hash):
            logging.info(f"Skip depth {depth} (docs-only)")
            continue

        commits_to_bench.append(commit)
        logging.info(f"Need depth {depth}: {commit.message[:60]}")

    logging.info(f"\nCommits to benchmark: {len(commits_to_bench)}")

    if args.dry_run:
        logging.info("\n=== Dry run complete ===")
        return 0

    # Save state
    original_branch = run_cmd(['git', 'branch', '--show-current']).stdout.strip() or None
    original_head = run_cmd(['git', 'rev-parse', 'HEAD']).stdout.strip()
    logging.info(f"Original branch: {original_branch or 'detached'}")
    logging.info(f"Original HEAD: {original_head}")

    # Move CSV to persistent backup location
    import shutil
    if csv_in_tree.exists():
        shutil.copy(csv_in_tree, csv_backup)
        logging.info(f"\nCopied CSV to persistent backup: {csv_backup}")
        logging.info(f"IMPORTANT: If script fails, CSV data can be recovered from: {csv_backup}")
        # Reset CSV in tree to clean state for git operations
        run_cmd(['git', 'checkout', 'HEAD', '--', str(csv_in_tree)], check=False)
    else:
        csv_backup.write_text('timestamp,git_commit,git_depth,git_branch,git_dirty,benchmark_name,seed,num_games,total_turns,total_actions,total_duration_ms,avg_turns_per_game,avg_actions_per_game,avg_duration_ms_per_game,games_per_sec,actions_per_sec,turns_per_sec,actions_per_turn,total_bytes_allocated,total_bytes_deallocated,net_bytes,avg_bytes_per_game,bytes_per_turn,bytes_per_sec\n')
        logging.info(f"Created new CSV backup: {csv_backup}")

    # Write files
    patch_file.write_text(BENCHMARK_PATCH)

    try:
        # Benchmark each commit
        for i, commit in enumerate(commits_to_bench, 1):
            logging.info(f"\n[{i}/{len(commits_to_bench)}] Benchmarking depth {commit.depth} ({commit.short_hash})")

            # Checkout
            result = run_cmd(['git', 'checkout', commit.hash], check=False)
            if result.returncode != 0:
                logging.error(f"✗ Checkout failed for {commit.short_hash}")
                continue

            # Verify depth using inclusive counting
            result = run_cmd(['git', 'rev-list', '--count', 'HEAD'])
            actual_depth = int(result.stdout.strip())
            if actual_depth != commit.depth:
                logging.error(f"✗ Depth mismatch: expected {commit.depth}, got {actual_depth}")
                continue

            # Install updated gitdepth.sh
            Path('scripts/gitdepth.sh').write_text(UPDATED_GITDEPTH)
            Path('scripts/gitdepth.sh').chmod(0o755)
            logging.info(f"Installed updated gitdepth.sh")

            # Apply patch if needed
            if needs_patch(commit.hash):
                run_cmd(['patch', '-p1'], input=BENCHMARK_PATCH, check=False)
                logging.info(f"Applied benchmark naming patch")

            # Copy backup CSV into tree so benchmarks can append to it
            shutil.copy(csv_backup, csv_in_tree)
            before_lines = len(open(csv_in_tree).readlines())

            # Run benchmarks
            run_cmd(['cargo', 'clean'], check=False)
            result = run_cmd(['./scripts/run_benchmark.sh'], check=False)

            # Copy updated CSV back to backup location
            shutil.copy(csv_in_tree, csv_backup)
            after_lines = len(open(csv_backup).readlines())
            added_lines = after_lines - before_lines

            if result.returncode == 0:
                logging.info(f"✓ Benchmarked depth {commit.depth} - added {added_lines} CSV rows")
            else:
                logging.error(f"✗ Benchmark failed for depth {commit.depth}")

            # Cleanup - restore files to original state
            if needs_patch(commit.hash):
                run_cmd(['git', 'checkout', 'mtg-benchmarks/benches/game_benchmark.rs'], check=False)
            run_cmd(['git', 'checkout', 'scripts/gitdepth.sh'], check=False)
            run_cmd(['git', 'checkout', str(csv_in_tree)], check=False)  # Restore CSV to clean state

    finally:
        # Restore git state
        logging.info("\n=== Restoring original state ===")

        # Checkout original (CSV should already be clean from last iteration)
        if original_branch:
            run_cmd(['git', 'checkout', original_branch], check=False)
        else:
            run_cmd(['git', 'checkout', original_head], check=False)

        # Copy final CSV back into tree
        if csv_backup.exists():
            shutil.copy(csv_backup, csv_in_tree)
            logging.info(f"✓ Restored CSV to git tree from: {csv_backup}")

        # Cleanup temp files (but keep backup!)
        patch_file.unlink(missing_ok=True)

    lines = len(open(csv_in_tree).readlines())
    logging.info(f"\n✓ Complete! CSV has {lines} lines")
    logging.info(f"✓ Backup preserved at: {csv_backup.absolute()}")
    logging.info(f"✓ To commit: git add {csv_in_tree} && git commit -m 'perf: Backfill depths {start_depth}-{end_depth}'")
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\nInterrupted")
        sys.exit(130)
