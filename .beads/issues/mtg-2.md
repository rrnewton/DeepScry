---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-01-03T01:23:13.670542403+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

## Latest Optimization (2026-01-03_#1472)

✅ **Avoid String/Vec allocations in resolve_top_spell_from_stack when not logging** - **2.6% allocation reduction**
- Moved card_name, card_effects, card_owner extraction inside `if should_log` branch
- In Silent mode, uses empty placeholders that are never accessed
- DHAT: 9,968 fewer bytes (-0.86%), 712 fewer blocks (-3.4%)
- Benchmark: Bytes/game 3284.49 → 3200.50 (-2.6%)
- Simple bolt benchmark: 359 MB → 334 MB allocations

## Previous Optimization (2026-01-03_#1469(e189dcb))

✅ **Guard print_battlefield_state with verbosity check** - **3.4% allocation reduction**
- Fixed wasteful allocations in Silent mode by checking verbosity level
- print_battlefield_state() now only executes when verbosity >= Normal
- DHAT: 40,128 fewer bytes (-3.4%), 612 fewer blocks (-2.8%)
- Eliminated hotspot #5: 37.50 KB in 600 blocks completely removed from top 20
- Runtime impact minimal (+0.24% actions/sec) due to small benchmark size
- Bigger impact expected in longer, more complex games with more turn transitions

## Previous Optimization (2025-12-04_#1140(d46eb3d))

✅ **Use SmallVec for spell_targets** - **+1.6% throughput**
- Changed spell_targets from `Vec<(CardId, Vec<CardId>)>` to `Vec<(CardId, SmallVec<[CardId; 2]>)>`
- Most spells have 0-2 targets, so SmallVec stores inline without heap allocation
- Actions/sec: +1.6% (3,319,919 → 3,373,996)
- Bytes/action: -0.1% (3305.81 → 3301.60)
- Note: Modest improvement - most allocations in benchmark are one-time initialization

## Earlier Optimization (2025-12-03_#1117(95ac82cf))

✅ **Eliminate Vec<Effect> clone in resolve_spell** - **2.5% allocation reduction, 5.5% fewer blocks**
- Changed resolve_spell to use index-based iteration instead of cloning entire Vec<Effect>
- Added resolve_effect_target helper for inline target resolution
- DHAT: 22,784 fewer bytes (928,374 → 905,590), 712 fewer blocks (12,936 → 12,224)
- Vec<Effect> clone hotspot (was #13 at 1.5%) no longer appears in top 20

## Earlier Optimization (2025-12-01_#1102(70b3a07))

✅ **Eliminate Vec<Effect> clone in get_valid_targets_for_spell** - **2.4% allocation reduction, ~0.9% speedup**
- Use index-based iteration instead of cloning effects vector
- Extract primitives upfront, re-fetch effects[i] each iteration
- DHAT: 22,784 fewer bytes, 712 fewer blocks (5.2% reduction)
- Benchmark: -0.87% execution time (p=0.04, statistically significant)

## Open Optimization Issues

**Priority 2 (Critical):**
- mtg-0ioei: Reduce WASM rendering frequency when idle (60 FPS → event-driven)
  - Battery/CPU waste: continuous 60 FPS even when game state unchanged
  - Investigate RatZilla manual redraw APIs or dirty-flag pattern
  - See ai_docs/UI_ARCHITECTURE.md for current event architecture

**Other optimization issues:**
- See OPTIMIZATION.md for methodology and allocation reduction patterns
- Continue profiling with DHAT to find allocation hotspots
- Target Vec clones in hot paths (game loop, effect resolution)
