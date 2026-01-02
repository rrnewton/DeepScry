---
title: Avatar set mechanics (Waterbend, Airbend) support
status: open
priority: 3
issue_type: task
created_at: 2026-01-02T04:51:41.304506805+00:00
updated_at: 2026-01-02T04:51:41.304506805+00:00
---

# Description

Track implementation of Avatar set-specific mechanics for full booster draft support.

## Mechanics Needed

### Waterbend (Convoke-like cost)
- Format: `Cost$ Waterbend<X>` where X is a number
- Effect: While paying a waterbend cost, you can tap your artifacts and creatures to help pay. Each one pays for {1}.
- Similar to Convoke keyword

**Cards affected in avatar decks:**
- Foggy Swamp Vinebender: `Cost$ Waterbend<5>` - put +1/+1 counter
- Flexible Waterbender: `Cost$ Waterbend<3>` - become 5/2 until EOT
- Thriving Grove (indirectly)

**Implementation needed:**
1. Parse `Waterbend<X>` as a cost type in Cost::parse()
2. Add Cost::Waterbend { amount: u8 } variant
3. During ability activation, allow tapping creatures/artifacts to reduce cost
4. Similar to Convoke implementation (when that's added)

### Airbend (Exile-recast effect)
- Format: `DB$ Airbend | ValidTgts$ Creature`
- Effect: Exile target. While exiled, owner may cast it for {2} rather than mana cost.
- Creates a replacement effect on the exiled card

**Cards affected:**
- Aang, the Last Airbender: ETB trigger airbends nonland permanent
- Monk Gyatso: Triggered on targeting other creatures
- Glider Staff: ETB airbend creature
- Airbender Ascension: ETB airbend creature

**Implementation needed:**
1. Add ApiType::Airbend to ability_parser.rs
2. Create Effect::Airbend in core/effect.rs
3. Handle exile zone + alternative cost casting
4. Track "airbended" state on exiled cards

## Current Status (2026-01-02)

- Waterbend abilities silently skipped (Cost::parse returns None)
- Airbend abilities unimplemented (no ApiType::Airbend)
- Games still run - abilities are just not available

## Priority

Medium - games function without these mechanics, but avatar deck gameplay
is incomplete without them.
