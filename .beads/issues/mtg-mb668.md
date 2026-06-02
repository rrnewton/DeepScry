---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-02T20:10:15.405563166+00:00
---

# Description

STATUS 2026-06-02 (SESSION-RESTART CHECKPOINT — fix implemented, verification incomplete):

FIX IMPLEMENTED per user directive on branch `netarch-undo-holes` @ commit **87a220e3** (PUSHED to origin). `pending_cast` was DELETED ENTIRELY — the field + all set-sites + the WASM-resume bypass block at priority.rs ~625-792 (−179 lines); `pending_cast` references remaining = 0. On replay the interrupted cast is now re-driven by the NORMAL control flow (choose_spell_ability_to_play → mode selection → target selection), each step reading its recorded ChoicePoint from the ActionLog, reaching the same mid-cast point deterministically — NO jump-forward bypass, NO double-resolve. (User's words: "there should be no hacky jump forward in the control flow on replay... it should reexecute all the same logic, backed by ActionLog reads, to reach the same point deterministically.")

VERIFICATION INCOMPLETE — got 1 GREEN robots42 seed=42 WASM-gate pass ONLY (the agent was stopped for a session restart before finishing).

RESUME PLAN (fresh agent-teams session), starting from 87a220e3 (worktree clean):
1. Loop robots42 seed=42 ~10× in the WASM gate to confirm the intermittent ~1/6 desync is GONE — one pass is NOT proof. (make build-network && make wasm-network; cd web && node test_network_gui_e2e.js --deck decks/old_school/03_robots_jesseisbak.dck --seed 42)
2. Verify a HUMAN mid-cast resume still works — the deletion removed the path the human mid-cast flow SHARED, so confirm the unified rewind+replay re-drives a human multi-step cast (modes+targets) correctly over the network.
3. If 10× shows any residual desync: the normal replay path has a remaining un-logged-choice hole at the mid-cast point — FIX THAT (log/replay-reconstruct the choice), do NOT re-add a bypass.
4. Then mtg-610 step 4 (unify run_network_mode_human_v2 + run_network_mode_ai_v2 into ONE controller-agnostic entrypoint), step 5 (human-path test closing mtg-4z4r9 — native WasmNetworkLocalController turn-2 own-drawn-PlayLand desync), step 6 (full `make validate` + MTG rules-review). Then coordinator merges to integration + redeploys.

DEPLOY note: a deploy of 6129694a to deepscry.net FAILED on the pre-deploy WASM-boot smoke = CPU-starvation flake under concurrent builds (NOT a regression — the netarch WASM decks boot green in-agent). Redeploy post-restart when CPU is free, or deploy once the verified fix lands.

===== ORIGINAL ROOT-CAUSE (retained) =====
NETARCH mtg-610 follow-up. Exposed once the 9 TurnStructure re-entry guards were deleted (commit 44c57bc2) and the WASM AI path unified onto undo-log rewind+replay.

SYMPTOM: old_school/03_robots_jesseisbak seed=42 mirror in the WASM network gate FAILS intermittently (~1/6 standalone runs). REAL cross-machine server desync (compute_view_hash), NOT a browser flake, NOT the rewind-verifier hash. Turn 28 Main1: Server Hands[7,7] Libs[45,49]; Client(WASM) Hands[7,12] Libs[45,44] => WASM drew 5 EXTRA cards for P2 at a MATCHING action_count (content drift at equal count). Browser log: '[WASM RESUME] Failed to cast spell 47: Failed to pay mana cost: Insufficient total mana to pay 1U'.

ROOT CAUSE: the pending_cast WASM-resumption path (priority.rs:625-792, built for the OLD no-rewind re-entry) resumed an interrupted cast bypassing choose_spell_ability_to_play. Under rewind+replay a multi-draw spell (robots42: Ancestral/Wheel/Braingeyser/Timetwister/Mind Twist) that blocked mid-resolution was re-driven BOTH by replay of its recorded ChoicePoint AND by the pending_cast resume -> double draw (+5). The 'Failed to pay mana' was the second attempt after mana already spent. FIX = delete pending_cast (done @87a220e3, see STATUS above).
