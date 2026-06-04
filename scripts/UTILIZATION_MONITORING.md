# System Utilization Monitoring

This directory contains scripts to measure and report system CPU utilization during validation runs.

## Overview

The monitoring system consists of two scripts that work together:

1. **utilization_prehook.sh** - Starts background CPU monitoring
2. **utilization_posthook.sh** - Stops monitoring and generates a detailed report

These scripts are automatically invoked by `scripts/validate.py` (a full `make validate`) to provide insights into how efficiently the validation process uses available CPU cores.

## Integration with validate.py

`scripts/validate.py`'s outer harness invokes these hooks as SUBPROCESSES (not `source`d — the harness is Python now):

- **Before validation**: runs `utilization_prehook.sh`, which backgrounds a disowned sampler and prints its PID + stats-file path (the harness parses these).
- **After validation**: runs `utilization_posthook.sh` with `UTIL_MONITOR_PID` / `UTIL_STATS_FILE` / `UTIL_START_TIME` passed explicitly in the environment, to stop the sampler and print the report.

Disable with `python3 scripts/validate.py --no-monitor-utilization`.

The monitoring runs regardless of whether validation succeeds or fails.

## Report Metrics

The utilization report includes:

### Basic Statistics
- **Duration**: Total time monitored (in seconds)
- **CPU Cores**: Number of available CPU cores
- **Samples**: Number of utilization measurements collected
- **Average/Min/Max**: CPU utilization percentages

### Time Distribution
Shows what percentage of time was spent at different CPU utilization levels:
- 0-25% busy
- 25-50% busy
- 50-75% busy
- 75-90% busy
- 90-100% busy

### Parallelism Assessment
Automatic evaluation of parallelism efficiency:

- **Excellent** (≥90% avg): CPUs are well-utilized throughout execution
- **Good** (≥75% avg): Most of the time CPUs are busy
- **Moderate** (≥50% avg): There's room to improve CPU utilization
- **Poor** (<50% avg): CPUs are underutilized - significant room for improvement

## Implementation Details

### Monitoring Method

The prehook script uses one of two methods, in order of preference:

1. **mpstat** (from sysstat package): More accurate, per-CPU measurements
2. **/proc/stat fallback**: Manual parsing of kernel statistics

Samples are collected every 1 second and stored in a temporary file.

### CPU Utilization Calculation

- For mpstat: `busy% = 100 - idle%`
- For /proc/stat: `busy% = 100 * (total_delta - idle_delta) / total_delta`

### Interpretation

**High utilization (>75%)**: Indicates good parallelism. Most CPU cores are busy most of the time.

**Low utilization (<50%)**: Suggests opportunities for improvement:
- Sequential bottlenecks in the test suite
- I/O-bound operations (disk, network)
- Insufficient parallel test execution
- Single-threaded compilation steps

**Uneven distribution**: Large amounts of time at low utilization followed by brief peaks suggests:
- Sequential phases (e.g., compilation) followed by parallel phases (e.g., test execution)
- Consider overlapping these phases if possible

## Using Standalone

While `validate.py` invokes these as subprocesses, they can also be sourced/used standalone:

```bash
# Start monitoring
source scripts/utilization_prehook.sh

# Run your workload
make test

# Stop monitoring and see report
source scripts/utilization_posthook.sh
```

## Requirements

- **bc**: For floating-point calculations
- **nproc**: To determine CPU core count
- **mpstat** (optional but recommended): For accurate CPU monitoring
  - Install via: `apt-get install sysstat` (Debian/Ubuntu)
- **/proc/stat**: Fallback if mpstat not available

## Example Output

```
========================================
System Utilization Report
========================================

Duration:         34s
CPU Cores:        16
Samples:          33

CPU Utilization:
  Average:        34.8%
  Minimum:        6.3%
  Maximum:        99.8%

Time Distribution:
  0-25% busy:      9 samples ( 27.2%)
  25-50% busy:    18 samples ( 54.5%)
  50-75% busy:     4 samples ( 12.1%)
  75-90% busy:     0 samples (  0.0%)
  90-100% busy:    2 samples (  6.0%)

Parallelism Assessment:
  ✗ Poor parallelism (avg 34.8% utilization)
    CPUs are underutilized - significant room for improvement
    Consider:
      - Using parallel test execution
      - Profiling to find sequential bottlenecks
      - Checking if I/O bound rather than CPU bound

  Note: Spent 81.8% of time at <50% CPU utilization
        This suggests periods of sequential execution or I/O waits

========================================
```

## Optimization Workflow

1. **Run validation** with monitoring to get baseline metrics
2. **Identify bottlenecks** using the time distribution data
3. **Apply optimizations**:
   - Increase cargo parallel jobs: `cargo test --jobs N`
   - Enable parallel test execution in test framework
   - Overlap compilation and test phases
4. **Re-run validation** to measure improvement
5. **Iterate** until achieving >75% average utilization

## Notes

- The monitoring process runs in the background and has minimal overhead (<1% CPU)
- Temporary files are automatically cleaned up
- Environment variables are unset after the posthook completes
- Works on Linux systems with standard /proc filesystem
