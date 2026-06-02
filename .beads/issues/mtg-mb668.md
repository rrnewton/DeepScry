---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-02T21:01:22.116102983+00:00
---

# Description

STATUS 2026-06-02 (agent-teams netarch-dev VERIFICATION — fix does NOT hold; precise root cause found):

VERIFY RESULT on clean 19c10c3a (pending_cast deleted): robots42 seed=42 WASM gate = 7 PASS / 3 FAIL of 10 (~30% desync), NOT green. The prior "1 green pass" was luck. Three distinct desync signatures, all = WASM shadow (P2 view) diverging from server under rewind+replay of HIDDEN-info events:
 (1) Demonic Tutor library search: server undo-log Choice(LibrarySearch(Some(idx)))->RevealCard(Island)->MoveCard(Lib->Hand)->Shuffle(38); WASM replay Choice(LibrarySearch(None))->Shuffle(39) — fetched NOTHING.
 (2) Mass-draw (Timetwister/Wheel) CONTENT divergence at EQUAL counts (Hands[7,7] Libs[50,50]) — replay shuffle/draw not byte-reproducing server's forward result.
 (3) Available-actions divergence ("Local abilities 3 != server 6") — shadow hand content already diverged.

CODE-CONFIRMED ROOT CAUSE for (1): Demonic Tutor = SP$ ChangeZone | Origin$ Library -> effect_converter.rs:456 makes Effect::SearchLibrary { player: PlayerId::new(0) /*placeholder*/ }. Placeholder routes to the INTERACTIVE path priority.rs:1720/1758 (NOT the non-interactive actions/mod.rs:4600). At priority.rs:1776 the ChoicePoint records a POSITIONAL INDEX: chosen_index = chosen_card_opt.and_then(|id| valid_cards.iter().position(|&c| c==id)). On the OPPONENT's shadow (P2 viewing P0's search) P0's library cards are hidden/uninstantiated, so valid_cards is empty and the fetched card isn't in it -> position() = None -> records ReplayChoice::LibrarySearch(None). In FORWARD play the correct count change still lands because it comes from the server's broadcast MoveCard (external sync), NOT the shadow's own search logic. Under REWIND+REPLAY that broadcast is undone and only the recorded None is replayed -> the fetch is lost -> P0 library 39 vs server 38 -> P2 view-hash mismatch.

state_sync (client.rs ActionLog<StateSyncEntry>) only carries RevealCard + LibraryReorder; a hidden opponent library-search MOVE is not represented there, and the card identity is never revealed to P2, so neither the index nor a CardId is reconstructable on P2's shadow from recorded data alone.

FIX DIRECTION (NOT yet implemented — awaiting scope decision): the rewind+replay of an opponent's hidden library search must reproduce the count-change move that forward play got from the server broadcast. Either (A) record/replay the authoritative move via state_sync so it survives rewind (preferred — matches non-destructive state-sync design), or (B) instantiate opponent library as opaque placeholders so the interactive search picks a stable placeholder index. Do NOT re-add a bypass. Signatures (2)/(3) are the same class (replay of hidden draws/shuffles) and likely need the same state-sync-driven replay, or RNG-state capture in the undo log for shuffles.

===== PRIOR CHECKPOINT BELOW =====

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
