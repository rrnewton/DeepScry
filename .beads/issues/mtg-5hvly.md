---
title: Gabriel Avatar Deck Compatibility
status: open
priority: 1
issue_type: task
created_at: 2026-01-13T15:09:09.537491408+00:00
updated_at: 2026-01-13T15:09:09.537491408+00:00
---

# Description

## Gabriel Avatar Deck Compatibility Tracking

This issue tracks compatibility testing and bug fixes for the **gabriel_avatar_draft.dck** deck when played in the web GUI.

## Priority Bug: Cracked Earth Technique "Card not in hand" Error [FIXED]

**FIXED** in `wasm/fancy_tui.rs::rewind_to_turn_start()`

**Root Cause**: When the web GUI rewound to turn start, it extracted ALL players' choices
from the undo log, including P2 (AI opponent) choices. When P1's ReplayController tried
to replay these choices, it would attempt to execute P2's actions (like casting P2's spell),
which failed with "Card not in hand" because P1 can't cast P2's cards.

**Fix**: Filter extracted choices by player_id in `rewind_to_turn_start()` so only P1's
choices are given to P1's ReplayController. P2's choices will be re-made by the AI
controller during replay.

Original error:
```
P2 casts Cracked Earth Technique (48) (putting on stack)
Cracked Earth Technique (48) resolves
P1 casts Cracked Earth Technique (48) (putting on stack)
Error casting spell: Invalid game action: Card not in hand
```

- [x] **FIX BUG**: Cracked Earth Technique SubAbility chain causes "Card not in hand" error
- [ ] Verify Cracked Earth Technique earthbends twice (two different lands)
- [ ] Verify Cracked Earth Technique grants 3 life

---

## Priority Bug: Barrels of Blasting Jelly Freeze

**CRITICAL BUG** - Causes game freeze/infinite loop.

Card: `{5}, {T}, Sacrifice this artifact: It deals 5 damage to target creature.`

Issues observed:
1. Log doesn't show target creature name: "It deals 5 damage to target creature" (should say "(NAME CARDID)")
2. After activation, game enters infinite rewind loop - keeps rewinding without making progress
3. Undo/replay system gets stuck cycling between action counts

Debug log shows stuck pattern:
```
Moving card Barrels of Blasting Jelly (id=46) from Battlefield to Graveyard
...
REWIND: Rewound to turn 10, 18 actions undone
...
Moving card Barrels of Blasting Jelly (id=46) from Graveyard to Battlefield
```

- [ ] **FIX BUG**: Barrels of Blasting Jelly causes infinite rewind loop
- [ ] Verify activated ability targets correctly
- [ ] Verify damage is dealt to target creature
- [ ] Verify artifact is sacrificed as part of cost

---

## UI Enhancement: Clickable Stack Cards

Currently no way to see card details for unknown opponent cards on the stack.

- [ ] Make stack card display clickable to show card details
- [ ] Handle cards the player hasn't seen before

---

## Deck Card Verification Checklist

### Cards: gabriel_avatar_draft.dck

**Lands (16):**
- [ ] Ba Sing Se (x2) - activated earthbend 2 ability
- [ ] Forest (x7) - basic land
- [ ] Plains (x6) - basic land
- [ ] Thriving Grove (x1) - enters tapped, choose color

**Creatures (16):**
- [ ] Badgermole (x1) - ETB earthbend 2, trample to countered creatures
- [ ] Cat-Owl (x1) - flying 2/1
- [ ] Earth Kingdom Soldier (x1) - 2/2 baseline
- [ ] Foggy Swamp Vinebender (x1) - waterbend effects
- [ ] Glider Kids (x1) - flying, token generation
- [ ] Master Piandao (x1) - equipment synergy
- [ ] Ostrich-Horse (x2) - haste, attack trigger
- [ ] Rabaroo Troop (x1) - token/creature synergy
- [ ] Raucous Audience (x3) - pump/anthem effects
- [ ] Suki, Kyoshi Warrior (x1) - legendary, combat abilities
- [ ] The Boulder, Ready to Rumble (x2) - fight/combat abilities
- [ ] Turtle-Duck (x1) - small utility creature

**Spells/Other (8):**
- [ ] Barrels of Blasting Jelly (x1) - **BUG: FREEZE** - mana/damage artifact
- [ ] Cracked Earth Technique (x1) - **BUG: CARD NOT IN HAND** - earthbend sorcery
- [ ] Pillar Launch (x1) - combat trick/pump
- [ ] Rocky Rebuke (x1) - removal spell
- [ ] Sandbenders' Storm (x2) - board effect
- [ ] Seismic Sense (x1) - card selection/draw
- [ ] White Lotus Reinforcements (x1) - token generation

---

## Testing Protocol

1. Fix Cracked Earth Technique SubAbility bug first
2. Fix Barrels of Blasting Jelly freeze
3. Test each card type in isolation with puzzles
4. Run full deck vs deck games
5. Verify web GUI compatibility

**NOTE**: When starting work on compatibility for a SPECIFIC CARD, expand its checklist entry into a detailed list with subtasks for each card ability (parsing, execution, targeting, triggers, etc.) - same format as mtg-0iad2 (Ryan Avatar Deck tracking issue). This ensures thorough verification of all card behaviors.

## Related Issues
- mtg-0iad2: Ryan Avatar Deck Compatibility (similar tracking issue)
