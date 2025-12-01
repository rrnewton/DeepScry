#!/bin/bash
# Backfill benchmark results for historical commits with benchmark naming fix
#
# This script applies a patch to fix benchmark naming (to distinguish different
# deck matchups) before running benchmarks on each historical commit.
#
# Usage:
#   ./scripts/backfill_benchmarks.sh [--depth-range START END] [--last-n N] [--cadence N] [--dry-run]
#
# Examples:
#   ./scripts/backfill_benchmarks.sh --depth-range 1000 1030  # Fill depths 1000-1030
#   ./scripts/backfill_benchmarks.sh --last-n 30             # Fill last 30 commits
#   ./scripts/backfill_benchmarks.sh --last-n 30 --cadence 2 # Every 2nd commit (1006,1008,1010...)
#   ./scripts/backfill_benchmarks.sh --dry-run --last-n 30   # Show what would be done

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Detect CPU name for output directory
CPU_NAME=$(lscpu | grep "Model name:" | sed 's/Model name: *//' | sed 's/ \+/_/g')
RESULTS_DIR="experiment_results/${CPU_NAME}"
CSV_FILE="${RESULTS_DIR}/perf_history.csv"

# Parse arguments
START_DEPTH=""
END_DEPTH=""
LAST_N=""
CADENCE=1
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --depth-range)
            START_DEPTH="$2"
            END_DEPTH="$3"
            shift 3
            ;;
        --last-n)
            LAST_N="$2"
            shift 2
            ;;
        --cadence)
            CADENCE="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--depth-range START END] [--last-n N] [--cadence N] [--dry-run]"
            exit 1
            ;;
    esac
done

# Validate arguments
if [[ -n "$START_DEPTH" && -n "$LAST_N" ]]; then
    echo "Error: Cannot specify both --depth-range and --last-n"
    exit 1
fi

if [[ -z "$START_DEPTH" && -z "$LAST_N" ]]; then
    echo "Error: Must specify either --depth-range or --last-n"
    exit 1
fi

# Calculate depth range
CURRENT_DEPTH=$(git rev-list --count HEAD)
if [[ -n "$LAST_N" ]]; then
    START_DEPTH=$((CURRENT_DEPTH - LAST_N + 1))
    END_DEPTH=$CURRENT_DEPTH
fi

echo "=== Backfilling Benchmarks ==="
echo "CPU: $CPU_NAME"
echo "Depth range: $START_DEPTH to $END_DEPTH ($((END_DEPTH - START_DEPTH + 1)) commits)"
echo "Cadence: every ${CADENCE} commit(s)"
echo "Results file: $CSV_FILE"
echo "Dry run: $DRY_RUN"
echo ""

# Save current HEAD
ORIGINAL_HEAD=$(git rev-parse HEAD)
ORIGINAL_BRANCH=$(git branch --show-current || echo "detached")

# Create patch file for benchmark naming fix
PATCH_FILE="/tmp/benchmark_naming_fix.patch"
echo "=== Creating benchmark naming patch ==="
cat > "$PATCH_FILE" << 'PATCH_EOF'
--- a/mtg-benchmarks/benches/game_benchmark.rs
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
PATCH_EOF
echo "Patch saved to: $PATCH_FILE"
echo ""

# Function to restore original state
restore_state() {
    local exit_code=$?
    echo ""
    echo "=== Restoring original state ==="

    # If there was an error and we have a backup, restore it
    if [[ $exit_code -ne 0 && -f "${CSV_FILE}.backfill_backup" ]]; then
        echo "Error detected - restoring CSV from backup"
        cp "${CSV_FILE}.backfill_backup" "$CSV_FILE"
    fi

    # Restore git state (suppress submodule warnings)
    if [[ "$ORIGINAL_BRANCH" == "detached" ]]; then
        git checkout "$ORIGINAL_HEAD" 2>&1 | grep -v "Submodule" || true
    else
        git checkout "$ORIGINAL_BRANCH" 2>&1 | grep -v "Submodule" || true
    fi

    # Clean up patch file and backups
    rm -f "$PATCH_FILE"
    rm -f "${CSV_FILE}.backfill_backup"
    rm -f "${CSV_FILE}.pre_bench"
    rm -f "/tmp/cleaned_perf_history.csv"
    rm -f "/tmp/backfill_final.csv"

    exit $exit_code
}

# Trap to ensure we restore state on exit
trap restore_state EXIT

