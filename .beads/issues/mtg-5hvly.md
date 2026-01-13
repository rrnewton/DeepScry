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

## Priority Bug: Cracked Earth Technique "Card not in hand" Error

**CRITICAL BUG** - Must fix first before other testing.

When the opponent casts Cracked Earth Technique, the game throws an error:
```
P2 casts Cracked Earth Technique (48) (putting on stack)
Cracked Earth Technique (48) resolves
P1 casts Cracked Earth Technique (48) (putting on stack)
Error casting spell: Invalid game action: Card not in hand
```

The card definition uses `SP$ Earthbend | SubAbility$ DBEarthbend` which chains:
1. First earthbend 3
2. SubAbility DBEarthbend (earthbend 3 again)
3. SubAbility DBGainLife (gain 3 life)

The error suggests the SubAbility chain is being misinterpreted as casting the spell again.

- [ ] **FIX BUG**: Cracked Earth Technique SubAbility chain causes "Card not in hand" error
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
