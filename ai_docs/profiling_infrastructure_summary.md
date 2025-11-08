# Profiling Infrastructure Summary

**Date**: 2025-11-08_#831
**Session Goal**: Regularize DHAT and perf benchmarking, add container-friendly CPU profiling

## What Was Accomplished

### 1. DHAT Profiling (Allocation Analysis)

**Before**: Old scripts scattered, manual analysis required
**After**: Integrated Makefile target with automatic analysis

```bash
make dhatprofile
```

**What it does**:
- Runs `dhat_profile` benchmark (100 game replays)
- Generates `experiment_results/dhat-heap.json`
- Automatically analyzes with `scripts/analyze_dhat.py`
- Shows top 20 allocation sites with human-readable summary
- Duration: ~10-15 seconds

**Key changes**:
- Fixed paths in `mtg-benchmarks/benches/dhat_profile.rs` (../cardsfolder, ../decks)
- Updated Makefile target to run analysis script automatically
- Added clear instructions for next steps (view in dh_view.html)

### 2. Linux Perf Profiling (Hardware Performance Counters)

**Status**: Documented but not working in containers

```bash
make perfprofile
```

**What it does**:
- Attempts to run perf with call-graph recording
- Fails gracefully with helpful error message
- Documents workarounds for container environments

**Problem**: Containers lack CAP_PERFMON and CAP_SYS_ADMIN capabilities

**Documentation**: `docs/PERF_PROFILING_PODMAN.md`
- Explains permission issues
- Provides Podman run command with required capabilities
- Recommends alternative profilers

### 3. Callgrind Profiling (CPU Instruction Counting) ⭐

**New**: Container-friendly CPU profiling fallback

```bash
make callgrindprofile
```

**What it does**:
- Runs Valgrind Callgrind on `rewind_bench` (250 games)
- Generates `experiment_results/callgrind.out`
- Automatically analyzes with `callgrind_annotate`
- Shows top 30 CPU hotspots by instruction count
- Duration: 1-2 minutes (~50x slowdown due to instrumentation)

**Key features**:
- Works in containers (no special permissions needed)
- Deterministic instruction counting (not sampling)
- Full call graph analysis
- Source-level annotation support
- KCachegrind visualization support (if available on host)

**Configuration**:
- 250 games (reduced from 500 for faster iteration)
- Sequential mode (single-threaded)
- Full instrumentation: `--dump-instr=yes --collect-jumps=yes --cache-sim=yes`

### 4. Comprehensive Documentation

Created three documentation files:

1. **`docs/PERF_PROFILING_PODMAN.md`**: Perf permission issues and workarounds
2. **`docs/PROFILING_GUIDE.md`**: Tool comparison matrix and usage guide
3. **`ai_docs/cpu_hotspot_analysis_callgrind.md`**: Detailed analysis of top 3 CPU hotspots

## Profiling Tool Comparison

| Tool | What It Measures | Container Support | Speed | Output |
|------|------------------|-------------------|-------|--------|
| **DHAT** | Heap allocations | ✅ Yes | Fast (1x) | `dhat-heap.json` |
| **Callgrind** | CPU instructions | ✅ Yes | Slow (50x) | `callgrind.out` |
| **Perf** | Hardware counters | ❌ No (requires caps) | Fast (1.1x) | `perf.data` |
| **Flamegraph** | CPU time (sampling) | ⚠️ Maybe | Medium (2-3x) | `flamegraph.svg` |

## Analysis Results

### Current Performance (Post-CardCache Optimization)

From commit #822 (855b05d5):

**Allocation Profile (DHAT)**:
- Total allocations: 86.4 MB (down from 1.48 GB - 94.2% reduction!)
- Total blocks: 5.6M
- Top hotspot: `mana_engine.rs:550` (27 MB, 31% of total)
- Throughput: 109.29 games/sec

**CPU Profile (Callgrind)**:
- Total instructions: 18.8 billion (250 games)
- Top hotspot: `ManaEngine::update` (5.7B, 30.5%)
- Second: `cast_spell_8_step` (4.4B, 23.3%)
- Third: `check_payment` (2.7B, 14.3%)

### Cross-Reference Findings

**Key insight**: The top CPU hotspots and top allocation sites are **related but different**.

| System | CPU Hotspot | Allocation Hotspot | Root Cause |
|--------|-------------|-------------------|------------|
| Mana System | `ManaEngine::update` (30.5%) | `mana_engine.rs:550` (27 MB) | Vec growth for dual land colors |
| Spell Casting | `cast_spell_8_step` (23.3%) | `actions.rs` (1.71 MB) | Error recovery paths |
| Mana Payment | `check_payment` (14.3%) | `mana_payment.rs:580` (2.57 MB) | Greedy algorithm with Vec allocations |

**Pattern**: High CPU usage correlates with Vec operations (growth, iteration, O(n²) lookups).

## Optimization Roadmap

Based on combined CPU + allocation analysis, we identified **9 specific optimizations** grouped into 3 phases:

### Phase 1: Quick Wins (~4 hours work)
- **Total impact**: 11-15% CPU reduction + 20-25 MB allocation reduction

1. **DHAT-1**: Pre-size Vec at `mana_engine.rs:550` (5 min)
   - Impact: 0.5-1% CPU, 15-20 MB allocations

2. **OPT-7**: Reuse candidates Vec in mana payment (30 min)
   - Impact: 4.3% CPU, ~2 MB allocations

