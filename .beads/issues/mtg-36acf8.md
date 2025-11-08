---
title: 'Optimize top 3 CPU hotspots: ManaEngine, spell casting, mana payment'
status: open
priority: 1
issue_type: task
created_at: 2025-11-08T15:35:45.354743062+00:00
updated_at: 2025-11-08T15:35:45.354743062+00:00
---

# Description

## Background

Callgrind CPU profiling (2025-11-08_#831) identified the top 3 hotspots consuming 68% of all CPU instructions:

1. **ManaEngine::update** (30.5% CPU, 5.7B instructions) - mana_engine.rs:254-353
2. **cast_spell_8_step** (23.3% CPU, 4.4B instructions) - actions.rs:640-771
3. **GreedyManaResolver::check_payment** (14.3% CPU, 2.7B instructions) - mana_payment.rs:246-390

Combined with DHAT allocation profiling, we identified the root causes and optimization opportunities.

## Analysis Documents

See comprehensive analysis in:
- ai_docs/cpu_hotspot_analysis_callgrind.md - Detailed CPU hotspot analysis
- ai_docs/optimization_synthesis_cpu_and_allocation.md - Combined CPU + allocation roadmap
- ai_docs/profiling_infrastructure_summary.md - Profiling infrastructure summary
- ai_docs/dhat_allocation_analysis_2025-11-07_#822.md - Current allocation profile

## Key Findings

Cross-Reference: CPU vs Allocations

| System | CPU Hotspot | Allocation Hotspot | Root Cause |
|--------|-------------|-------------------|------------|
| Mana System | ManaEngine::update (30.5%) | mana_engine.rs:550 (27 MB) | Vec growth + string comparisons |
| Spell Casting | cast_spell_8_step (23.3%) | actions.rs (1.71 MB) | Error recovery paths |
| Mana Payment | check_payment (14.3%) | mana_payment.rs:580 (2.57 MB) | Greedy algorithm O(n²) + Vec allocations |

Pattern: These systems are recompute-every-time architectures that repeatedly:
- Scan the battlefield for mana sources
- Allocate temporary Vecs
- Perform O(n²) lookups
- Validate and unwind on errors

## Proposed Approach: Incremental Framework

Instead of the analyzed quick-win optimizations (caching flags, Vec reuse, etc.), explore a more fundamental architectural shift:

### Current Architecture (Recompute-Every-Time)
- ManaEngine::update() called 30+ times per spell cast
- Scans entire battlefield each time
- Allocates fresh Vecs for colors, sources, candidates
- Performs string comparisons and type checks repeatedly

### Proposed Architecture (Incremental Updates)
- Maintain mana availability as game state (not recomputed)
- Update incrementally on battlefield changes (ETB, leaves, tap/untap)
- Cache mana source lists between queries
- Use dirty flags to trigger recomputation only when needed

### Benefits
- CPU: Amortize scanning cost across many queries
- Allocations: Reuse persistent Vecs instead of allocating fresh
- Accuracy: Single source of truth, updated transactionally
- Simplicity: No need for O(n²) greedy resolution if we track available mana directly

### Challenges
- Requires state management (when to invalidate cache)
- More complex undo/rewind logic
- Needs careful testing to ensure correctness

## TODO: Investigation and Implementation

### Phase 1: Investigation (TBD - user input needed)
- [ ] Review user vision for incremental framework
- [ ] Decide between quick-win optimizations vs architectural shift
- [ ] If incremental: design state management approach
- [ ] If quick-wins: proceed with analyzed optimizations from synthesis doc

### Phase 2: Quick Wins (IF we proceed with analyzed approach)

From optimization_synthesis_cpu_and_allocation.md Phase 1:

- [ ] DHAT-1: Pre-size Vec at mana_engine.rs:550 (5 min, 15-20 MB reduction)
- [ ] OPT-7: Reuse candidates Vec in mana payment (30 min, 4.3% CPU + 2 MB)
- [ ] OPT-1: Cache land types in CardCache (1-2 hours, 4.5% CPU)

Expected impact: 11-15% CPU reduction + 20-25 MB allocation reduction

### Phase 3: Medium Impact (IF we proceed with analyzed approach)

- [ ] OPT-8: HashSet for tap_order lookup (1 hour, 5.0% CPU)
- [ ] DHAT-2: Pre-size Vecs in game_loop (15 min, 5-10 MB)
- [ ] DHAT-3: SmallVec for dual land colors (30 min, 8-12 MB)

Expected cumulative impact: 15-16% CPU + 22-32 MB allocation reduction

### Phase 4: Incremental Framework (IF we pursue architectural approach)

- [ ] Design: Sketch incremental mana tracking state structure
- [ ] Design: Define update triggers (ETB, leaves, tap/untap events)
- [ ] Design: Undo/rewind integration
- [ ] Implement: Incremental ManaEngine state
- [ ] Implement: Event-driven updates
- [ ] Implement: Query interface (zero-cost lookups)
- [ ] Test: Validate correctness against old implementation
- [ ] Benchmark: Measure CPU and allocation improvements

Expected impact: TBD (potentially 20-30% CPU + 40-50 MB allocation reduction)

## Measurement Strategy

For any optimization approach:

1. Before: make callgrindprofile and make dhatprofile
2. After: make callgrindprofile and make dhatprofile
3. Validate: Compare metrics, run make validate, check win rate

## References

Profiling Infrastructure (commit #831):
- make callgrindprofile - CPU instruction counting
- make dhatprofile - Heap allocation analysis
- docs/PROFILING_GUIDE.md - Tool comparison

Current Performance (commit #822):
- Allocations: 86.4 MB (down from 1.48 GB after CardCache)
- Throughput: 109.29 games/sec (3.5x speedup from caching)
- Top allocation: mana_engine.rs:550 (27 MB, 31% of total)

Next Session: User will provide direction on incremental framework vs quick-win approach.
