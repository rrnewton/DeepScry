---
title: 'Undo-log completeness audit (netarch): 8 confirmed rewind/replay hole classes'
status: open
priority: 2
issue_type: task
depends_on:
  mtg-610: related
created_at: 2026-06-03T03:15:27.975463149+00:00
updated_at: 2026-06-03T03:15:27.975463149+00:00
---

# Description

Undo-log completeness audit (netarch rewind/replay) — 8 confirmed hole classes.

Source: exhaustive 8-category read-only audit + adversarial verify, run by team-lead on netarch-undo-holes @ 6ee19e63 (2026-06-03). 36 mutations flagged, 15 confirmed real, 21 false-flags filtered, deduplicated to 8 classes below.

INVARIANT being audited: every state mutation that affects a HASHED, non-excluded field MUST have a covering undoable GameAction (or be a documented per-turn transient blanket-cleared in rewind_to_turn_start). A mutation that is neither = an undo-log hole = the "no undo-log incompleteness" merge bar is not met. ALL fixes below are "complete the undo log" (log a covering GameAction), consistent with the rewind vision — NOT hash-exclusion (these are real gameplay state). Note which path each hits: turn-start rewind+replay (the network shadow path; hand-clears many transients) vs the PER-ACTION undo path (human undo / UndoTest oracle / MCTS mid-game rewind).

PRIORITIZED (by relevance to current target decks + the network path):

1. [HIGH — hits 1994/old-school target decks + rewind-verifier] ETB choice fields ride inside MoveCard, but MoveCard.undo (undo.rs:692-755) does NOT reset them:
   - card.tapped via ETB enters_tapped  — state.rs:1283
   - card.chosen_color (Thriving lands)  — state.rs:1300
   - card.chosen_player (Black Vise ETB "choose a player") — state.rs:1322
   On a mid-turn rewind the ETB MoveCard is undone (card leaves battlefield) but these fields stay stale on the off-battlefield card -> turn-start hash diverges. Fix: GameAction::SetChosenColor{card_id,prev} + SetChosenPlayer{card_id,prev} logged alongside the MoveCard; reuse TapCard for the ETB-tapped write. BLACK VISE is in 1994/old-school decks -> this is the most current-relevant hole.

2. [MED] Library-reorder class (raw library Vec mutation, no covering action; hashed by rewind-verifier; NETWORK hash excludes library order so NOT a live cross-machine desync, but breaks rewind-oracle + MCTS):
   - scry_apply_decision   — state.rs:1814-1833 (reorder only)
   - surveil_apply_decision — state.rs:1900-1919 (reorder + library->graveyard via RAW ops, not move_card)
   - Dig "rest to bottom"  — game_loop/priority.rs:3453-3459 AND duplicate actions/mod.rs:5790-5799
   Fix: GameAction::ReorderLibrary{player, previous_order} (+ previous_graveyard for surveil; route surveil's library->graveyard through move_card) mirroring ShuffleLibrary's previous_order restore (undo.rs:1080-1095). DRY: factor one shared undo-logging helper for all 4 sites.

3. [MED] Token creation — Effect::CreateToken, actions/mod.rs:4779-4796: raw battlefield.add (bypasses move_card) + next_entity_id advanced (state.rs:788, unlogged). On rewind the token leaks (stays in cards+battlefield) AND next_entity_id stays advanced -> forward replay creates a SECOND token with a higher id (dedup guard misses) -> duplicate token + hash divergence. Fix: GameAction::CreateEntity{card_id} whose undo clears the card, removes from battlefield, AND decrements next_entity_id. (next_entity_id round-trip is a general concern beyond tokens.)

4. [MED] Counter annihilation — add_counter card.rs:1302-1324 / add_counters state.rs:2012: P1P1/M1M1 annihilation cancels counters, but AddCounter logs the INTENDED amount/type; undo via remove_counter then can't restore annihilated counters (lost permanently). Fix: log the EXACT per-type delta actually applied (e.g. CounterDelta listing each (type, signed_amount)); undo reverses each.

5. [MED] Regeneration replacement — apply_regeneration_shield state.rs:670 -> combat.rs:115-128 remove_from_combat: the combat-removal + regeneration_shields decrement have NO covering action on the per-action undo path (turn-start rewind is safe — undo.rs:1516 hand-clears combat). Fix: GameAction::RegenerateReplaceDestroy{card_id, prev_combat: Box<CombatState>, prev_shields}.

6. [MED] source_prevention_shields (Circle of Protection) — push actions/mod.rs:5093, consume 8173-8182: unlogged. turn-start rewind blanket-clears it (safe), but per-action undo (UndoTest / human undo) desyncs. Fix: log push/consume GameActions.

7. [LOW — Avatar/Firebending decks only] combat_mana_pool — player.rs:201 add (Firebend) / 246+330 spend / combat.rs:576 clear: 3 unlogged mutations. turn-start rewind safe-by-accident (always None at a boundary), per-action undo desyncs. Fix: log AddCombatMana + clear/spend actions; OR add combat_mana_pool to the rewind per-player transient sweep (undo.rs:1596-1599) AND log for the per-action oracle.

8. [MED — multiplayer-commander only] has_lost via commander damage — combat.rs:957: SetCommanderDamage.undo restores commander_damage_taken but does NOT re-derive has_lost (ModifyLife.undo DOES, undo.rs:783-784). Only 3+ player commander (2-player ends the game so no further rewind). Fix: re-derive has_lost in SetCommanderDamage undo.

REGRESSION COVERAGE (why these went unnoticed): the rewind/replay oracle matrix (whole_game_rewind_replay_e2e.rs) uses simple_bolt/combat_test_4ed/Bazaar — no tokens, scry, dig, regen-in-combat, counters, COP, Firebend, or Black Vise. Each fix should add the relevant card class to the oracle matrix to lock in coverage.

Relates mtg-610 (netarch). This list IS the "no undo-log incompleteness" checklist — close all 8 (or explicitly justify any as out-of-scope) before netarch merge.
