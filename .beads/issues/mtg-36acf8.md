---
title: 'Optimize top 3 CPU hotspots: ManaEngine, spell casting, mana payment'
status: open
priority: 1
issue_type: task
created_at: 2025-11-08T15:35:45.354743062+00:00
updated_at: 2025-11-08T15:51:14.645333609+00:00
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
- **ai_docs/java_forge_architecture_comparison.md - Java Forge comparison (NEW 2025-11-08)**

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

## Java Forge Comparison (2025-11-08)

**CRITICAL FINDING:** Java Forge uses the **SAME recompute-every-time architecture**\!

### What Java Does (NOT) Do

**NO incremental caching:**
-  rescans battlefield every call (30+ times per spell)
-  rebuilds multimap every time
- No event listeners for battlefield changes
- No cached mana source lists

**ONLY caches:**
- Floating mana in ManaPool (mana already produced)
- NOT potential mana from untapped sources

### Why Java Feels Faster

Despite same O(n) scan pattern:
1. JIT optimization inlines hot paths
2. GC amortizes allocation cost
3. 15+ years of heuristic tuning
4. Looser correctness validation

**BUT:** We're already competitive (109 games/sec Rust vs ~50-80 Java).

### What We Learned

1. **Incremental architecture is NOVEL** - neither codebase does it
2. **Our 8-step structure is BETTER** - more explicit than Java's 300-line scattered logic
3. **Java proves naive approach works** - incremental is pure upside
4. **We have greenfield advantage** - can build what Java can't retrofit

See full comparison in .

## Proposed Approach: Incremental Framework (RECOMMENDED)

Based on Java analysis, we should pursue incremental updates as our PRIMARY optimization:

### Why Incremental Is The Right Path

1. **Novel optimization** - neither Java nor current Rust does this
2. **Addresses root cause** - amortizes O(n) scan across queries
3. **Safe fallback** - can recompute on uncertainty
4. **Better architecture** - explicit state management vs ad-hoc rescanning

### Proposed Design

```rust
pub struct ManaTracker {
    mana_sources: Vec<ObjectId>,           // Precomputed available sources
    source_colors: HashMap<ObjectId, ColorSet>,  // Precomputed color production
    dirty: bool,                            // Invalidation flag
    battlefield_version: u64,               // Change detection
}

impl ManaTracker {
    pub fn available_mana(&mut self, game: &GameState) -> &[ObjectId] {
        if self.dirty || self.battlefield_version \!= game.battlefield_version {
            self.recompute(game);  // Only when needed\!
        }
        &self.mana_sources
    }

    pub fn mark_dirty(&mut self) { self.dirty = true; }

    fn recompute(&mut self, game: &GameState) {
        // Current ManaEngine::update logic
        // But called ONCE per battlefield change, not 30+ times per spell
        // ...
    }
}
```

### Event Hooks

```rust
impl GameState {
    fn on_card_enters_battlefield(&mut self, card: ObjectId) {
        self.mana_tracker.mark_dirty();
    }

    fn on_card_leaves_battlefield(&mut self, card: ObjectId) {
        self.mana_tracker.mark_dirty();
    }

    fn on_card_tapped(&mut self, card: ObjectId) {
        if self.mana_tracker.is_mana_source(card) {
            self.mana_tracker.mark_dirty();
        }
    }
}
```

### Benefits vs Costs

**Benefits:**
- Amortize O(n) scan: 1x per battlefield change instead of 30x per spell
- Expected: 20-30% CPU reduction + 40-50 MB allocation reduction
- Foundation for other incremental queries (castability, creature counts, etc.)

**Costs:**
- Must invalidate on ALL relevant events (ETB, leaves, tap, untap, abilities gained/lost)
- More complex undo/rewind logic
- Needs extensive correctness testing

**Risk Mitigation:**
- Start with just mana source caching
- Compare vs naive recompute in tests
- Fall back to recompute on any uncertainty

## Alternative: Quick-Win Optimizations (Lower Risk)

If incremental seems too ambitious, proceed with analyzed quick wins:

### Phase 1: Quick Wins (5-15% improvement)

From optimization_synthesis_cpu_and_allocation.md:

- [ ] DHAT-1: Pre-size Vec at mana_engine.rs:550 (5 min, 15-20 MB reduction)
- [ ] OPT-7: Reuse candidates Vec in mana payment (30 min, 4.3% CPU + 2 MB)
- [ ] OPT-1: Cache land types in CardCache (1-2 hours, 4.5% CPU)

Expected impact: 11-15% CPU reduction + 20-25 MB allocation reduction

### Phase 2: Medium Impact (additional 5-10%)

- [ ] OPT-8: HashSet for tap_order lookup (1 hour, 5.0% CPU)
- [ ] DHAT-2: Pre-size Vecs in game_loop (15 min, 5-10 MB)
- [ ] DHAT-3: SmallVec for dual land colors (30 min, 8-12 MB)

Expected cumulative impact: 15-20% CPU + 22-32 MB allocation reduction

## Decision Point: User Input Needed

**Question:** Which path should we pursue?

**Option A (RECOMMENDED): Incremental Framework**
- Higher risk, higher reward
- Novel optimization not in Java
- Addresses root cause
- 20-30% CPU + 40-50 MB potential
- Prototype on branch, validate with tests

**Option B: Quick-Win First, Then Decide**
- Lower risk
- 15-20% improvement proven achievable
- Learn where remaining bottlenecks are
- Revisit incremental after data

**Option C: Hybrid**
- Start with DHAT-1, OPT-7 (30 min work)
- Prototype incremental in parallel on branch
- Benchmark both approaches
- Pick winner based on data

## Measurement Strategy

For any optimization approach:

1. **Before:** make callgrindprofile && make dhatprofile
2. **After:** make callgrindprofile && make dhatprofile  
3. **Validate:** Compare metrics, run make validate, check win rate

## References

Profiling Infrastructure (commit #831):
- make callgrindprofile - CPU instruction counting
- make dhatprofile - Heap allocation analysis
- docs/PROFILING_GUIDE.md - Tool comparison

Current Performance (commit #822):
- Allocations: 86.4 MB (down from 1.48 GB after CardCache)
- Throughput: 109.29 games/sec (3.5x speedup from caching)
- Top allocation: mana_engine.rs:550 (27 MB, 31% of total)

Java Forge Performance (estimated):
- Throughput: ~50-80 games/sec (rough estimate, not directly measured)
- Architecture: Same recompute-every-time pattern
- Advantage: JIT + GC + mature heuristics (~10-50% faster, not 10x)

---

**Status:** Awaiting user decision on optimization path (A/B/C).

**Next Session:** Implement chosen approach with before/after profiling.
