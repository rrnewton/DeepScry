# MTG Forge-rs Profiling Guide

This guide explains the various profiling tools available for analyzing performance in the MTG Forge-rs project.

## Quick Reference

| Tool | Target | Works in Containers | Overhead | Use Case |
|------|--------|---------------------|----------|----------|
| **`make dhatprofile`** | Allocations | ✅ Yes | Low (~2-3x) | Find allocation hotspots |
| **`make callgrindprofile`** | CPU | ✅ Yes | High (~50x) | CPU instruction analysis |
| **`make perfprofile`** | CPU + Cache | ❌ No | Low (~2%) | Detailed CPU profiling |
| **`make profile`** | CPU | Varies | Medium (~10x) | Visual flamegraphs |
| **`make heapprofile`** | Allocations | ✅ Yes | Medium (~20x) | Alternative heap profiling |

## Recommended Workflow

### 1. **Start with DHAT** (Allocation Profiling)
```bash
make dhatprofile
```

**What it does:**
- Profiles heap allocations with full Rust symbol information
- Runs 100 iterations of rewind+replay (takes ~10-20 seconds)
- Shows top 20 allocation sites immediately
- Outputs `experiment_results/dhat-heap.json`

**When to use:**
- Reducing memory allocations
- Finding allocation hotspots
- Optimizing memory-heavy code
- First step in performance optimization

**Output example:**
```
=== TOP 20 ALLOCATION SITES BY TOTAL BYTES ===
#1: 9.75 MB (11.8%) in 1,278,093 blocks (avg 8.0 bytes/block)
  Location: mtg_forge_rs::game::mana_engine::get_complex_mana_production (src/game/mana_engine.rs:550:20)
```

**Key metrics:**
- **Total bytes**: How much memory was allocated
- **Blocks**: Number of allocation calls
- **Avg size**: Typical allocation size (helps identify Vec growth vs fixed allocations)

### 2. **Use Callgrind** (CPU Profiling in Containers)
```bash
make callgrindprofile
```

**What it does:**
- Profiles CPU instruction counts and call graphs using Valgrind
- Runs 250 games (takes 1-2 minutes due to ~50x slowdown)
- Works perfectly in containers without special permissions
- Collects instruction counts, cache statistics, and call graphs
- Outputs `experiment_results/callgrind.out`

**When to use:**
- Identifying CPU hotspots
- Understanding call graph structure
- Cache behavior analysis (L1/LL miss rates)
- When you can't use perf (containerized environments)

**Output example:**
```
=== Top 30 CPU Hotspots (by instruction count) ===
Ir                      file:function
5,701,666,144 (30.46%)  mtg_forge_rs::game::mana_engine::ManaEngine::update
4,358,477,547 (23.28%)  mtg_forge_rs::game::actions::cast_spell_8_step
2,667,150,711 (14.25%)  mtg_forge_rs::game::mana_payment::check_payment
```

**Key metrics:**
- **Ir** (Instruction Reads): Total CPU instructions executed
- **Dr/Dw** (Data Reads/Writes): Memory access patterns
- **I1mr/D1mr** (L1 cache misses): Cache efficiency
- **LLmr** (Last Level cache misses): Main memory accesses

**Cache statistics:**
```
I1 miss rate: 0.61%    (instruction cache - very good if <3%)
D1 miss rate: 1.1%     (data cache - good if <5%)
LL miss rate: 0.0%     (last level cache - excellent if <1%)
```

### 3. **Use Perf** (When Available on Host)
```bash
make perfprofile
```

**What it does:**
- Profiles CPU with hardware performance counters
- Low overhead (~2%) - suitable for larger sample sizes
- Requires Linux perf and special permissions
- Falls back gracefully in containers (see `docs/PERF_PROFILING_PODMAN.md`)

**When to use:**
- On host systems (not in containers)
- Need accurate timing (not just instruction counts)
- Want hardware-level cache/branch prediction stats
- Profiling production-like workloads

**To enable in containers:**
```bash
podman run --cap-add=CAP_PERFMON --cap-add=CAP_SYS_ADMIN \
           --security-opt seccomp=unconfined \
           your-container
```

## Detailed Tool Comparison

### DHAT (Recommended for Allocations)

**Pros:**
- ✅ Full Rust symbol information
- ✅ Works in containers without permissions
- ✅ Low overhead (2-3x slowdown)
- ✅ Immediate human-readable output
- ✅ Interactive visualization via dh_view.html

**Cons:**
- ❌ Only tracks allocations (not CPU)
- ❌ Doesn't show deallocation timing

**Best for:**
- Reducing `Vec` growth allocations
- Finding `String::to_lowercase()` hotspots
- Optimizing memory-heavy algorithms
- Pre-sizing `Vec::with_capacity()`

### Callgrind (Recommended for CPU in Containers)

**Pros:**
- ✅ Works in containers without permissions
- ✅ Deterministic (instruction counts, not wall time)
- ✅ Detailed cache statistics (L1/LL miss rates)
- ✅ Full call graph with caller/callee relationships
- ✅ Source-level annotation support
- ✅ KCachegrind visualization

**Cons:**
- ❌ Very high overhead (50x slowdown)
- ❌ Small sample sizes due to slowness
- ❌ Instruction counts ≠ wall time (branch prediction, pipelining not modeled)

**Best for:**
- Finding CPU-intensive functions
- Understanding call graphs
- Cache behavior analysis
- When perf isn't available