3. **OPT-1**: Cache land types in CardCache (1-2 hours)
   - Impact: 4.5% CPU, minimal allocations

### Phase 2: Medium Impact (~3 hours work)
- **Total impact**: 5-8% CPU reduction + 10-15 MB allocation reduction

4. **OPT-8**: HashSet for tap_order lookup (1 hour) ⭐ Highest single CPU impact!
   - Impact: 5.0% CPU, minimal allocations

5. **DHAT-2**: Pre-size Vecs in game_loop (15 min)
   - Impact: 0.5-1% CPU, 5-10 MB allocations

6. **DHAT-3**: SmallVec for dual land colors (30 min)
   - Impact: 0.5-1% CPU, 8-12 MB allocations

### Phase 3: Structural Changes (~6 hours work)
- **Total impact**: 2-4% CPU reduction + 5-10 MB allocation reduction

7-9. Various caching and pooling optimizations

**See `ai_docs/optimization_synthesis_cpu_and_allocation.md` for full details.**

## Files Modified

### Makefile
- Updated `dhatprofile` target to run analysis automatically
- Updated `perfprofile` target with better error handling
- Added `callgrindprofile` target for container-friendly CPU profiling

### mtg-benchmarks/benches/dhat_profile.rs
- Fixed paths: `cardsfolder` → `../cardsfolder`, `decks/simple_bolt.dck` → `../decks/simple_bolt.dck`

### .gitignore
- Added `experiment_results/callgrind.out` to ignore binary profiling output

## How to Use the Profiling Infrastructure

### Quick Start

1. **Allocation profiling** (what allocates memory):
   ```bash
   make dhatprofile
   ```
   View results: Open https://nnethercote.github.io/dh_view/dh_view.html and load `experiment_results/dhat-heap.json`

2. **CPU profiling** (what takes CPU time):
   ```bash
   make callgrindprofile
   ```
   View results: `callgrind_annotate experiment_results/callgrind.out | less`

3. **Both at once** (for optimization work):
   ```bash
   make dhatprofile > before_alloc.txt
   make callgrindprofile > before_cpu.txt
   # ... make your changes ...
   make dhatprofile > after_alloc.txt
   make callgrindprofile > after_cpu.txt
   diff before_alloc.txt after_alloc.txt
   diff before_cpu.txt after_cpu.txt
   ```

### Deep Dive Analysis

**For allocation profiling**:
```bash
# Re-analyze existing DHAT data
python3 scripts/analyze_dhat.py

# Create dated analysis document
python3 scripts/analyze_dhat.py > ai_docs/dhat_analysis_$(date +%Y-%m-%d)_#$(git rev-list --count HEAD).md
```

**For CPU profiling**:
```bash
# Function-level analysis
callgrind_annotate --auto=yes experiment_results/callgrind.out | less

# Source-level annotation
callgrind_annotate --auto=yes experiment_results/callgrind.out mtg-engine/src/game/mana_engine.rs

# Interactive visualization (requires KCachegrind on host)
kcachegrind experiment_results/callgrind.out

# Call graph analysis
callgrind_annotate --tree=both experiment_results/callgrind.out | less
```

## Next Steps

### Immediate (Today)
1. Implement DHAT-1: Pre-size Vec at `mana_engine.rs:550` (5 min, 15-20 MB impact)
2. Implement OPT-7: Reuse candidates Vec (30 min, 4.3% CPU + 2 MB)
3. Run benchmarks to validate impact

### This Week
4. Implement OPT-1: Cache land types in CardCache (1-2 hours, 4.5% CPU)
5. Implement OPT-8: HashSet for tap_order lookup (1 hour, 5.0% CPU) ⭐
6. Implement DHAT-2: Pre-size Vecs in game_loop (15 min, 5-10 MB)

**Expected cumulative impact**: 15-16% CPU reduction + 22-32 MB allocation reduction

## Validation Checklist

For each optimization:
- [ ] Run `make callgrindprofile` before and after
- [ ] Run `make dhatprofile` before and after
- [ ] Compare instruction counts and allocation totals
- [ ] Run `make validate` to ensure correctness
- [ ] Check win rate determinism (should stay ~63.1%)
- [ ] Document results in tracking issue

## Related Documents

- `docs/PROFILING_GUIDE.md`: Tool comparison and when to use each profiler
- `docs/PERF_PROFILING_PODMAN.md`: Perf permission issues in containers
- `ai_docs/cpu_hotspot_analysis_callgrind.md`: Detailed CPU hotspot analysis
- `ai_docs/dhat_allocation_analysis_2025-11-07_#822.md`: Post-CardCache allocation analysis
- `ai_docs/optimization_synthesis_cpu_and_allocation.md`: Combined CPU + allocation optimization roadmap

## Conclusion

We now have a **robust, container-friendly profiling infrastructure** with:
- ✅ Allocation profiling (DHAT)
- ✅ CPU profiling (Callgrind)
- ✅ Automatic analysis and human-readable output
- ✅ Comprehensive documentation
- ✅ Clear optimization roadmap

The infrastructure is ready for development iteration:
1. Profile before changes
2. Implement optimization
3. Profile after changes
4. Validate correctness
5. Document results

**Total time invested**: ~4 hours
**Expected return**: Enables 15-20% CPU improvements and 50% allocation reduction (86 MB → ~40-50 MB)
