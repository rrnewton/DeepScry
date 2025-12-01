#!/usr/bin/env python3
"""
Backfill benchmark results for historical commits with proper labeling.

This script systematically benchmarks historical commits, applying a patch
to fix benchmark naming where needed, ensuring different deck matchups are
properly distinguished in the CSV output.
"""

import argparse
import csv
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional, Set


@dataclass
class CommitInfo:
    """Information about a commit to benchmark."""
    hash: str
    short_hash: str
    depth: int  # First-parent depth
    message: str


# Benchmark naming fix patch (introduced at depth 823, commit 72d86871)
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


def run_command(cmd: List[str], check=True, capture_output=True, text=True, **kwargs):
    """Run a command and return result."""
    return subprocess.run(cmd, check=check, capture_output=capture_output, text=text, **kwargs)


def get_cpu_name() -> str:
    """Get CPU name from lscpu."""
    result = run_command(['lscpu'])
    for line in result.stdout.splitlines():
        if line.startswith('Model name:'):
            name = line.split(':', 1)[1].strip()
            # Replace spaces with underscores
            return name.replace(' ', '_')
    raise RuntimeError("Could not determine CPU name")


def get_current_branch() -> Optional[str]:
    """Get current branch name, or None if detached HEAD."""
    result = run_command(['git', 'rev-parse', '--abbrev-ref', 'HEAD'])
    branch = result.stdout.strip()
    return None if branch == 'HEAD' else branch


def get_first_parent_depth(commit: str = 'HEAD') -> int:
    """Get first-parent depth of a commit (like gitdepth.sh but with --first-parent)."""
    result = run_command(['git', 'rev-list', '--count', '--first-parent', commit])
    return int(result.stdout.strip())


def get_commit_at_depth(depth: int) -> Optional[CommitInfo]:
    """Get commit info for a specific first-parent depth."""
    # Get all first-parent commits in reverse order (oldest first)
    result = run_command(['git', 'rev-list', '--first-parent', '--reverse', 'HEAD'])
    commits = result.stdout.strip().split('\n')

    if depth < 1 or depth > len(commits):
        return None

    commit_hash = commits[depth - 1]
    short_hash = commit_hash[:8]

    # Get commit message
    result = run_command(['git', 'log', '-1', '--oneline', commit_hash])
    message = result.stdout.strip()

    return CommitInfo(
        hash=commit_hash,
        short_hash=short_hash,
        depth=depth,
        message=message
    )


def is_docs_only_commit(commit_hash: str) -> bool:
    """Check if a commit only changes documentation/non-code files."""
    result = run_command(['git', 'diff-tree', '--no-commit-id', '--name-only', '-r', commit_hash])
    changed_files = [f for f in result.stdout.strip().split('\n') if f]

    if not changed_files:
        return True  # Empty commit

    docs_patterns = ['.beads/', 'docs/', '.md', 'experiment_results/', 'scripts/plot_performance', '.gitignore']

    for file in changed_files:
        # Check if file is NOT a docs file
        is_docs = any(pattern in file for pattern in docs_patterns)
        if not is_docs:
            return False  # Found a code file

    return True  # All files are docs


def verify_depth(expected_depth: int) -> bool:
    """Verify the current checkout has expected depth."""
    actual_depth = get_first_parent_depth('HEAD')
    return actual_depth == expected_depth


def apply_patch(patch_file: Path) -> bool:
    """Apply benchmark naming patch. Returns True if successful or already applied."""
    result = run_command(['patch', '-p1', '-i', str(patch_file)], check=False, capture_output=True)

    if result.returncode == 0:
        print("✓ Patch applied successfully")
        return True

    # Check if already present
    try:
        with open('mtg-benchmarks/benches/game_benchmark.rs', 'r') as f:
            if 'format!("{}/{}", group_name, bench_name)' in f.read():
                print("✓ Benchmark naming fix already present")
                return True
    except FileNotFoundError:
        pass

    print("⚠ Warning: Could not apply patch and fix not present")
    return False


def run_benchmarks() -> bool:
    """Run benchmark script. Returns True if successful."""
    result = run_command(['./scripts/run_benchmark.sh'], check=False)
    return result.returncode == 0


