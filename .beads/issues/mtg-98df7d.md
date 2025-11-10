---
title: Implement Equipment attachment system
status: open
priority: 2
issue_type: feature
depends_on:
  mtg-3: discovered-from
created_at: 2025-11-10T11:52:25.419378578+00:00
updated_at: 2025-11-10T13:09:39.886624268+00:00
---

# Description

## Equipment Attachment Implementation Plan

Based on study of Java Forge implementation (2025-11-10).

## Architecture Overview

### Java Forge Approach (Verified)

**Card.java**:
- `private GameEntity entityAttachedTo` - single field tracks attachment to Card or Player
- `getEquippedBy()` - returns Equipment attached TO this creature
- `getEquipping()` / `isEquipping()` - checks if THIS card is attached to something
- `attachToEntity(GameEntity e, SpellAbility sa)` - performs attachment with timestamp update
- `unattachFromEntity(GameEntity e)` - handles detachment

**AttachEffect.java**:
- Resolves Equip activated ability
- Validates target can be attached (`canBeAttached` predicate)
- Calls `attachment.attachToEntity(attachTo, sa)`
- Handles optional attachment confirmation

**StaticAbilityContinuous.java**:
- Line 1011-1081: `getAffectedCards()` determines which cards are affected
- "Creature.EquippedBy" selector filters for creatures equipped by this Equipment
- Lines 156-166: `AddPower` / `AddToughness` parameters in MODIFYPT layer
- Lines 688-697: Applies P/T bonuses via `affectedCard.addPTBoost()`

## Rust Implementation Plan

### Phase 1: Card Structure ✅ COMPLETE (5d14a43c)

**Card.attached_to field** (mtg-engine/src/core/card.rs):
```rust
pub struct Card {
    pub attached_to: Option<CardId>,  // Equipment/Aura attachment tracking
}
```

**Helper methods**:
- `is_equipment()` - checks for Equipment subtype
- `is_attached()` - checks if attached_to is Some
- `get_attached_to()` - returns attached_to value

**GameState actions** (mtg-engine/src/game/actions.rs):
- `attach_equipment(equipment_id, target_id)` - validates and attaches Equipment
- `detach_equipment(equipment_id)` - safe detachment
- `get_attached_equipment(creature_id)` - query helper

**Tests**: 4 comprehensive tests (attachment, detachment, multiple Equipment)

### Phase 2: Equipment Buff Calculation ✅ COMPLETE (0d2230dc)

**Buff calculation methods** (mtg-engine/src/game/actions.rs):
- `get_effective_power(creature_id)` - calculates total power including Equipment buffs
- `get_effective_toughness(creature_id)` - calculates total toughness including Equipment buffs
- Currently hardcoded for Spider-Suit (+2/+2), will be replaced with parsed effects in Phase 3

**Tests**: 2 new tests (buff lifecycle, multiple Equipment stacking)
**Total tests**: 6 Equipment tests passing

### Phase 2b: Combat Integration & State-Based Actions ✅ COMPLETE (c20cd2cc)

**Combat damage integration** (mtg-engine/src/game/actions.rs):
- Modified `assign_combat_damage()` to use `get_effective_power()` for all damage calculations
- Attackers deal buffed damage (not base power) to blockers and players
- Blockers deal buffed damage back to attackers
- Changed `remaining_power` from `i8` to `i32` to match Equipment buff system

**State-based action** (mtg-engine/src/game/state.rs):
- Added automatic Equipment detachment when creatures leave battlefield
- Implemented in `move_card()` when creature exits Zone::Battlefield
- Equipment remains on battlefield but `attached_to` field is cleared

**Tests**: 2 new tests (combat damage calculation, state-based action)
**Total tests**: 8 Equipment tests, all 484 tests passing

### Phase 3: Static Ability Parsing (TODO)

Parse Spider-Suit static ability from card data:
```
S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2 | AddType$ Spider & Hero
```

**Tasks**:
1. Parse "S:Mode$ Continuous" lines from cardsfolder
2. Parse "Affected$ Creature.EquippedBy" selector
3. Parse "AddPower$ 2 | AddToughness$ 2" effects
4. Replace hardcoded Spider-Suit buff with parsed static abilities
5. Support generic Equipment with different buff values

