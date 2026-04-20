---
title: Gabriel Avatar Deck Compatibility
status: open
priority: 1
issue_type: task
created_at: 2026-01-13T15:09:09.537491408+00:00
updated_at: 2026-01-17T23:09:56.891913744+00:00
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
- [ ] Verify Cracked Earth Technique earthbends twice (two different lands) - requires manual testing (AI heuristic gap)
- [ ] Verify Cracked Earth Technique grants 3 life - requires manual testing (AI heuristic gap)

---

## Bug: Pillar Launch EntityNotFound(0) [FIXED]

**FIXED** in `mtg-engine/src/game/actions/mod.rs` and related files.

- [x] **FIX BUG**: Pillar Launch SubAbility with Defined$ Targeted causes EntityNotFound(0)

---

## Priority Bug: Barrels of Blasting Jelly Freeze

**CRITICAL BUG** - Causes game freeze/infinite loop (web GUI only).

- [x] Verify activated ability targets correctly (CLI works)
- [x] Verify damage is dealt to target creature (CLI works)
- [x] Verify artifact is sacrificed as part of cost (CLI works)
- [ ] **FIX BUG**: Debug web GUI rewind/replay loop (requires browser console)

---

## UI Enhancement: Clickable Stack Cards

- [ ] Make stack card display clickable to show card details
- [ ] Handle cards the player hasn't seen before

---

## Deck Card Verification Checklist

### Cards: gabriel_avatar_draft.dck

**Lands (16):**
- [x] Ba Sing Se (x2) - activated earthbend 2 ability (VERIFIED 2026-01-15)
- [x] Forest (x7) - basic land
- [x] Plains (x6) - basic land
- [x] Thriving Grove (x1) - enters tapped, choose color (VERIFIED 2026-01-15)

**Creatures (16):**
- [x] Badgermole (x1) - ETB earthbend 2, trample to countered creatures (VERIFIED 2026-01-14)
- [x] Cat-Owl (x1) - flying 3/3, attack trigger untap (FIXED 2026-01-14)
- [ ] Earth Kingdom Soldier (x1) - ETB put counters needs multi-target support
- [x] Foggy Swamp Vinebender (x1) - Waterbend 5 PutCounter (VERIFIED 2026-01-17)
- [x] Glider Kids (x1) - flying, ETB scry 1 (VERIFIED 2026-01-14)
- [ ] Master Piandao (x1) - attack trigger Dig 4 (GAP: DB$ Dig not implemented)
- [x] Ostrich-Horse (x2) - ETB mill+choose land (VERIFIED 2026-01-15)
- [x] Rabaroo Troop (x1) - landfall trigger pump+life (VERIFIED 2026-01-15)
- [ ] Raucous Audience (x3) - mana ability with conditional (GAP: Count$Compare not fully implemented)
- [ ] Suki, Kyoshi Warrior (x1) - */4 CharacteristicDefining, attack trigger token (GAP: CharacteristicDefining)
- [ ] The Boulder, Ready to Rumble (x2) - attack earthbend X (GAP: variable X from Count$Valid)
- [x] Turtle-Duck (x1) - AB$ Animate (VERIFIED 2026-01-15)

**Spells/Other (8):**
- [x] Barrels of Blasting Jelly (x1) - Engine works (verified via agentplay), web GUI rewind bug
- [x] Cracked Earth Technique (x1) - **FIXED** - earthbend sorcery
- [x] Pillar Launch (x1) - **FIXED** - SubAbility$ DBUntap with Defined$ Targeted
- [ ] Rocky Rebuke (x1) - GAP: DamageSource$ ParentTarget not implemented
- [x] Sandbenders' Storm (x2) - SP$ Charm modal spells (VERIFIED 2026-01-17: Both modes work, powerGE4 restriction implemented)
  - **BUG FOUND 2026-04-20**: Earthbend 3 mode resolved without targeting a land (mtg-a385df, FIXED in targeting.rs)
- [ ] Seismic Sense (x1) - GAP: SP$ Dig library manipulation not implemented
- [x] White Lotus Reinforcements (x1) - 2/3 Vigilance creature with Ally anthem (VERIFIED 2026-01-15)

---

## Engine Implementation Gaps (2026-01-17 Update)

The following mechanics are NOT YET IMPLEMENTED in the engine:

- **Dig**: `SP$ Dig` library manipulation not implemented (affects Seismic Sense)
- **DamageSource$ ParentTarget**: Fight-style damage from targeted creature (affects Rocky Rebuke)
- **CharacteristicDefining**: `*/*` power/toughness from formula (affects Suki, Kyoshi Warrior)
- **Count$Valid X**: Variable amounts from creature counts (affects The Boulder)
- **Multi-target PutCounter**: ETB put counters on up to N targets (affects Earth Kingdom Soldier)

Recently fixed:
- ~~**Scry**: `ApiType::Scry` / `DB$ Scry`~~ **IMPLEMENTED**
- ~~**Waterbend**: Avatar-specific mechanic~~ **IMPLEMENTED** (mtg-aui0v CLOSED)
- ~~**Charm**: `SP$ Charm` modal spells~~ **IMPLEMENTED** 2026-01-17
- ~~**Animate**: `AB$ Animate` power/keyword grant~~ **IMPLEMENTED** 2026-01-15
- ~~**powerGE4**: Target restriction for creature power~~ **IMPLEMENTED** 2026-01-17

## AI Heuristic Gaps (2026-01-14)

The following Avatar-specific mechanics work at the engine level but the AI doesn't know how to evaluate them:

- **Earthbend spells** (e.g., Cracked Earth Technique): No heuristic evaluation logic
- **Waterbend effects**: Similar gap - no heuristic evaluation

These are **mtg-77** (Heuristic AI completeness) issues, not engine bugs.

---

## Testing Protocol

1. Test each card type in isolation with puzzles
2. Run full deck vs deck games
3. Verify web GUI compatibility

## Related Issues
- mtg-0iad2: Ryan Avatar Deck Compatibility
- mtg-fmm68: Julian Avatar Deck Compatibility
