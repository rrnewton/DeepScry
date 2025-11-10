---
title: Implement Equipment attachment system
status: open
priority: 2
issue_type: feature
depends_on:
  mtg-3: discovered-from
created_at: 2025-11-10T11:52:25.419378578+00:00
updated_at: 2025-11-10T11:53:05.247896352+00:00
---

# Description

Equipment cards (artifacts with the Equipment subtype) need full attachment mechanics:

## What's Working
- ✅ Equipment artifacts can be cast and enter the battlefield
- ✅ Equipment cards have the Equip keyword parsed from card data
- ✅ Test coverage verifies Equipment resolution (tests/test_spider_suit_equipment.rs)

## What's Missing

### 1. Card Structure Enhancement
- Add attached_to: Option<CardId> field to Card struct
- Track Equipment→Creature attachment relationship bidirectionally
- Support for Aura attachments (similar to Equipment)

### 2. Equip Activated Ability
- Implement Equip as an activated ability that can be triggered
- Cost: Mana cost specified in Equip keyword (e.g., Equip {3})
- Target: Creature you control
- Timing restriction: Sorcery speed only
- Effect: Attach Equipment to target creature (detach from previous if attached)

### 3. Continuous Effects from Equipment
- Parse Equipment buff effects (e.g., Equipped creature gets +2/+2)
- Apply stat bonuses to attached creature
- Apply keyword grants to attached creature
- Apply type grants to attached creature (e.g., Spider Hero types)

### 4. State-Based Actions
- When equipped creature leaves battlefield → detach Equipment
- When Equipment leaves battlefield → clear attachment reference
- When Equipment becomes a creature → detach from creature

### 5. Game Rules Integration
- Equipment can't be attached to opponent's creatures
- Equipment can only attach to creatures
- Multiple Equipment can attach to same creature
- One Equipment can only attach to one creature at a time

## Test Cases Needed
- Equip ability activation and resolution
- Stat bonuses applied correctly
- Equipment detaches when creature dies
- Equipment detaches when Equipment leaves battlefield
- Multiple Equipment on same creature
- Combat damage reflects Equipment buffs

## Example Card
Spider-Suit (test case in tests/test_spider_suit_equipment.rs):
- Cost: {1}
- Effect: Equipped creature gets +2/+2 and is a Spider Hero
- Equip: {3}

## Related Files
- mtg-engine/src/core/card.rs (Card struct needs attached_to field)
- mtg-engine/src/game/actions.rs (equip ability implementation)
- mtg-engine/src/game/game_loop.rs (continuous effects application)
- tests/test_spider_suit_equipment.rs (test coverage)

## Priority Justification
Equipment is a fundamental MTG mechanic present in many sets including the Spider-Man set we're testing.