class BackfillManager:
    """Manages the backfill process."""

    def __init__(self, csv_path: Path):
        self.csv_path = csv_path
        self.original_branch = get_current_branch()
        self.original_head = run_command(['git', 'rev-parse', 'HEAD']).stdout.strip()
        self.csv_backup = csv_path.with_suffix('.csv.backfill_backup')
        self.patch_file = Path('/tmp/benchmark_naming_fix.patch')

    def __enter__(self):
        """Setup: save current state and create patch file."""
        # Save CSV backup
        if self.csv_path.exists():
            import shutil
            shutil.copy(self.csv_path, self.csv_backup)
            print(f"Created CSV backup: {self.csv_backup}")

        # Write patch file
        self.patch_file.write_text(BENCHMARK_PATCH)
        print(f"Patch saved to: {self.patch_file}\n")

        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        """Cleanup: restore git state and remove temporary files."""
        print("\n=== Restoring original state ===")

        # Restore CSV on error
        if exc_type is not None and self.csv_backup.exists():
            import shutil
            print("Error detected - restoring CSV from backup")
            shutil.copy(self.csv_backup, self.csv_path)

        # Save final CSV before git checkout (to avoid conflicts)
        csv_final = Path('/tmp/backfill_final_csv.csv')
        if self.csv_path.exists():
            import shutil
            shutil.copy(self.csv_path, csv_final)

        # Restore git state
        if self.original_branch:
            run_command(['git', 'checkout', self.original_branch], check=False, capture_output=False)
        else:
            run_command(['git', 'checkout', self.original_head], check=False, capture_output=False)

        # Restore final CSV after git checkout
        if csv_final.exists():
            import shutil
            shutil.copy(csv_final, self.csv_path)
            csv_final.unlink()

        # Cleanup temp files
        for temp_file in [self.csv_backup, self.patch_file,
                         Path('/tmp/backfill_csv_current.csv'),
                         Path('/tmp/gitdepth.sh.backup')]:
            if temp_file.exists():
                temp_file.unlink()

    def benchmark_commit(self, commit: CommitInfo) -> bool:
        """Benchmark a single commit. Returns True if successful."""
        print(f"\n=== Benchmarking depth {commit.depth} ({commit.short_hash}) ===")
        print(commit.message)
        print()

        # Save CSV to temp location before checkout (so git doesn't complain)
        csv_temp = Path('/tmp/backfill_csv_current.csv')
        if self.csv_path.exists():
            import shutil
            shutil.copy(self.csv_path, csv_temp)

        # Checkout commit
        result = run_command(['git', 'checkout', commit.hash], check=False, capture_output=False)
        if result.returncode != 0:
            print(f"✗ Failed to checkout {commit.short_hash}")
            # Restore CSV if needed
            if csv_temp.exists():
                shutil.copy(csv_temp, self.csv_path)
                csv_temp.unlink()
            return False

        # Restore CSV after checkout
        if csv_temp.exists():
            import shutil
            shutil.copy(csv_temp, self.csv_path)
            csv_temp.unlink()

        # Verify depth matches expectation
        if not verify_depth(commit.depth):
            actual = get_first_parent_depth('HEAD')
            print(f"✗ Depth mismatch! Expected {commit.depth}, got {actual}")
            print(f"  This indicates inconsistent git history calculation")
            return False

        print(f"✓ Checked out commit at depth {commit.depth}")

        # Install updated gitdepth.sh so benchmarks record correct depth
        # Save original first
        gitdepth_backup = Path('/tmp/gitdepth.sh.backup')
        gitdepth_path = Path('scripts/gitdepth.sh')
        import shutil
        if gitdepth_path.exists():
            shutil.copy(gitdepth_path, gitdepth_backup)

        # Write updated version
        gitdepth_path.write_text('''#!/bin/bash
# Count commits in first-parent (main branch) history only
# This matches what users see in `git log --oneline --first-parent`
git rev-list --count --first-parent HEAD
''')
        gitdepth_path.chmod(0o755)
        print(f"✓ Installed updated gitdepth.sh for consistent depth recording")

        # Apply patch if needed (only for commits older than patch introduction)
        needs_patch = commit.depth < PATCH_INTRODUCED_DEPTH
        if needs_patch:
            print(f"Applying benchmark naming patch (commit is older than depth {PATCH_INTRODUCED_DEPTH})...")
            if not apply_patch(self.patch_file):
                print("  Continuing anyway (benchmarks may have poor labeling)")
        else:
            print(f"Skipping patch (commit depth {commit.depth} >= {PATCH_INTRODUCED_DEPTH}, already has fix)")

        # Clean build
        print("Cleaning build artifacts...")
        run_command(['cargo', 'clean'], check=False, capture_output=True)

        # Save CSV before benchmarking
        csv_pre_bench = self.csv_path.with_suffix('.csv.pre_bench')
        if self.csv_path.exists():
            import shutil
            shutil.copy(self.csv_path, csv_pre_bench)

        # Run benchmarks
        print("\nRunning benchmarks...")
        success = run_benchmarks()

        if success:
            print(f"✓ Benchmarked depth {commit.depth}")
            csv_pre_bench.unlink(missing_ok=True)
        else:
            print(f"✗ Benchmark failed for depth {commit.depth}")
            # Restore CSV
            if csv_pre_bench.exists():
                import shutil
                shutil.copy(csv_pre_bench, self.csv_path)
                csv_pre_bench.unlink()

        # Restore modified files
        if needs_patch:
            run_command(['git', 'checkout', 'mtg-benchmarks/benches/game_benchmark.rs'],
                       check=False, capture_output=True)

        # Restore original gitdepth.sh
        if gitdepth_backup.exists():
            import shutil
            shutil.copy(gitdepth_backup, gitdepth_path)
            gitdepth_backup.unlink()

        return success