**Advanced usage:**
```bash
# Annotate a specific source file
callgrind_annotate --auto=yes experiment_results/callgrind.out \
    mtg-engine/src/game/mana_engine.rs

# Show call tree
callgrind_annotate --tree=both experiment_results/callgrind.out | less

# Visualize with KCachegrind (on host with GUI)
kcachegrind experiment_results/callgrind.out
```

### Perf (Recommended for CPU on Host)

**Pros:**
- ✅ Very low overhead (2% typical)
- ✅ Hardware performance counters (accurate)
- ✅ Large sample sizes possible
- ✅ Cache miss analysis
- ✅ Branch prediction statistics

**Cons:**
- ❌ Requires special permissions/capabilities
- ❌ Doesn't work in standard containers
- ❌ Needs debug symbols for good output
- ❌ Sampling-based (not deterministic)

**Best for:**
- Production-like workload profiling
- Accurate wall-time CPU attribution
- Hardware-level cache analysis
- When you have host access

### Flamegraph (`make profile`)

**Pros:**
- ✅ Beautiful visual representation
- ✅ Easy to understand call stacks
- ✅ Interactive SVG output

**Cons:**
- ❌ Requires cargo-flamegraph installation
- ❌ Similar to perf (may have permission issues)
- ❌ Less detailed than callgrind

**Best for:**
- Presenting results to others
- Quick visual overview
- Identifying call stack patterns

## Interpreting Results

### DHAT Allocation Analysis

**High total bytes (>1MB per hotspot):**
- Look for `Vec` growth (avg size ~8-32 bytes)
- Consider `Vec::with_capacity()` pre-sizing
- Consider `SmallVec` for small collections

**Many small blocks (avg <32 bytes):**
- Likely `Vec` growth from push()
- Use `Vec::with_capacity()` or `Vec::reserve()`

**Few large blocks (avg >1KB):**
- String allocations
- Large Vec allocations
- Consider caching or pooling

### Callgrind CPU Analysis

**High instruction count (>10% of total):**
- Critical hot path
- Prime optimization target
- Profile cache behavior

**High D1 miss rate (>5%):**
- Poor data locality
- Consider data structure reorganization
- May benefit from prefetching

**High LL miss rate (>1%):**
- Main memory bottleneck
- Reduce working set size
- Improve cache locality

## Optimization Workflow

1. **Baseline measurements:**
   ```bash
   make dhatprofile       # Measure allocations
   make callgrindprofile  # Measure CPU
   ```

2. **Identify top hotspots** (usually top 3-5 functions account for 50%+ of cost)

3. **Implement optimization** (one at a time)

4. **Re-profile to verify:**
   ```bash
   make dhatprofile       # Check allocation reduction
   make callgrindprofile  # Check CPU improvement
   ```

5. **Document results** in commit message with before/after metrics

## Example Optimization Session

### Step 1: Profile baseline
```bash
make dhatprofile
```
Output:
```
#1: 9.75 MB (11.8%) in 1,278,093 blocks (avg 8.0 bytes/block)
  Location: mana_engine.rs:550 - Vec::new() followed by push()
```

### Step 2: Identify issue
- Avg 8 bytes/block suggests `Vec` growth
- 1.27M allocations = lots of small Vec growth
- Line 550: `let mut colors = Vec::new();`

### Step 3: Fix
```rust
// Before
let mut colors = Vec::new();

// After
let mut colors = Vec::with_capacity(5);  // Dual lands have ≤2, but allow headroom
```

### Step 4: Verify
```bash
make dhatprofile
```
Expected: Allocation count reduced by ~50-70%

## Profiling Best Practices

1. **Always profile in release mode** (`--release` flag)
   - Debug builds have 10-100x different performance characteristics
   - All `make *profile` targets use release builds

2. **Use representative workloads**
   - Our benchmarks use `rewind_bench` with robots mirror match
   - 250-5000 games depending on profiler overhead
   - Consistent seed (43) for reproducibility

3. **Profile one thing at a time**
   - Change one function at a time
   - Re-profile after each change
   - Avoid premature optimization

4. **Document your findings**
   - Save profiler output to `experiment_results/`
   - Create analysis documents with timestamps (use `gitdepth.sh`)
   - Include before/after metrics in commit messages

5. **Focus on the top hotspots**
   - 80/20 rule: top 20% of functions account for 80% of cost
   - Optimizing anything below 1% total is usually not worth it
   - Exception: If it's called millions of times, even 0.1% matters

## Container-Friendly Summary

For containerized development environments (Podman/Docker):

**Works without special permissions:**
- ✅ `make dhatprofile` - Allocation profiling
- ✅ `make callgrindprofile` - CPU profiling
- ✅ `make heapprofile` - Alternative allocation profiling

**Requires special permissions/capabilities:**
- ❌ `make perfprofile` - Needs `CAP_PERFMON`, `CAP_SYS_ADMIN`
- ❌ `make profile` - May need elevated permissions (depends on system)

**Recommendation:** Use `dhatprofile` and `callgrindprofile` for containerized development. They provide excellent insight without requiring privileged containers.

## See Also

- `docs/PERF_PROFILING_PODMAN.md` - Detailed perf setup for containers
- `scripts/analyze_dhat.py` - DHAT analysis script
- `OPTIMIZATION.md` - Optimization principles and patterns
- `experiment_results/` - Historical profiling data

## References

- [DHAT: Dynamic Heap Analysis Tool](https://valgrind.org/docs/manual/dh-manual.html)
- [Valgrind Callgrind](https://valgrind.org/docs/manual/cl-manual.html)
- [Linux perf](https://perf.wiki.kernel.org/index.php/Main_Page)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
