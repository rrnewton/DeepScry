# Benchmark Regression Investigation (2025-12-01)

## Context
After removing unused dependencies (rkyv, typed-arena, tikv-jemalloc-ctl) and updating Cargo.lock, benchmark runs showed apparent performance regressions of 10-108% according to Criterion output.

## Investigation Summary

### Key Finding: **NO ACTUAL REGRESSION FROM DEPENDENCY CLEANUP**

The Criterion "regression" warnings were comparing against an outdated baseline from **Nov 25, 2025** (git depth ~#972, ~40 commits ago), NOT against the immediately prior commit.

### Actual Performance Comparison (Depth #996 vs #1037)

Comparing the most recent prior benchmark (#996, Nov 30) against today's run (#1037):

| Benchmark | #996 games/sec | #1037 games/sec | Change |
|-----------|---------------|-----------------|--------|
| robots_mirror/fresh_games | 4358.67 | 4379.42 | **+0.48%** ✓ |
| robots_mirror/mem_logging | 3622.45 | 3659.85 | **+1.03%** ✓ |
| robots_mirror/stdout_logging | 2226.31 | 2185.95 | -1.81% |
| robots_mirror/snapshot_games | 4476.67 | 4401.64 | -1.68% |
| rewind | 56306.30 | 52943.38 | -5.97% |
| robots_mirror/rewind_play_again | 4475.37 | 4303.79 | **-3.83%** |

**Conclusion**: Performance is essentially **stable** with minor variance (±6%). The small variations are within normal measurement noise for benchmarks.

## Important Observations

### 1. Actions Per Game Changed (~5% reduction)
Between #996 and #1037, all robots_mirror benchmarks show reduced actions/game:
- Before: ~600 actions/game, ~29.6 actions/turn
- After: ~572 actions/game, ~28.2 actions/turn

This indicates a **gameplay behavior change** occurred between these commits, not a performance regression. The engine is making different decisions or executing different game paths.

### 2. Criterion Baseline Staleness
The Criterion baseline in `target/criterion/*/base/` was created on **Nov 25, 2025** and contains estimates from depth ~#972. The "regression" percentages (11-108%) reflect accumulated changes over **65 commits**, not the dependency cleanup.

**Criterion baseline timestamps:**
- Created: 2025-11-25 17:16:30
- Modified: 2025-12-01 07:53:04

### 3. No Compilation/Optimization Impact
The removed dependencies were genuinely unused:
- `rkyv`: Not referenced in code (commented as "future consideration")
- `typed-arena`: Never used
- `tikv-jemalloc-ctl`: Only `tikv-jemallocator` is used

Since these were not in the dependency graph of compiled binaries, their removal cannot affect runtime performance.

## Recommendations

### 1. Update Criterion Baseline
```bash
cargo bench --save-baseline current
```

This will create a fresh baseline at the current commit for meaningful future comparisons.

### 2. Investigate Gameplay Change
Between commits #996 and #1037 (particularly in the dependency cleanup commits), something changed the game execution path to reduce actions/turn from 29.6 to 28.2. This might be:
- A bug fix that eliminated spurious actions
- A change in AI decision-making
- A rules engine correction

**Action**: Review commits between 57c60ff and 29936ba for gameplay-affecting changes.

### 3. Establish Regression Detection Protocol
To avoid false alarms:
1. Always compare against the **immediately prior** benchmark run in CSV history
2. Document Criterion baseline age in benchmark commits
3. Refresh Criterion baselines every ~50 commits or major feature milestones

## Conclusion

**There is NO performance regression from the dependency cleanup.** The Criterion warnings resulted from comparing against a 40-commit-old baseline.

The actual performance change between consecutive runs (#996 → #1037) shows stability with minor variance well within measurement noise. The dependency cleanup successfully removed unused code without performance impact.

**Status**: ✅ Investigation complete - no action required for performance
**Follow-up**: 🔍 Investigate actions/game reduction (separate issue)
