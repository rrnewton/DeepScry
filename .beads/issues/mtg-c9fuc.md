---
title: 'NETARCH N4 follow-up: live WASM step_harness re-entry must rewind for the 10 kept TurnStructure guards'
status: open
priority: 2
issue_type: task
created_at: 2026-06-01T06:01:00.050375555+00:00
updated_at: 2026-06-01T06:48:17.410193787+00:00
---

# Description



--- N4 skeptic (a9db6f77) MERGE-OK follow-ups ---
A. combat.rs ~42-48 comment MISLEADING: real attackers_declared_turn removal safety = WasmNetworkLocalController::choose_attackers has_pending_submission() gate + RNG-determinism (same choice=vec![0]=pass), NOT 'rewind clears CombatState' (that's snapshot/resume). Only safe for AI controllers (Random/Zero/Heuristic); a HUMAN WASM controller would still need the guard — ties mtg-uzvu4. Fix comment so future maintainers don't remove remaining guards for the human path.
B. main1/main2_delayed_fired_turn removal mechanically safe (check_delayed_triggers_on_phase removes trigger from store before firing, undo-logged → re-entry no-op) but NOT exercised by any Mode$Phase delayed trigger (Mana Drain) on the LIVE WASM-network path (robots42 excluded, mtg-559). ADD a Mana-Drain network e2e once mtg-559 fixed to close coverage gap.
