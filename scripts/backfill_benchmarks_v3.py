#!/usr/bin/env python3
"""
Backfill benchmark results for historical commits.

NEW APPROACH (v3):
- Create .backfill_results/ directory for temporary results
- During benchmarking: checkout commit, run benchmark to real CSV, extract new rows to temp file, clean working directory
- After all benchmarks: consolidate all temp results into real CSV
- Never leave dirty files that block git checkout

Strategy:
- Use inclusive commit counting (all commits, not just first-parent)
- Build map of all commits with their depths
- Only benchmark commits on main branch (first-parent history)
- Keep results in .backfill_results/ during backfill to avoid conflicts
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
@@ -150,7 +150,9 @@ fn bench_rewind_play_again<B, C, F>(
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
    """Build map of depth -> CommitInfo for all commits.

    The depth is the actual commit depth (git rev-list --count $hash),
    not the position in the chronological list. This ensures depth values
    are stable and match what gitdepth.sh returns.
    """
    all_commits = get_all_commits()
    main_commits = get_main_branch_commits()

    commit_map = {}
    for commit_hash in all_commits:
        # Get actual depth of this commit
        result = run_cmd(['git', 'rev-list', '--count', commit_hash])
        depth = int(result.stdout.strip())

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


def clean_working_directory():
    """Clean working directory to ensure git checkout succeeds.

    Only resets tracked files - never removes untracked files/directories
    as this would destroy .devcontainer/ and other valuable configuration.
    """
    # Reset any modified tracked files
    run_cmd(['git', 'reset', '--hard', 'HEAD'], check=False)


def extract_csv_header(csv_file: Path) -> str:
    """Extract header line from CSV file."""
    if csv_file.exists():
        with open(csv_file, 'r') as f:
            return f.readline()
    return 'timestamp,git_commit,git_depth,git_branch,git_dirty,benchmark_name,seed,num_games,total_turns,total_actions,total_duration_ms,avg_turns_per_game,avg_actions_per_game,avg_duration_ms_per_game,games_per_sec,actions_per_sec,turns_per_sec,actions_per_turn,total_bytes_allocated,total_bytes_deallocated,net_bytes,avg_bytes_per_game,bytes_per_turn,bytes_per_sec\n'


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
    results_dir = Path('.backfill_results')
    patch_file = Path('/tmp/benchmark_naming_fix.patch')

    logging.info("")
    logging.info("=== Backfilling Benchmarks ===")
    logging.info(f"CPU: {cpu_name}")
    logging.info(f"Depth range: {start_depth} to {end_depth}")
    logging.info(f"Cadence: every {args.cadence}")
    logging.info(f"Patch commit: {PATCH_COMMIT_HASH}")
    logging.info(f"Log file: {log_file.absolute()}")
    logging.info(f"Results directory: {results_dir.absolute()}")
    logging.info("")

    # Get already-benchmarked commits for idempotency
    benchmarked_commits = get_benchmarked_commits(csv_in_tree)
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

    # Create results directory
    results_dir.mkdir(exist_ok=True)
    logging.info(f"\nCreated results directory: {results_dir}")

    # Get CSV header
    csv_header = extract_csv_header(csv_in_tree)
    logging.info(f"CSV header: {csv_header.strip()}")

    # Write patch file
    patch_file.write_text(BENCHMARK_PATCH)

    # Track benchmarking progress
    successful_benchmarks = []
    failed_benchmarks = []

    try:
        # Benchmark each commit
        for i, commit in enumerate(commits_to_bench, 1):
            logging.info(f"\n[{i}/{len(commits_to_bench)}] Benchmarking depth {commit.depth} ({commit.short_hash})")

            # Clean working directory before checkout
            clean_working_directory()

            # Checkout
            result = run_cmd(['git', 'checkout', commit.hash], check=False)
            if result.returncode != 0:
                logging.error(f"✗ Checkout failed for {commit.short_hash}: {result.stderr}")
                failed_benchmarks.append((commit.depth, commit.short_hash, "checkout failed"))
                continue

            # Verify we checked out the right commit
            result = run_cmd(['git', 'rev-parse', 'HEAD'])
            actual_hash = result.stdout.strip()
            if actual_hash != commit.hash:
                logging.error(f"✗ Checkout mismatch: expected {commit.hash}, got {actual_hash}")
                failed_benchmarks.append((commit.depth, commit.short_hash, "checkout mismatch"))
                continue
            logging.info(f"✓ Checked out {commit.short_hash}")

            # Install updated gitdepth.sh
            Path('scripts/gitdepth.sh').write_text(UPDATED_GITDEPTH)
            Path('scripts/gitdepth.sh').chmod(0o755)
            logging.info(f"✓ Installed updated gitdepth.sh")

            # Apply patch if needed
            if needs_patch(commit.hash):
                result = run_cmd(['patch', '-p1'], input=BENCHMARK_PATCH, check=False)
                if result.returncode != 0:
                    logging.warning(f"⚠ Patch failed (may already be applied): {result.stderr}")
                else:
                    logging.info(f"✓ Applied benchmark naming patch")

            # Get baseline line count of CSV
            csv_in_tree.parent.mkdir(parents=True, exist_ok=True)
            if not csv_in_tree.exists():
                csv_in_tree.write_text(csv_header)
            before_lines = len(open(csv_in_tree).readlines())

            # Run benchmarks
            logging.info(f"Running cargo clean...")
            run_cmd(['cargo', 'clean'], check=False)
            logging.info(f"Running benchmarks...")
            result = run_cmd(['./scripts/run_benchmark.sh'], check=False, timeout=600)

            if result.returncode != 0:
                logging.error(f"✗ Benchmark failed for depth {commit.depth}: {result.stderr[-500:]}")
                failed_benchmarks.append((commit.depth, commit.short_hash, "benchmark failed"))
                clean_working_directory()
                continue

            # Extract new rows from CSV
            if csv_in_tree.exists():
                with open(csv_in_tree, 'r') as f:
                    all_lines = f.readlines()
                new_lines = all_lines[before_lines:]
                added_lines = len(new_lines)

                if added_lines > 0:
                    # Save new rows to results directory
                    result_file = results_dir / f"depth_{commit.depth}.csv"
                    with open(result_file, 'w') as f:
                        f.write(csv_header)
                        f.writelines(new_lines)
                    logging.info(f"✓ Benchmarked depth {commit.depth} - saved {added_lines} rows to {result_file.name}")
                    successful_benchmarks.append((commit.depth, commit.short_hash, added_lines))
                else:
                    logging.warning(f"⚠ No new rows added for depth {commit.depth}")
                    failed_benchmarks.append((commit.depth, commit.short_hash, "no rows"))
            else:
                logging.error(f"✗ CSV file not found after benchmarking")
                failed_benchmarks.append((commit.depth, commit.short_hash, "csv missing"))

            # Clean working directory for next iteration
            clean_working_directory()

    finally:
        # Restore git state
        logging.info("\n=== Restoring original state ===")
        clean_working_directory()
        if original_branch:
            run_cmd(['git', 'checkout', original_branch], check=False)
        else:
            run_cmd(['git', 'checkout', original_head], check=False)
        logging.info(f"✓ Restored to original state")

        # Cleanup temp files
        patch_file.unlink(missing_ok=True)

    # Consolidate results
    logging.info("\n=== Consolidating results ===")
    result_files = sorted(results_dir.glob("depth_*.csv"), key=lambda p: int(p.stem.split('_')[1]))

    if result_files:
        # Read existing CSV
        existing_lines = []
        if csv_in_tree.exists():
            with open(csv_in_tree, 'r') as f:
                existing_lines = f.readlines()
        else:
            existing_lines = [csv_header]

        # Collect all new rows
        new_rows = []
        for result_file in result_files:
            with open(result_file, 'r') as f:
                lines = f.readlines()
                # Skip header
                new_rows.extend(lines[1:])

        # Write consolidated CSV
        with open(csv_in_tree, 'w') as f:
            f.writelines(existing_lines)
            f.writelines(new_rows)

        total_rows = len(new_rows)
        logging.info(f"✓ Added {total_rows} rows to {csv_in_tree}")
        logging.info(f"✓ CSV now has {len(existing_lines) + total_rows - 1} data rows (plus header)")

    # Summary
    logging.info("\n=== Summary ===")
    logging.info(f"Successful benchmarks: {len(successful_benchmarks)}")
    for depth, short_hash, rows in successful_benchmarks:
        logging.info(f"  ✓ depth {depth} ({short_hash}): {rows} rows")

    if failed_benchmarks:
        logging.info(f"\nFailed benchmarks: {len(failed_benchmarks)}")
        for depth, short_hash, reason in failed_benchmarks:
            logging.info(f"  ✗ depth {depth} ({short_hash}): {reason}")

    logging.info(f"\n✓ Complete!")
    logging.info(f"✓ Results directory: {results_dir.absolute()}")
    if result_files:
        logging.info(f"✓ To commit: git add {csv_in_tree} && git commit -m 'perf: Backfill depths {start_depth}-{end_depth}'")

    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\nInterrupted")
        sys.exit(130)
