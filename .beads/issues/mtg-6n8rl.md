---
title: Avatar set mechanics (Waterbend, Airbend) support
status: open
priority: 3
issue_type: task
created_at: 2026-01-02T04:51:41.304506805+00:00
updated_at: 2026-01-02T05:33:16.005738281+00:00
---

# Description

Track implementation of Avatar set-specific mechanics for full booster draft support.

## Mechanics Needed

### Waterbend (Convoke-like cost) - PARTIALLY IMPLEMENTED
- Format: `Cost$ Waterbend<X>` where X is a number
- Effect: While paying a waterbend cost, you can tap your artifacts and creatures to help pay. Each one pays for {1}.
- Similar to Convoke keyword

**Cards affected in avatar decks:**
- Foggy Swamp Vinebender: `Cost$ Waterbend<5>` - put +1/+1 counter ✓ WORKING
- Flexible Waterbender: `Cost$ Waterbend<3>` - uses AB$ Animate (not implemented)
- Thriving Grove (indirectly)

**Implementation Status (2026-01-02_#1431):**
- [x] Parse `Waterbend<X>` as a cost type in Cost::parse()
- [x] Add Cost::Waterbend { amount: u8 } variant
- [x] Add PutCounter effect conversion in effect_converter.rs
- [x] Basic payment handling (as generic mana cost)
- [x] Self-targeting for PutCounter abilities (Defined$ Self)
- [ ] AB$ Animate effect (set base P/T until end of turn) - needed for Flexible Waterbender
- [ ] During ability activation, allow tapping creatures/artifacts to reduce cost

Note: PutCounter abilities now work correctly (Fire Sages, Foggy Swamp Vinebender). Animate abilities still need implementation.

### Airbend (Exile-recast effect) - NOT IMPLEMENTED
- Format: `DB$ Airbend | ValidTgts$ Creature`
- Effect: Exile target. While exiled, owner may cast it for {2} rather than mana cost.

**Cards affected (not in current test decks):**
- Aang, the Last Airbender: ETB trigger airbends nonland permanent
- Monk Gyatso: Triggered on targeting other creatures
- Glider Staff: ETB airbend creature
- Airbender Ascension: ETB airbend creature

## Other Avatar Card Limitations

**Twin Blades (Equipment with ETB auto-attach)**
- Uses `T:Mode$ ChangesZone | Execute$ TrigAttach` with `DB$ Attach`
- Auto-attach on ETB not implemented - tracked in mtg-17
- Basic equip ability works

## Known Limitations (tracked elsewhere)

- Token creation not implemented (mtg-34) - affects Suki creating Ally tokens
- CharacteristicDefining power/toughness (mtg-20) - affects Suki's */4 power
- DB$ Attach in ETB triggers (mtg-17) - affects Twin Blades auto-attach

## Current Status

Games run successfully with avatar decks. Waterbend PutCounter abilities work. Several advanced
mechanics (Animate, Airbend, auto-attach, tokens) are not implemented but games are playable.

## Tested Seeds

Verified working: 1, 5, 10, 42, 77, 200, 300, 400, 500, 1000, 2000, 3000, 4000, 5000, 6000, 7777, 8888, 9999, 12345

## Priority

Low - games function well without remaining mechanics.
