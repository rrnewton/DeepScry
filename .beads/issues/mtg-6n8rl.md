---
title: Avatar set mechanics (Waterbend, Airbend) support
status: open
priority: 3
issue_type: task
created_at: 2026-01-02T04:51:41.304506805+00:00
updated_at: 2026-01-02T05:25:03.943302133+00:00
---

# Description

Track implementation of Avatar set-specific mechanics for full booster draft support.

## Mechanics Needed

### Waterbend (Convoke-like cost) - PARTIALLY IMPLEMENTED
- Format: `Cost$ Waterbend<X>` where X is a number
- Effect: While paying a waterbend cost, you can tap your artifacts and creatures to help pay. Each one pays for {1}.
- Similar to Convoke keyword

**Cards affected in avatar decks:**
- Foggy Swamp Vinebender: `Cost$ Waterbend<5>` - put +1/+1 counter ✓
- Flexible Waterbender: `Cost$ Waterbend<3>` - become 5/2 until EOT
- Thriving Grove (indirectly)

**Implementation Status (2026-01-02_#168):**
- [x] Parse `Waterbend<X>` as a cost type in Cost::parse()
- [x] Add Cost::Waterbend { amount: u8 } variant
- [x] Add PutCounter effect conversion in effect_converter.rs
- [x] Basic payment handling (as generic mana cost)
- [x] Self-targeting for PutCounter abilities (Defined$ Self)
- [ ] During ability activation, allow tapping creatures/artifacts to reduce cost

Note: Abilities now load and activate correctly. Payment treats Waterbend<X> as {X} generic mana. Full Convoke-like tapping is TODO.

### Airbend (Exile-recast effect) - NOT IMPLEMENTED
- Format: `DB$ Airbend | ValidTgts$ Creature`
- Effect: Exile target. While exiled, owner may cast it for {2} rather than mana cost.
- Creates a replacement effect on the exiled card

**Cards affected (not in current test decks):**
- Aang, the Last Airbender: ETB trigger airbends nonland permanent
- Monk Gyatso: Triggered on targeting other creatures
- Glider Staff: ETB airbend creature
- Airbender Ascension: ETB airbend creature

## Known Limitations (tracked elsewhere)

- Token creation not implemented (mtg-34) - affects Suki creating Ally tokens
- CharacteristicDefining power/toughness (mtg-20) - affects Suki's */4 power
- Attack triggers with token creation - dependent on above

## Current Status

Games run successfully with avatar decks (gabriel vs ryan draft decks). Waterbend abilities
work correctly for putting +1/+1 counters on creatures. Token-creating and
characteristic-defining abilities don't function yet but are tracked in separate issues.

## Tested Seeds

Verified working: 1, 5, 10, 42, 77, 200, 300, 400, 500, 1000, 2000, 3000, 4000, 5000, 6000

## Priority

Low - games function well without remaining mechanics.
