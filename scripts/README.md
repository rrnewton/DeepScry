# Scripts

Utility scripts for the MTG Forge Rust project.

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
