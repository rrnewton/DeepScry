---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-02T19:39:54.432003632+00:00
---

# Description

NETARCH mtg-610 follow-up. Exposed once the 9 TurnStructure re-entry guards were deleted (commit 44c57bc2 on branch netarch-undo-holes) and the WASM AI path unified onto undo-log rewind+replay.

SYMPTOM: old_school/03_robots_jesseisbak seed=42 mirror in the WASM network gate FAILS intermittently (~1/6 standalone runs; varies in player + choice_seq). REAL cross-machine server desync (compute_view_hash), NOT a browser flake, NOT the rewind-verifier hash.

EVIDENCE (turn 28 Main1): Server Hands[7,7] Libs[45,49]; Client(WASM) Hands[7,12] Libs[45,44] => WASM drew 5 EXTRA cards for P2 at a MATCHING action_count (2303=2303) — content drift at equal count, not an action-count drift. Browser log (repeated): '[WASM RESUME] Failed to cast spell 47: Failed to pay mana cost: Insufficient total mana to pay 1U'.

HYPOTHESIS (localized): the pending_cast WASM-RESUMPTION path at mtg-engine/src/game/game_loop/priority.rs:625-792 (built for the OLD no-rewind re-entry) resumes an interrupted cast at the top of the priority loop, bypassing choose_spell_ability_to_play. Under rewind+replay a multi-draw spell (robots42: Ancestral/Wheel/Braingeyser/Timetwister/Mind Twist) that blocked mid-resolution appears re-driven BOTH by replay of its recorded ChoicePoint AND by the pending_cast resume -> double draw (+5). The 'Failed to pay mana' is the second attempt after mana already spent. Fix likely: suppress/reconcile pending_cast resume when in rewind+replay (GameLoop.replaying / fancy_tui.in_rewind_replay), or make pending_cast undo-logged/replay-reconstructed rather than a separately-surviving field.

REPRO: make build-network && make wasm-network; loop: cd web && node test_network_gui_e2e.js --deck decks/old_school/03_robots_jesseisbak.dck --seed 42 (fails ~1/6). The file-based undo dumps in debug/netarch-undo-dumps/ are EMPTY for this class (dump only fires on action_count mismatch, not equal-count hash mismatch) — enhancing the dump trigger to also fire on equal-count view-hash mismatch would help.

STATE: monored Earthbend Haste hole + robots42 CloneCard/PushExtraTurn holes FIXED & green; counterspells/rogerbrand green. This pending_cast/replay double-resolution is the last blocker to a fully-green guard-deleted gate.
