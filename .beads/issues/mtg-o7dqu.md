---
title: Implement proper SVar parsing and StaticAbility support
status: open
priority: 2
issue_type: feature
created_at: 2026-01-02T19:12:02.059359190+00:00
updated_at: 2026-01-02T19:27:03.086235391+00:00
---

# Description

## Description

Implement full SVar parsing infrastructure and StaticAbility support for Avatar deck cards.

## Background

SVars (Script Variables) are a core Java Forge mechanism for:
1. Defining reusable ability definitions (DB$, AB$ blocks)
2. Defining static abilities (Mode$ CantBlockBy, Mode$ Continuous, etc.)
3. Holding computed values (X variables, counters, etc.)
4. Chaining SubAbility effects

Currently we store SVars as raw strings in `Card.svars` HashMap but don't properly parse them.

## Avatar Deck Requirements

Based on analysis of ryan_avatar_draft.dck and gabriel_avatar_draft.dck:

### StaticAbility Mode$ Types Needed

1. **Mode$ CantBlockBy** - Blocking restrictions (Deserter's Disciple) ✅ IMPLEMENTED
   - ValidAttacker$ Card.IsRemembered
   - Properly parsed via svar_parser.rs

2. **Mode$ Continuous** - Continuous effects (Fire Lord Ozai)
   - Affected$ Card.IsRemembered | AffectedZone$ Exile
   - MayPlay$ True | MayPlayWithoutManaCost$ True

3. **Mode$ Attacks** - Attack triggers
4. **Mode$ ChangesZone** - ETB and zone change triggers
5. **Mode$ Phase** - Phase-based triggers
6. **Mode$ SpellCast** - Spell cast triggers (Fire Lord Ozai Play1)
7. **Mode$ LandPlayed** - Land play triggers (Fire Lord Ozai Play2)
8. **Mode$ Sacrificed** - Sacrifice triggers

### Common DB$ Effect Types in SVars

- DB$ Pump (with KW$ for keyword grants)
- DB$ GainLife
- DB$ PutCounter
- DB$ Token
- DB$ DealDamage
- DB$ Destroy
- DB$ ChangeZone
- DB$ Cleanup (state management)
- DB$ Dig
- DB$ Scry
- DB$ Effect (nested effect creation)
- DB$ Charm (multi-choice)

### RememberObjects Pattern

Many cards use the RememberObjects/IsRemembered pattern:
1. AB$ Effect creates effect with RememberObjects$ Targeted
2. StaticAbilities$ reference SVars with ValidAttacker$ Card.IsRemembered
3. DB$ Cleanup clears ClearRemembered$ at end

## Implementation Plan

### Phase 1: SVar Parser Infrastructure ✅ COMPLETED

- [x] Create `svar_parser.rs` module with parsing infrastructure
- [x] Create `ParsedSVar` enum to represent parsed SVars (StaticAbility, Effect, BooleanFlag, ComputedValue, Raw)
- [x] Create `StaticAbilityDef` struct for Mode$ definitions with params HashMap
- [x] Create `StaticAbilityMode` enum for all 8+ mode types
- [x] Create `EffectDef` struct for DB$/AB$ definitions
- [x] Parse SVar strings with `parse_svar()` and `parse_all_svars()` functions
- [x] Export types via loader/mod.rs

### Phase 2: Effect Conversion Integration ✅ COMPLETED

- [x] Add `params_to_effect_with_svars()` for SVar-aware effect conversion
- [x] Properly resolve StaticAbilities$ references to SVar definitions
- [x] Determine effect type based on Mode$ (e.g., CantBlockBy -> GrantCantBeBlocked)
- [x] Fall back to name-based heuristics when SVar lookup fails

### Phase 3: StaticAbility Runtime System (TODO)

- [ ] Create `StaticAbility` runtime tracking in GameState
- [ ] Extend Mode$ CantBlockBy to use parsed SVar parameters
- [ ] Implement Mode$ Continuous for MayPlay effects
- [ ] Hook into combat and casting systems

### Phase 4: RememberObjects Tracking (TODO)

- [ ] Add remembered_objects field to effect cards
- [ ] Implement Card.IsRemembered validator
- [ ] Support ClearRemembered$ cleanup

### Phase 5: SubAbility Chaining (TODO)

- [ ] Proper Execute$ resolution from SVars
- [ ] SubAbility$ chaining
- [ ] Charm choices (DB$ Charm)

## Key Cards to Test

1. **Deserter's Disciple** - CantBlockBy static ability ✅ Working via PersistentEffect::CantBeBlocked
2. **Rabaroo Troop** - Landfall with DB$ Pump + KW$ + SubAbility$
3. **Thriving Grove** - ETB replacement with ChooseColor
4. **Fire Lord Ozai** - Complex MayPlay from exile with triggers
5. **Zuko, Conflicted** - Multi-choice Charm ability

## Files Modified

- mtg-engine/src/loader/svar_parser.rs ✅ NEW - Core SVar parsing infrastructure
- mtg-engine/src/loader/mod.rs ✅ - Added svar_parser module exports
- mtg-engine/src/loader/ability_parser.rs ✅ - Added iter() method for params
- mtg-engine/src/loader/effect_converter.rs ✅ - Added params_to_effect_with_svars()

## Test Coverage

- 7 unit tests for svar_parser.rs
- 2 unit tests for SVar-based effect conversion

## Related Issues

- mtg-bka7e: AB$ Effect implementation (depends on this)
- mtg-cga7i: Airbend implementation (uses similar patterns)
