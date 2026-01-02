---
title: Avatar set mechanics (Waterbend, Airbend) support
status: open
priority: 3
issue_type: task
created_at: 2026-01-02T04:51:41.304506805+00:00
updated_at: 2026-01-02T13:06:48.751082080+00:00
---

# Description

Track implementation of Avatar set-specific mechanics for full booster draft support.

## Mechanics Needed

### Waterbend (Convoke-like cost) - FULLY IMPLEMENTED (2026-01-02_#1435)
- Format: `Cost$ Waterbend<X>` where X is a number
- Effect: While paying a waterbend cost, you can tap your artifacts and creatures to help pay. Each one pays for {1}.
- Similar to Convoke keyword

**Cards affected in avatar decks:**
- Foggy Swamp Vinebender: `Cost$ Waterbend<5>` - put +1/+1 counter ✓ WORKING
- Flexible Waterbender: `Cost$ Waterbend<3>` - uses AB$ Animate ✓ WORKING

**Implementation Status (2026-01-02_#1447):**
- [x] Parse `Waterbend<X>` as a cost type in Cost::parse()
- [x] Add Cost::Waterbend { amount: u8 } variant
- [x] Add PutCounter effect conversion in effect_converter.rs
- [x] Self-targeting for PutCounter abilities (Defined$ Self)
- [x] Full Convoke-like payment: tap creatures/artifacts to pay {1} each
- [x] AB$ Animate effect (set base P/T until end of turn) - MERGED
- [x] Effect::SetBasePowerToughness - sets temp_base_power/temp_base_toughness
- [x] Cleanup at end of turn (cleared in cleanup_temporary_effects)

Note: Waterbend cost payment works correctly. Animate effect merged with documented
benchmark impact (see commit 0c4c69c for details on Mishra's Factory gameplay changes).

### Continuous Effects - WORKING

Verified working: `S:Mode$ Continuous | Affected$ Ally.Other+YouCtrl | AddPower$ 1 | AddToughness$ 1`
- White Lotus Reinforcements correctly buffs other Allies +1/+1
- Glider Kids shows 3/4 (instead of base 2/3) when WLR is on battlefield
- Foggy Swamp Vinebender shows 5/4 (instead of base 4/3) when WLR is on battlefield

### Airbend (Exile-recast effect) - IN PROGRESS (tracked in mtg-cga7i)

See mtg-cga7i for detailed implementation status.

Infrastructure completed:
- [x] PersistentEffectStore - dedicated storage (NOT command zone like Java)
- [x] Effect::Airbend variant, parser, converter
- [x] Targeting and execution logic

Still needed (see mtg-cga7i):
- [ ] MayPlay from exile: allow casting exiled card for {2}
- [ ] Cleanup triggers: remove effect when card leaves exile or is cast

**Cards affected:**
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

## Current Status - NOT PLAYABLE

**IMPORTANT: Avatar decks are NOT fully playable.**

Previous claims of "playable" status were incorrect. These decks are missing core mechanics:

1. **Airbend** (mtg-cga7i) - A fundamental mechanic for Air Nation cards
2. **Token creation** (mtg-34) - Suki and other cards create Ally tokens
3. **CharacteristicDefining P/T** (mtg-20) - Suki's power is undefined

Games may run without crashes, but the gameplay is NOT correct without these mechanics.
A deck is only "playable" when ALL mechanics are implemented correctly.

### ETB Damage Triggers (ValidTgts$ Any) - FIXED (2026-01-02_#1437)

Fixed issue where ETB triggers with `ValidTgts$ Any` (like Mongoose Lizard's "deals 1 damage
to any target") would crash when no opponent creatures were on the battlefield.

## Priority

HIGH - games are NOT balanced without Airbend mechanic.
