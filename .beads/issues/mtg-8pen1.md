---
title: Implement CopyPermanent effect for token copy cards
status: closed
priority: 3
issue_type: task
labels:
- feature
created_at: 2026-01-04T21:53:16.687375407+00:00
updated_at: 2026-01-04T22:13:58.889708465+00:00
---

# Description

## CopyPermanent Effect Implementation

## Summary
Implemented the CopyPermanent effect which creates token copies of existing permanents with optional modifications.

## Implementation Details

### New Effect Type
Added `Effect::CopyPermanent` with:
- `target`: The permanent to copy
- `controller`: Who controls the token
- `non_legendary`: Remove Legendary supertype (not yet enforced - legendary rule not implemented)
- `set_power`: Override power
- `set_toughness`: Override toughness  
- `add_types`: Add creature types (e.g., Hero, Coward)
- `num_copies`: Number of copies to create

### Files Modified
- `loader/ability_parser.rs`: Added `ApiType::CopyPermanent`
- `core/effects.rs`: Added `Effect::CopyPermanent` variant
- `loader/effect_converter.rs`: Added parsing for CopyPermanent parameters
- `game/actions/mod.rs`: Added effect handler and target resolution
- `game/actions/targeting.rs`: Added targeting support for CopyPermanent
- `game/game_loop/actions.rs`: Added to `spell_requires_battlefield_target`
- `game/game_loop/logging.rs`: Added logging for CopyPermanent

### Test Coverage
Created puzzle files:
- `puzzles/copy_permanent_basic_e2e.pzl`: Cackling Counterpart test
- `puzzles/ember_island_production_e2e.pzl`: Modal CopyPermanent test

Added unit tests:
- `test_convert_copy_permanent_simple`
- `test_convert_copy_permanent_with_modifications`
- `test_convert_copy_permanent_multiple_types`
- `test_convert_copy_permanent_with_num_copies`
- `test_charm_with_copy_permanent_ember_island`

## Cards Supported
- Cackling Counterpart (simple copy)
- Ember Island Production (modal with SetPower/SetToughness/AddTypes)
- ~347 other cards using CopyPermanent

## Known Limitations
- `non_legendary` flag parsed but not enforced (legendary rule not implemented)
- Some advanced parameters not yet supported (SetColor, AddKeywords)

## Status: CLOSED
