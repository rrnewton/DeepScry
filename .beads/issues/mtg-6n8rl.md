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

### Waterbend (Convoke-like cost) - FULLY IMPLEMENTED (2026-01-02_#1435)
- Format: `Cost$ Waterbend<X>` where X is a number
- Effect: While paying a waterbend cost, you can tap your artifacts and creatures to help pay. Each one pays for {1}.
- Similar to Convoke keyword

**Cards affected in avatar decks:**
- Foggy Swamp Vinebender: `Cost$ Waterbend<5>` - put +1/+1 counter ✓ WORKING
- Flexible Waterbender: `Cost$ Waterbend<3>` - uses AB$ Animate ✓ WORKING

**Implementation Status (2026-01-02_#1435):**
- [x] Parse `Waterbend<X>` as a cost type in Cost::parse()
- [x] Add Cost::Waterbend { amount: u8 } variant
- [x] Add PutCounter effect conversion in effect_converter.rs
- [x] Self-targeting for PutCounter abilities (Defined$ Self)
- [x] Full Convoke-like payment: tap creatures/artifacts to pay {1} each
- [x] AB$ Animate effect (set base P/T until end of turn)
- [x] Effect::SetBasePowerToughness - sets temp_base_power/temp_base_toughness
- [x] Cleanup at end of turn (cleared in cleanup_temporary_effects)

### AB$ Animate Effect - DEFERRED (2026-01-02_#1433)

**Benchmark Impact Analysis:**

The Animate effect implementation (commit 5ba51ab on avatar branch) was tested during
cherry-pick bisection and found to dramatically change benchmark metrics:

| Metric           | Before Animate | After Animate | Change  |
|------------------|----------------|---------------|---------|
| Actions/game     | 604            | 1,639         | +171%   |
| Actions/turn     | 28             | 77            | +175%   |
| P1 win rate      | 87%            | 56%           | -31pts  |
| Games/sec        | ~7,500         | ~1,700        | -77%    |

**Root Cause: Mishra's Factory**

The robots benchmark deck (`03_robots_jesseisbak.dck`) contains 4x Mishra's Factory,
a creature-land with `AB$ Animate | Cost$ 1 | ... | Power$ 2 | Toughness$ 2`.

Before the Animate commit: Mishra's Factory was just a colorless mana-only land
(animate ability silently ignored).

After the Animate commit: Mishra's Factory works correctly as a 2/2 creature-land,
which dramatically changes gameplay:
- AI can now animate factories for attacks/blocks
- Games are more balanced (56%/44% vs 87%/13%)
- Games take more actions to complete (creature-lands are powerful in MTG)

**Decision: Defer Animate to Avoid Benchmark Churn**

The Animate effect is working correctly - this is NOT a bug. However, to maintain
benchmark stability and clear historical comparisons, the Animate commit was NOT
cherry-picked into main. The current main branch has:

- 7 of 16 avatar commits cherry-picked
- Benchmark metrics match historical baseline (604 actions/game, 87% win rate)
- Waterbend, PutCounter, mana fixes all working

The Animate commit can be merged later when we're ready to update benchmark baselines.
It's preserved in the avatar branch at commit 5ba51ab.

Note: Waterbend cost payment now works correctly. Player can tap untapped creatures/artifacts
to help pay the cost. Each tapped permanent pays for {1}. Any remaining cost must be paid
with mana from the mana pool.

### Continuous Effects - WORKING

Verified working: `S:Mode$ Continuous | Affected$ Ally.Other+YouCtrl | AddPower$ 1 | AddToughness$ 1`
- White Lotus Reinforcements correctly buffs other Allies +1/+1
- Glider Kids shows 3/4 (instead of base 2/3) when WLR is on battlefield
- Foggy Swamp Vinebender shows 5/4 (instead of base 4/3) when WLR is on battlefield

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

Games run successfully with all avatar decks:
- avatar_5c_allies.dck
- booster_draft/avatar/ryan_avatar_draft.dck
- booster_draft/avatar/gabriel_avatar_draft.dck
- booster_draft/avatar/eric_avatar_draft.dck

All Waterbend abilities work:
- Waterbend cost payment with Convoke-like tapping
- PutCounter abilities (Foggy Swamp Vinebender)
- Animate/SetBasePowerToughness abilities (Flexible Waterbender)

Continuous effects (Ally buffs) work correctly.

Remaining gaps: Airbend, auto-attach, tokens. Games are playable without these.

### ETB Damage Triggers (ValidTgts$ Any) - FIXED (2026-01-02_#1437)

Fixed issue where ETB triggers with `ValidTgts$ Any` (like Mongoose Lizard's "deals 1 damage
to any target") would crash with "DealDamage effect requires a target" when no opponent
creatures were on the battlefield.

Now the trigger correctly targets the opponent player as a fallback when no valid creature
targets exist. This matches MTG rules for "any target" effects.

**Cards affected:**
- Mongoose Lizard: ETB deals 1 damage to any target ✓ WORKING

## Tested Seeds

Verified working: 1, 5, 10, 42, 77, 200, 300, 400, 500, 1000, 2000, 3000, 4000, 5000, 6000,
7777, 8888, 9999, 11111, 12345, 22222, 33333, 44444, 55555, 66666, 77777, 88888, 99999,
100000, 111111, 200000, 300000, 400000, 500000, 600000, 700000, 800000, 900000, 1000000,
111222, 222333, 333444, 123456, 234567, 345678, 456789, 567890, 678901, 789012, 890123,
1234567, 2000000, 3000000, 4000000, 5555555, 7654321, 8888888, 9999999, 10000000, 20000000,
30000000

## Priority

Low - games function well without remaining mechanics.
