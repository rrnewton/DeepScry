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

## Latest Optimization (2026-04-03_#2062)

✅ **OPT-8: Use precomputed ManaCapacity for O(1) can_pay() on simple sources** - **-2% to -5% time**
- Replaced SimpleManaResolver's O(n) source iteration with O(1) lookup against precomputed simple_capacity
- simple_capacity already has exact untapped counts per color from read_from_cache()
- Eliminates redundant has_complex check + bounds_check_payment iteration on every can_pay() call
- simple_bolt: -4.2% time (p = 0.00), -1.6% time (p = 0.01) across two runs
- whiteweenie_mirror: -3.8% time (p = 0.00)
- jeskai_trolldisk: -2.2% time (p = 0.00)
- robots_mirror/rewind: -1.8% time (p = 0.00)

## Previous Optimizations (2026-03-21_#1966)

✅ **Pre-parsed boolean flags for trigger checking** - **-14.6% time** (da13db67)
- Replaced runtime .starts_with("[controller_only]") and .contains("[noncreature]") with pre-parsed
  boolean flags (controller_turn_only, requires_noncreature) on Trigger struct
- robots_mirror/mem_logging: -14.6% time (p = 0.00) - largest single optimization win
- Massive because robots deck has many creatures with upkeep/combat triggers

✅ **Empty mana pool fast-path** - **-2.9% time** (0771ab02)
- Skip pool calculations when mana pool is empty (common case - no Dark Ritual)
- robots_mirror/mem_logging: -2.9% time (p = 0.00)

✅ **SmallVec for SBA collections** - **-1.9% time** (40d375aa)
- check_aura_attachment and check_lethal_damage use SmallVec instead of Vec
- simple_bolt: -1.9% time (p = 0.00)

## Previous Optimization (2026-03-14_#1937(3a67f89))

✅ **sort_unstable + SBA debug logging guard** - **-5% to -7.5% time**
- Used sort_unstable_by_key for abilities_buffer (avoids allocation overhead of stable sort)
- Guarded SBA debug logging with log_enabled! check (avoid evaluating conditions when debug off)
- Removed unnecessary card.name.contains("Peter Porker") string search in SBA hot path
- robots_mirror/mem_logging: -5.8% to -9.2% time (p = 0.00)
- simple_bolt: -4.5% to -5.3% time (p = 0.00)
- Tracked: mem_logging 2,265,401 actions/sec, simple_bolt 7,045,777 actions/sec

## Previous Optimization (2026-03-14_#1928(87675a5))

✅ **Eliminate inner Vec allocation in check_triggers** - **-2% to -4.6% time**
- Replaced filter_map + collect::<Vec<TriggerInfo>> + flatten with filter_map + flat_map
- Eliminated intermediate Vec allocation for each card on battlefield
- Most cards have zero matching triggers, so Vecs were allocated empty and dropped
- robots_mirror/mem_logging: -3.2% to -4.6% time (p = 0.00)
- simple_bolt: -2.0% time (p = 0.00)
- Tracked: mem_logging 2,235,556 actions/sec, simple_bolt 6,654,001 actions/sec

## Previous Optimization (2026-03-10_#1912)

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
