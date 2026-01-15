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
- [ ] Verify Cracked Earth Technique earthbends twice (two different lands) - requires manual testing (AI heuristic gap)
- [ ] Verify Cracked Earth Technique grants 3 life - requires manual testing (AI heuristic gap)

---

## Bug: Pillar Launch EntityNotFound(0) [FIXED]

**FIXED** in `mtg-engine/src/game/actions/mod.rs` and related files.

**Root Cause**: SubAbility chains with `Defined$ Targeted` (like Pillar Launch's `SP$ Pump | SubAbility$ DBUntap`)
expected to reuse the parent ability's target, but the effect converter created a placeholder CardId(0). When resolving,
the code tried to consume a NEW target from `chosen_targets`, but only one target was provided for the whole chain.

**Fix**: Implemented `REUSE_PREVIOUS_TARGET` sentinel (u32::MAX) to indicate "reuse previous target":
- `mtg-engine/src/core/entity.rs`: Added `REUSE_PREVIOUS_TARGET` constant and `is_reuse_previous()`/`reuse_previous()` methods
- `mtg-engine/src/loader/effect_converter.rs`: Detect `Defined$ Targeted` and use `CardId::reuse_previous()` sentinel
- `mtg-engine/src/game/actions/mod.rs`: Track `last_resolved_target` through effect chain, reuse it for sentinel targets

Original error:
```
Entity not found: 0
```

Now works correctly:
```
Pillar Launch (62) gives Raucous Audience (72) +2/+2 until end of turn
Pillar Launch (62) untaps <target>
```

- [x] **FIX BUG**: Pillar Launch SubAbility with Defined$ Targeted causes EntityNotFound(0)

---

## Priority Bug: Barrels of Blasting Jelly Freeze

**CRITICAL BUG** - Causes game freeze/infinite loop (web GUI only).

Card: `{5}, {T}, Sacrifice this artifact: It deals 5 damage to target creature.`

### Investigation Results (2026-01-14)

**Engine works correctly!** Verified via agentplay test:
```bash
./agentplay/start_game.sh --start-state="test_puzzles/test_barrels_of_blasting_jelly.pzl"
./agentplay/continue_game.sh "activate Barrels of Blasting Jelly"
```

Output shows correct behavior:
```
Barrels of Blasting Jelly activates ability: It deals 5 damage to target creature.
Grizzly Bears (11) takes 5 damage (total: 5)
Grizzly Bears (11) dies from lethal damage
```

- [x] Verify activated ability targets correctly (CLI works)
- [x] Verify damage is dealt to target creature (CLI works)
- [x] Verify artifact is sacrificed as part of cost (CLI works)

### Remaining Issue: Web GUI Infinite Rewind Loop

The bug is **specific to the web GUI's rewind/replay system**, not the game engine.
This requires browser-based debugging of `fancy_tui.rs` WASM code.

Hypothesis: Something in the WASM TUI's event handling or render callback may be
triggering repeated rewind cycles after activated ability with sacrifice cost resolves.

Original debug log pattern:
```
Moving card Barrels of Blasting Jelly (id=46) from Battlefield to Graveyard
...
REWIND: Rewound to turn 10, 18 actions undone
...
Moving card Barrels of Blasting Jelly (id=46) from Graveyard to Battlefield
```

- [ ] **FIX BUG**: Debug web GUI rewind/replay loop (requires browser console)

---

## UI Enhancement: Clickable Stack Cards

Currently no way to see card details for unknown opponent cards on the stack.

- [ ] Make stack card display clickable to show card details
- [ ] Handle cards the player hasn't seen before

---

## Deck Card Verification Checklist

### Cards: gabriel_avatar_draft.dck

**Lands (16):**
- [x] Ba Sing Se (x2) - activated earthbend 2 ability (VERIFIED 2026-01-15: SorcerySpeed$ parsing, Earthbend targeting, mana exclusion fixes)
- [x] Forest (x7) - basic land
- [x] Plains (x6) - basic land
- [x] Thriving Grove (x1) - enters tapped, choose color (VERIFIED 2026-01-15: ETB tapped works, ChooseColor works)

**Creatures (16):**
- [x] Badgermole (x1) - ETB earthbend 2, trample to countered creatures (VERIFIED 2026-01-14: earthbend works, makes 2/2 land creature)
- [x] Cat-Owl (x1) - flying 3/3, attack trigger untap (FIXED 2026-01-14)
- [ ] Earth Kingdom Soldier (x1) - ETB put counters needs multi-target support
- [ ] Foggy Swamp Vinebender (x1) - waterbend effects (GAP: waterbend not implemented)
- [x] Glider Kids (x1) - flying (works), ETB scry 1 (VERIFIED 2026-01-14: Scry implemented)
- [ ] Master Piandao (x1) - attack trigger Dig 4 (GAP: DB$ Dig not implemented)
- [x] Ostrich-Horse (x2) - ETB mill+choose land (VERIFIED 2026-01-15: Mill 3 works, +1/+1 counter added if no land chosen)
- [x] Rabaroo Troop (x1) - landfall trigger pump+life (VERIFIED 2026-01-15: Landfall implemented, life gain works)
- [ ] Raucous Audience (x3) - mana ability with conditional (GAP: Count$Compare not fully implemented)
- [ ] Suki, Kyoshi Warrior (x1) - */4 CharacteristicDefining, attack trigger token (GAP: CharacteristicDefining)
- [ ] The Boulder, Ready to Rumble (x2) - attack earthbend X (GAP: variable X from Count$Valid)
- [ ] Turtle-Duck (x1) - AB$ Animate (GAP: Animate not implemented)

**Spells/Other (8):**
- [x] Barrels of Blasting Jelly (x1) - Engine works (verified via agentplay), web GUI rewind bug
- [x] Cracked Earth Technique (x1) - **FIXED** - earthbend sorcery (was web GUI replay bug)
- [x] Pillar Launch (x1) - **FIXED** - SubAbility$ DBUntap with Defined$ Targeted now works
- [ ] Rocky Rebuke (x1) - GAP: DamageSource$ ParentTarget not implemented
- [ ] Sandbenders' Storm (x2) - GAP: SP$ Charm modal spells not implemented
- [ ] Seismic Sense (x1) - GAP: SP$ Dig library manipulation not implemented
- [x] White Lotus Reinforcements (x1) - 2/3 Vigilance creature with Ally anthem (VERIFIED 2026-01-15: anthem gives +1/+1 to other allies)

---

## Engine Implementation Gaps (2026-01-14)

The following mechanics are NOT YET IMPLEMENTED in the engine:

- ~~**Scry**: `ApiType::Scry` / `DB$ Scry` not implemented (affects Glider Kids)~~ **IMPLEMENTED**
- **Waterbend**: Avatar-specific mechanic not implemented (affects Foggy Swamp Vinebender)
- **Dig**: `SP$ Dig` library manipulation not implemented (affects Seismic Sense)
- **Charm**: `SP$ Charm` modal spells not implemented (affects Sandbenders' Storm)
- **Animate**: `AB$ Animate` power/keyword grant not implemented (affects Turtle-Duck)
- **DamageSource$ ParentTarget**: Fight-style damage from targeted creature (affects Rocky Rebuke)
- **CharacteristicDefining**: `*/*` power/toughness from formula (affects Suki, Kyoshi Warrior)
- **Count$Valid X**: Variable amounts from creature counts (affects The Boulder)
- ~~**Mill in ETB triggers**: `DB$ Mill` not parsed for ChangesZone triggers (affects Ostrich-Horse)~~ **WORKS** - verified 2026-01-15
- ~~**Landfall triggers**: `ValidCard$ Land.YouCtrl` not parsed (affects Rabaroo Troop)~~ **IMPLEMENTED** 2026-01-15
- **Multi-target PutCounter**: ETB put counters on up to N targets (affects Earth Kingdom Soldier)

## AI Heuristic Gaps (2026-01-14)

The following Avatar-specific mechanics work at the engine level but the AI doesn't know how to evaluate them:

- **Earthbend spells** (e.g., Cracked Earth Technique): `should_cast_spell()` in heuristic_controller.rs doesn't have Earthbend evaluation logic. The spell parses correctly and executes when cast, but AI never chooses to cast it.
- **Waterbend effects**: Similar gap - no heuristic evaluation.

These are **mtg-77** (Heuristic AI completeness) issues, not engine bugs. The mechanics work if cast manually or through puzzles.

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
