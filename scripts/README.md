# Scripts

Utility scripts for the MTG Forge Rust project.

## run_benchmark.sh

Runs benchmarks and records results to both CSV and full log files.

### Usage

```bash
./scripts/run_benchmark.sh [benchmark_name]
```

### Output Files

The script creates two output files in `experiment_results/<CPU_NAME>/`:

1. **CSV File**: `perf_history.csv`
   - Append-only history of extracted metrics
   - One row per benchmark configuration
   - Machine-readable for analysis and plotting

2. **Log File**: `benchmark_log_YYYYMMDD_#depth.log`
   - Complete benchmark stdout including:
     - Full aggregated metrics (turns, actions, duration, allocations)
     - Win rate analysis (P1 vs P2 percentages)
     - Criterion timing estimates and confidence intervals
     - All benchmark output and warnings
   - Human-readable detailed results
   - Includes metadata header with CPU, timestamp, git commit

### Example

```bash
./scripts/run_benchmark.sh rewind_play_again
```

Creates:
- `experiment_results/AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores/perf_history.csv` (appended)
- `experiment_results/AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores/benchmark_log_20251107_#165.log` (new)

## periodically_run_benchmarks.sh

Automatically runs benchmarks when the git depth has advanced by 5 or more commits.

### Usage

```bash
./scripts/periodically_run_benchmarks.sh
```

### Behavior

The script:
1. Gets the current git depth (commit count: `git rev-list --count HEAD`)
2. Reads the last recorded git depth from `experiment_results/perf_history.csv`
3. Calculates the depth delta (difference)
4. **If delta >= 5**: Runs `cargo bench` and appends results to the CSV
5. **If delta < 5**: Skips benchmarks and reports how many more commits are needed

### Example Output

**When skipping:**
```
[INFO] Checking if benchmarks should run...
[INFO] Current git depth: 594
[INFO] Last recorded depth: 591
[INFO] Depth delta: 3
[WARN] Depth delta (3) is less than minimum (5)
[WARN] Skipping benchmarks - need 2 more commits
```

**When running:**
```
[INFO] Checking if benchmarks should run...
[INFO] Current git depth: 594
[INFO] Last recorded depth: 0
[INFO] Depth delta: 594
[SUCCESS] Depth delta (594) >= minimum (5)
[INFO] Running benchmarks...
[INFO] Building benchmarks...
[INFO] Running benchmark suite...
...
[SUCCESS] Benchmarks completed successfully!
[SUCCESS] Results recorded in experiment_results/perf_history.csv (depth: 594)
```

### Integration

This script can be:
- Run manually after development work
- Called from CI/CD pipelines
- Added as a git post-commit hook
- Scheduled with cron for periodic checks

### Configuration

Edit these variables in the script to customize:
- `CSV_FILE`: Path to results file (default: `experiment_results/perf_history.csv`)
- `MIN_DEPTH_DELTA`: Minimum commits before running (default: `5`)