# Function to check if commit only changes non-code files
is_docs_only_commit() {
    local commit_hash=$1

    # Get list of changed files
    local changed_files=$(git diff-tree --no-commit-id --name-only -r "$commit_hash" 2>/dev/null || echo "")

    if [[ -z "$changed_files" ]]; then
        # Empty commit or initial commit
        return 0
    fi

    # Check if any changed file is code
    local has_code=false
    while IFS= read -r file; do
        # Skip empty lines
        [[ -z "$file" ]] && continue

        # Skip if file matches documentation/issues/results patterns
        if [[ "$file" =~ ^\.beads/ ]] || \
           [[ "$file" =~ ^docs/ ]] || \
           [[ "$file" =~ \.md$ ]] || \
           [[ "$file" =~ ^experiment_results/ ]] || \
           [[ "$file" =~ ^scripts/plot_performance.*\.py$ ]] || \
           [[ "$file" =~ ^\.gitignore$ ]]; then
            continue  # Skip non-code files
        fi

        # Found a code file
        has_code=true
        break
    done <<< "$changed_files"

    if [[ "$has_code" == true ]]; then
        return 1  # Has code changes
    else
        return 0  # Docs-only
    fi
}

# Get commit info for depth range
echo "=== Analyzing commits in range ==="

COMMITS_TO_BENCHMARK=()
COMMITS_TO_SKIP=()

# Get commits in depth range (oldest first, with cadence)
for depth in $(seq $START_DEPTH $CADENCE $END_DEPTH); do
    # Get commit at this depth
    commit_hash=$(git rev-list --reverse HEAD | sed -n "${depth}p")

    if [[ -z "$commit_hash" ]]; then
        echo "Warning: No commit found at depth $depth"
        continue
    fi

    commit_short=$(echo "$commit_hash" | cut -c1-8)

    # Check if this is a docs-only commit
    if is_docs_only_commit "$commit_hash"; then
        echo "Skip depth $depth (docs-only): $(git log -1 --oneline $commit_hash | head -c 60)"
        COMMITS_TO_SKIP+=("$depth")
        continue
    fi

    # Add to benchmark list
    COMMITS_TO_BENCHMARK+=("$depth|$commit_hash|$commit_short")
    echo "Need depth $depth: $(git log -1 --oneline $commit_hash | head -c 60)"
done

echo ""
echo "=== Summary ==="
echo "Commits to skip (docs-only): ${#COMMITS_TO_SKIP[@]}"
echo "Commits to benchmark: ${#COMMITS_TO_BENCHMARK[@]}"
echo ""

if [[ "$DRY_RUN" == true ]]; then
    echo "=== Dry run - would benchmark these commits ==="
    for item in "${COMMITS_TO_BENCHMARK[@]}"; do
        IFS='|' read -r depth hash short <<< "$item"
        echo "  Depth $depth ($short): $(git log -1 --oneline $hash)"
    done
    echo ""
    echo "Next step (without --dry-run): Clean CSV and benchmark missing commits"
    exit 0
fi

# Save backup of CSV before we start
CSV_BACKUP="${CSV_FILE}.backfill_backup"
CSV_CLEANED="/tmp/cleaned_perf_history.csv"
if [ -f "$CSV_FILE" ]; then
    cp "$CSV_FILE" "$CSV_BACKUP"
    echo "Created CSV backup: $CSV_BACKUP"
fi
echo ""

# Use Python to clean CSV based on reuse policy (write to temp file)
echo "=== Cleaning CSV based on reuse policy ==="
python3 << PYTHON_SCRIPT
import csv
import sys
import subprocess

csv_file = "$CSV_FILE"
cleaned_csv = "$CSV_CLEANED"
start_depth = $START_DEPTH
end_depth = $END_DEPTH

# Read existing CSV
rows = []
header = None
if open(csv_file).read().strip():
    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)
        header = reader.fieldnames
        rows = list(reader)

# Get current git info for each depth in range
depth_to_hash = {}
for depth in range(start_depth, end_depth + 1):
    result = subprocess.run(
        ['git', 'rev-list', '--reverse', 'HEAD'],
        capture_output=True, text=True, check=True
    )
    commits = result.stdout.strip().split('\n')
    if depth <= len(commits):
        depth_to_hash[depth] = commits[depth - 1]

# Filter rows based on reuse policy
kept_rows = []
removed_count = 0

for row in rows:
    depth = int(row['git_depth'])
    existing_hash = row['git_commit']
    existing_branch = row['git_branch']

    # If outside our range, keep it
    if depth < start_depth or depth > end_depth:
        kept_rows.append(row)
        continue

    # If no commit exists at this depth, remove it
    if depth not in depth_to_hash:
        print(f"Warning: Removing depth {depth} - no commit exists at this depth", file=sys.stderr)
        removed_count += 1
        continue

    actual_hash = depth_to_hash[depth]

    # Check if hash matches
    if existing_hash == actual_hash:
        # Reuse this result
        kept_rows.append(row)
        print(f"Reusing depth {depth}: {existing_hash[:8]}", file=sys.stderr)
    else:
        # Hash mismatch - apply policy
        if existing_branch != 'main':
            print(f"Warning: Removing depth {depth} - non-main branch with mismatched hash", file=sys.stderr)
            print(f"  Existing: {existing_hash[:8]} ({existing_branch}), Actual: {actual_hash[:8]}", file=sys.stderr)
            removed_count += 1
        else:
            print(f"Warning: Removing depth {depth} - main branch with mismatched hash (forced update?)", file=sys.stderr)
            print(f"  Existing: {existing_hash[:8]}, Actual: {actual_hash[:8]}", file=sys.stderr)
            removed_count += 1