**New module**: mtg-engine/src/loader/continuous_effects.rs (?)

### Phase 4: Equip Activated Ability (TODO)

Parse and implement Equip from card data:
```
K:Equip:3
```

**Tasks**:
1. Parse "K:Equip:N" from cardsfolder
2. Create ActivatedAbility for Equip
3. Implement sorcery-speed timing restriction
4. Implement target selection (creature you control)
5. Resolve Equip by calling attach_equipment()

**Integration**: mtg-engine/src/loader/ability_parser.rs

### Phase 5: Testing & Polish (TODO)

**Integration tests**:
- Cast Equipment from hand (already tested)
- Pay Equip cost and activate ability
- Attach to creature via Equip
- Attack with equipped creature
- Verify damage = base + Equipment buffs
- Creature dies, Equipment detaches
- Re-equip to different creature

**Files to complete**:
1. ~~mtg-engine/src/core/card.rs~~ ✅ DONE
2. ~~mtg-engine/src/game/actions.rs~~ ✅ DONE (attach/detach/buffs/combat)
3. ~~mtg-engine/src/game/state.rs~~ ✅ DONE (state-based action)
4. mtg-engine/src/loader/continuous_effects.rs (NEW - for static ability parsing)
5. mtg-engine/src/loader/ability_parser.rs (parse K:Equip)
6. ~~mtg-engine/tests/test_spider_suit_equipment.rs~~ ✅ DONE (8 tests)

## Progress Summary

**Completed (2025-11-10)**:
- ✅ Phase 1: Card structure and attachment infrastructure (5d14a43c)
- ✅ Phase 2: Equipment buff calculation (0d2230dc)
- ✅ Phase 2b: Combat integration and state-based actions (c20cd2cc)

**Remaining**:
- Phase 3: Parse static abilities from card data (~3 hours)
- Phase 4: Implement Equip activated ability (~2 hours)
- Phase 5: End-to-end integration testing (~1 hour)

**Current Functionality**:
- Equipment can be cast and enter battlefield ✅
- Equipment can attach/detach programmatically ✅
- Creatures get stat buffs from attached Equipment ✅
- Combat damage uses buffed stats ✅
- Equipment auto-detaches when creature dies ✅
- Equip ability not yet functional ❌ (needs Phase 3-4)

**Test Coverage**: 8 Equipment tests, all 484 tests passing

## Relationship to Java Forge

Java Forge uses a sophisticated layered system with:
- 7 effect layers applied in order (CR 613)
- Timestamp-based ordering within layers
- Full support for Auras, Equipment, and Fortifications
- Complex selector language ("Creature.EquippedBy", "Creature+YouCtrl")

Our Rust implementation:
- **Phase 1-2b (DONE)**: Core attachment infrastructure with hardcoded buffs
- **Phase 3-4 (TODO)**: Generic static ability parsing for data-driven buffs
- **Future**: Expand to Auras, player attachments, complex selectors as needed

## Estimated Complexity

- ~~Phase 1 (Card struct): 30 minutes~~ ✅ DONE
- ~~Phase 2 (Buff calculation): 1 hour~~ ✅ DONE
- ~~Phase 2b (Combat & SBA): 1 hour~~ ✅ DONE
- **Phase 3 (Static abilities): 3 hours** (IN PROGRESS)
- Phase 4 (Equip ability): 2 hours
- Phase 5 (Testing): 1 hour
- **Total**: 8.5 hours (~3.5 hours completed, ~5 hours remaining)

## Next Steps

**Immediate (Phase 3)**:
1. Study Java Forge's StaticAbilityContinuous parsing
2. Create continuous_effects.rs or extend existing ability parser
3. Parse "S:Mode$ Continuous" lines from cardsfolder
4. Replace hardcoded Spider-Suit buff with parsed static abilities

**Then (Phase 4)**:
1. Parse "K:Equip:N" from cardsfolder
2. Implement Equip as activated ability
3. Add sorcery-speed timing check

**Finally (Phase 5)**:
1. End-to-end test: cast, equip, attack, verify buffed damage
2. Create puzzle file for Spider-Suit scenario (if desired)

---

**Status**: Phases 1-2b complete, core infrastructure working. Next: static ability parsing.
**Latest commit**: c20cd2cc (2025-11-10)
