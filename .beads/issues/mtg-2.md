---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-10T00:51:25.960442323+00:00
---

# Description

## Latest Optimization (2026-03-10_#1912)

✅ **Defer card_name clone in check_triggers until matching triggers found** - **-5% to -20% time**
- Moved `card.name.clone()` from outer loop into inner `map()` closure
- Previously cloned Arc<str> for every card on battlefield, even if no triggers match
- Now only clones for cards with actual matching triggers
- Perf profiling: CardName clone/drop overhead reduced from 3.36% to 0.76%
- robots_mirror/32x_par: -15% to -23% time (p = 0.00)
- robots_mirror/mem_logging: -2.6% to -6.5% time (p = 0.00)

## Previous Optimization (2026-03-10_#1911)

✅ **Use CardName (Arc<str>) instead of String in TriggerInfo** - **-3% to -12% time**
- Changed TriggerInfo.card_name from `String` to `CardName` (Arc<str>)
- Eliminated heap allocation from `.to_string()` on every trigger check
- Perf profiling showed `CardName::clone` was 2.56% of CPU, Arc<str>::drop was 1.10%
- simple_bolt: -2.9% to -3.9% time (p = 0.00)
- robots_mirror/32x_par: -10% to -15% time (p = 0.00)

## Previous Optimization (2026-03-10_#1907(931f1c5))

✅ **Add try_get_player() to avoid MtgError allocation on hot paths** - **+14.4% actions/sec**
- Added `try_get_player()` and `try_get_player_mut()` methods returning Option instead of Result
- Converted 4 hot-path usages in priority checking: push_castable_spells, push_castable_from_exile, push_activatable_abilities, push_cycling_abilities
- These functions were using `get_player().map(...).unwrap_or_default()` which allocates MtgError on every call even when discarded
- simple_bolt: 5,963,538→6,821,923 actions/sec (+14.4%)
- robots_mirror/fresh_games: -6.1% time (p = 0.00)
- robots_mirror/mem_logging: -5.0% time (p = 0.00)

## Previous Optimization (2026-03-10_#1903(2220e88))

✅ **Guard debug formatting in priority_round with log_enabled check** - **+6.2% actions/sec, -36% bytes/game**
- Wrapped ability debug logging in `log::log_enabled!(Debug)` check
- Prevents expensive `format!("{:?}", a)` allocations when debug logging disabled
- simple_bolt: 5,617,090→5,963,538 actions/sec (+6.2%)
- Bytes/game: 2,751→1,752 bytes (-36% allocation reduction)
- Criterion: -9.8% to -7.9% time (p = 0.00)

## Previous Optimization (2026-03-10_#1893(cabe142))

✅ **Merge two loops in bounds_check_payment** - **+5-6.6% actions/sec**
- bounds_check_payment was iterating over sources twice: once for available_delta, once for color bounds
- Merged into single loop with identical filtering (is_tapped/has_summoning_sickness check)
- Reduces cache misses by halving iteration count
- simple_bolt: 5,374,109→5,642,275 actions/sec (+5.0%)
- Criterion: -6.6% time (p = 0.00)

## Earlier Optimization (2026-03-07_#1876)

✅ **Box MtgError::NeedInput + convert hot-path cards.get()→try_get()** - **+9.5-10.2% actions/sec**
- Boxing NeedInput(ChoiceContext) which contained Option<CardDefinition> (huge struct with Vec/HashMap)
  reduced MtgError enum size, making every Result<T, MtgError> smaller to move/drop
- Converted ~60+ hot-path `.cards.get()` calls to `.cards.try_get()` (returns Option instead of Result)
  eliminating MtgError construction/drop overhead in tight loops
- Perf profiling showed `drop_in_place::<MtgError>` was 7% of CPU before optimization
- robots_mirror: 2,557,826→2,802,017 actions/sec (+9.5%)
- simple_bolt: 5,198,761→5,702,465 actions/sec (+9.7%)
- monoblack: 2,666,430→2,937,489 actions/sec (+10.2%), bytes/game -12.0%

## Earlier Optimization 2 (2026-03-07_#1865(cdb8c74))

✅ **Eliminate Vec allocation+clone in targeting callback** - **-3.9% bytes/game (simple_bolt), -4.4% allocation blocks**
- Changed cast_spell_8_step callback from FnMut→FnOnce, Vec<CardId>→SmallVec<[CardId; 2]>
- Eliminated 2 heap allocations per spell cast (Vec creation + Vec::clone)
- SmallVec<[CardId; 4]> for tapped_sources avoids heap for spells costing ≤4 mana
- DHAT: 1,258,103→1,245,431 bytes (-1.0%), 18,006→17,214 blocks (-4.4%)
- simple_bolt: 2,863→2,751 bytes/game (-3.9%), +0.7% actions/sec (p=0.00)
