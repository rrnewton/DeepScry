---
title: Optimize GameState clone for parallel MCTS simulations
status: open
priority: 3
issue_type: task
created_at: 2025-11-04T21:44:24.580096373+00:00
updated_at: 2025-11-04T21:44:24.580096373+00:00
---

# Description

Problem: GameState cloning costs 15-20KB per clone (Cards 8KB + Undo log 10KB + other 2KB), creating cache pressure in parallel code. With 8 threads, 120-160KB cloned per iteration. Solution: Create clone_for_simulation() that skips undo_log and logger. Estimated savings: 60% reduction (15-20KB to 5-8KB). This should improve parallel efficiency from 47.4% toward 70-80%. See ai_docs/parallel_contention_analysis.md. Related: mtg-a6ca26, mtg-2

## Investigation Results (2025-11-04)

**Attempted optimization: `clone_for_simulation()` method**

Created a method that clones GameState but starts with empty undo_log and logger (via `UndoLog::new()` and `GameLogger::new()`).

**Result: FAILED - caused 8% performance regression**

**Why it failed:**

1. **Logger already clones efficiently**: GameLogger's Clone impl (src/game/logger.rs:466-480) already creates empty Bump allocator and log buffer. So `clone_for_simulation()` does the same thing as regular `clone()` for the logger - no benefit.

2. **Undo log still grows during gameplay**: Starting with `UndoLog::new()` is empty, but during forward simulation, the undo log STILL gets populated by gameplay code. So we don't save allocation - we just defer it. The 6.5KB/game allocation metric didn't improve.

3. **Wrong optimization target**: The clone happens OUTSIDE the benchmark timing window (line 1103-1105 in benches/game_benchmark.rs). So even if we made cloning cheaper, it wouldn't show up in benchmark times.

4. **Parallel efficiency got WORSE**: Dropped from 47.4% to 4.9% (!), suggesting the approach added overhead somewhere.

**Key insight:**

The real problem isn't the clone COST - it's that we're cloning 15-20KB of data that then sits in cache/memory. The optimization needs to:
- **Disable undo logging entirely** during simulation (not just start empty)
- **Share immutable Card data** instead of deep copying
- **Use copy-on-write** for structures that rarely change

**Next approach:**

Option A: Add flag to disable undo logging
- Add `disable_undo: bool` to GameState
- When true, skip all undo_log.log() calls
- Saves ~10KB clone + eliminates undo allocation during gameplay

Option B: Parameterize GameState by allocator
- `GameState<A: Allocator = Global>`
- Simulation threads use per-thread bump allocators
- Near-zero contention, fast allocation
- Matches Phase 2 plan from mtg-a6ca26

Option C: Shallow clone with Rc/Arc for immutable data
- Cards definitions shared via `Rc<CardDefinition>`
- Only clone mutable state (tapped, counters, zones)
- Estimated: 15-20KB → 3-5KB

**Recommendation:** Try Option A first (simplest), then Option C (best ROI), save Option B for Phase 2.