def main():
    parser = argparse.ArgumentParser(
        description='Backfill benchmark results for historical commits',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog='''
Examples:
  %(prog)s --depth-range 1 100      # Fill depths 1-100
  %(prog)s --last-n 30              # Fill last 30 commits
  %(prog)s --last-n 30 --cadence 2  # Every 2nd commit (1,3,5,...)
  %(prog)s --dry-run --last-n 30    # Show what would be done
        '''
    )

    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--depth-range', nargs=2, type=int, metavar=('START', 'END'),
                      help='Depth range to fill (inclusive)')
    group.add_argument('--last-n', type=int, metavar='N',
                      help='Fill last N commits')

    parser.add_argument('--cadence', type=int, default=1, metavar='N',
                       help='Benchmark every Nth commit (default: 1)')
    parser.add_argument('--dry-run', action='store_true',
                       help='Show what would be done without running benchmarks')

    args = parser.parse_args()

    # Determine depth range
    current_depth = get_first_parent_depth('HEAD')

    if args.depth_range:
        start_depth, end_depth = args.depth_range
    else:  # --last-n
        start_depth = max(1, current_depth - args.last_n + 1)
        end_depth = current_depth

    # Get CPU and CSV path
    cpu_name = get_cpu_name()
    results_dir = Path('experiment_results') / cpu_name
    csv_path = results_dir / 'perf_history.csv'

    # Print configuration
    print("=== Backfilling Benchmarks ===")
    print(f"CPU: {cpu_name}")
    print(f"Depth range: {start_depth} to {end_depth} ({end_depth - start_depth + 1} commits)")
    print(f"Cadence: every {args.cadence} commit(s)")
    print(f"Results file: {csv_path}")
    print(f"Dry run: {args.dry_run}")
    print(f"Patch introduced at: depth {PATCH_INTRODUCED_DEPTH}")
    print()

    # Verify gitdepth.sh consistency
    gitdepth_result = run_command(['./scripts/gitdepth.sh'])
    gitdepth_value = int(gitdepth_result.stdout.strip())

    if gitdepth_value != current_depth:
        print(f"⚠ WARNING: gitdepth.sh reports {gitdepth_value} but first-parent depth is {current_depth}")
        print(f"  gitdepth.sh may need updating to use --first-parent")
        print()

    # Analyze commits in range
    print("=== Analyzing commits in range ===")

    commits_to_benchmark: List[CommitInfo] = []
    commits_to_skip: List[int] = []

    for depth in range(start_depth, end_depth + 1, args.cadence):
        commit = get_commit_at_depth(depth)
        if not commit:
            print(f"⚠ Warning: No commit found at depth {depth}")
            continue

        if is_docs_only_commit(commit.hash):
            print(f"Skip depth {depth} (docs-only): {commit.message[:60]}")
            commits_to_skip.append(depth)
            continue

        commits_to_benchmark.append(commit)
        print(f"Need depth {depth}: {commit.message[:60]}")

    print()
    print("=== Summary ===")
    print(f"Commits to skip (docs-only): {len(commits_to_skip)}")
    print(f"Commits to benchmark: {len(commits_to_benchmark)}")
    print()

    if args.dry_run:
        print("=== Dry run - would benchmark these commits ===")
        for commit in commits_to_benchmark:
            print(f"  Depth {commit.depth} ({commit.short_hash}): {commit.message}")
        print()
        print("Next step (without --dry-run): Benchmark missing commits")
        return 0

    # Run backfill
    with BackfillManager(csv_path) as manager:
        benchmarked = 0
        failed = 0

        for i, commit in enumerate(commits_to_benchmark, 1):
            print(f"\n[{i}/{len(commits_to_benchmark)}]")

            if manager.benchmark_commit(commit):
                benchmarked += 1
            else:
                failed += 1
                print("Continuing with next commit...")

        # Success message
        print()
        print("=== Backfill complete ===")
        print(f"Benchmarked: {benchmarked} commits")
        print(f"Failed: {failed} commits")
        print(f"Skipped (docs-only): {len(commits_to_skip)} commits")
        print()
        print(f"Results saved to: {csv_path}")
        print()
        print("Next steps:")
        print("  1. Regenerate plots: make plot")
        print("  2. Review results in browser")
        print(f"  3. Commit results: git add {csv_path} && git commit -m 'perf: Backfill benchmark results for depths {start_depth}-{end_depth}'")

    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n\nInterrupted by user")
        sys.exit(130)
    except Exception as e:
        print(f"\nError: {e}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        sys.exit(1)