# Write cleaned CSV to temp file
with open(cleaned_csv, 'w', newline='') as f:
    if header:
        writer = csv.DictWriter(f, fieldnames=header)
        writer.writeheader()
        writer.writerows(kept_rows)

print(f"Removed {removed_count} rows, kept {len(kept_rows)} rows", file=sys.stderr)
PYTHON_SCRIPT

# Keep cleaned CSV for later use
echo ""

# Now benchmark missing commits
BENCHMARK_COUNT=0
TOTAL_BENCHMARKS=${#COMMITS_TO_BENCHMARK[@]}

for item in "${COMMITS_TO_BENCHMARK[@]}"; do
    IFS='|' read -r depth hash short <<< "$item"
    BENCHMARK_COUNT=$((BENCHMARK_COUNT + 1))

    # Check if we already have a result for this depth after cleaning
    if grep -q "^[^,]*,[^,]*,$depth," "$CSV_CLEANED" 2>/dev/null; then
        echo "[$BENCHMARK_COUNT/$TOTAL_BENCHMARKS] Skip depth $depth ($short) - already benchmarked"
        continue
    fi

    echo ""
    echo "=== [$BENCHMARK_COUNT/$TOTAL_BENCHMARKS] Benchmarking depth $depth ($short) ==="
    git log -1 --oneline "$hash"
    echo ""

    # Checkout the commit (suppress submodule warnings)
    git checkout "$hash" 2>&1 | grep -v "Submodule" || true

    # Now install the cleaned CSV (CSV_FILE at this commit may be different)
    cp "$CSV_CLEANED" "$CSV_FILE"

    # Apply benchmark naming patch
    echo "Applying benchmark naming patch..."
    if patch -p1 < "$PATCH_FILE" >/dev/null 2>&1; then
        echo "✓ Patch applied successfully"
    else
        # Patch may already be applied or not applicable
        echo "Note: Patch did not apply (may already be present or N/A for this commit)"
        # Check if the fix is already present
        if grep -q 'format!("{}/{}", group_name, bench_name)' mtg-benchmarks/benches/game_benchmark.rs 2>/dev/null; then
            echo "✓ Benchmark naming fix already present in this commit"
        else
            echo "Warning: Could not apply patch and fix not present - benchmarks may have poor labeling"
        fi
    fi

    # Clean any previous build artifacts to ensure fresh build
    cargo clean -q 2>/dev/null || true

    # Save CSV before benchmarking (in case benchmark fails partway)
    CSV_PRE_BENCH="${CSV_FILE}.pre_bench"
    cp "$CSV_FILE" "$CSV_PRE_BENCH"

    # Run benchmark (which appends to CSV)
    if ! ./scripts/run_benchmark.sh; then
        echo "Error: Benchmark failed for depth $depth"
        # Restore CSV to pre-benchmark state
        echo "Restoring CSV to pre-benchmark state"
        cp "$CSV_PRE_BENCH" "$CSV_FILE"
        rm -f "$CSV_PRE_BENCH"
        # Restore clean state before continuing
        git checkout mtg-benchmarks/benches/game_benchmark.rs 2>/dev/null || true
        echo "Continuing with next commit..."
        continue
    fi

    # Benchmark succeeded - remove pre-benchmark backup
    rm -f "$CSV_PRE_BENCH"

    # Restore game_benchmark.rs to original state (undo patch)
    git checkout mtg-benchmarks/benches/game_benchmark.rs 2>/dev/null || true

    echo "✓ Benchmarked depth $depth"
done

# Success! Copy final CSV to a safe location before restoring
FINAL_CSV="/tmp/backfill_final.csv"
cp "$CSV_FILE" "$FINAL_CSV"
rm -f "${CSV_FILE}.backfill_backup"
rm -f "$CSV_CLEANED"

# Restore to original state
restore_state

# Copy the final CSV back
cp "$FINAL_CSV" "$CSV_FILE"
rm -f "$FINAL_CSV"

echo ""
echo "=== Backfill complete ==="
echo "Benchmarked: $BENCHMARK_COUNT commits"
echo "Skipped (docs-only): ${#COMMITS_TO_SKIP[@]} commits"
echo ""
echo "Results saved to: $CSV_FILE"
echo ""
echo "Next steps:"
echo "  1. Regenerate plots: make plot"
echo "  2. Review results in browser"
echo "  3. Commit results: git add $CSV_FILE && git commit -m 'perf: Backfill benchmark results for depths $START_DEPTH-$END_DEPTH'"
