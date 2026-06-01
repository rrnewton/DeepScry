---
title: 'NETARCH N4 follow-up: live WASM step_harness re-entry must rewind for the 10 kept TurnStructure guards'
status: open
priority: 2
issue_type: task
created_at: 2026-06-01T06:01:00.050375555+00:00
updated_at: 2026-06-01T06:01:00.050375555+00:00
---

# Description

Follow-up to netarch N4 (mtg-53okw, branch netarch-n4-guards). N4 removed only 2 of the 12 TurnStructure once-per-turn guards (attackers_declared_turn @86847b14; main1/main2_delayed_fired_turn @9b00fff0). The other 10 are NOT redundant: removing each produces a real, reproducible network desync, proving the live ai_harness step_harness rewind+replay does NOT actually suppress double-application of these once-per-turn INTERNAL state mutations on a WASM GameLoop re-entry.

REPRODUCERS (each: remove the guard, build network+wasm, run web/test_network_multideck.js --quick OR make validate):
- upkeep/end_step/draw_triggers_checked_turn (check_phase_triggers guard_slot): All Hallow's Eve upkeep trigger double-fires (RemoveCounter Scream x2) -> P2 hash mismatch, multideck rogerbrand mirror seed=3 (local +6 @turn13, action_count~741). This is the mtg-609 fix; still load-bearing.
- draw_step_executed_turn: P2 hash mismatch @Turn2 Draw step (action_count=86), single-deck network e2e random mode.
- turn_state_reset_turn: FATAL DESYNC Local abilities(1)!=server(0): [PlayLand] -- reset_turn_state re-zeroes lands_played_this_turn on re-entry; NOT undo-logged (game_loop/mod.rs reset_turn_state).
- blockers_declared_turn: P2 hash mismatch action_count=961 rogerbrand mirror (local 1 behind; re-entry consumes wrong ChoiceRequest).
- combat_first_strike_damage_dealt_turn / combat_first_strike_priority_done_turn / combat_damage_dealt_turn: P2 hash mismatch action_count=577 monored mirror seed=13 (local +1; combat damage / first-strike priority re-applied).

THE ACTUAL FIX: make the WASM step_harness re-entry path ALWAYS rewind-to-checkpoint+replay for these events (same mechanism snapshot/resume uses and N3 validated), so the once-per-turn mutation is reversed by the undo log before re-running. Likely gaps: (a) reset_turn_state mutations (lands_played, mana pool clear, loyalty flag) are NOT undo-logged; (b) begin-of-phase trigger firing + combat-damage application re-run on a re-entry that did not rewind. Investigate why these events re-enter WITHOUT a rewind while attackers/main-delayed re-enter cleanly.

GATE: web/test_network_multideck.js --quick (All Hallow's Eve rogerbrand mirror seed=3 sharpest canary) + full make validate. When all guards removable AND robots42 mtg-559 in-stack-resolution fixed, robots42 re-joins the network gate.

Refs: mtg-53okw (N4 tracker), mtg-610 (arch), mtg-559 (robots42 in-stack), mtg-609 (All Hallow's Eve), ai_harness.rs step_harness, undo.rs rewind_to_turn_start, game/phase.rs TurnStructure.
