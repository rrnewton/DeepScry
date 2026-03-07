---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-07T23:00:20.834428483+00:00
---

# Description

## Latest Optimization (2026-03-07_#1876)

✅ **Box MtgError::NeedInput + convert hot-path cards.get()→try_get()** - **+9.5-10.2% actions/sec**
- Boxing NeedInput(ChoiceContext) which contained Option<CardDefinition> (huge struct with Vec/HashMap)
  reduced MtgError enum size, making every Result<T, MtgError> smaller to move/drop
- Converted ~60+ hot-path `.cards.get()` calls to `.cards.try_get()` (returns Option instead of Result)
  eliminating MtgError construction/drop overhead in tight loops
- Perf profiling showed `drop_in_place::<MtgError>` was 7% of CPU before optimization
- robots_mirror: 2,557,826→2,802,017 actions/sec (+9.5%)
- simple_bolt: 5,198,761→5,702,465 actions/sec (+9.7%)
- monoblack: 2,666,430→2,937,489 actions/sec (+10.2%), bytes/game -12.0%

## Previous Optimization (2026-03-07_#1865(cdb8c74))

✅ **Eliminate Vec allocation+clone in targeting callback** - **-3.9% bytes/game (simple_bolt), -4.4% allocation blocks**
- Changed cast_spell_8_step callback from FnMut→FnOnce, Vec<CardId>→SmallVec<[CardId; 2]>
- Eliminated 2 heap allocations per spell cast (Vec creation + Vec::clone)
- SmallVec<[CardId; 4]> for tapped_sources avoids heap for spells costing ≤4 mana
- DHAT: 1,258,103→1,245,431 bytes (-1.0%), 18,006→17,214 blocks (-4.4%)
- simple_bolt: 2,863→2,751 bytes/game (-3.9%), +0.7% actions/sec (p=0.00)
