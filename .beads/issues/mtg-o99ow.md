---
title: 'NETARCH: reveal-as-choice unification — key reveals by game action_count, revert the action_count exclusion'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T03:13:00.957496754+00:00
updated_at: 2026-06-04T11:46:29.689698576+00:00
---

# Description

## STATUS 2026-06-04 (slot01 finisher) — WASM bug#2 has 4 layers; 3 client layers DONE+pushed @bd788773; layer-4 = HARD STOP (server-side design Q)

Committed+pushed bd788773: arrival-order-independent state-sync log (client-only).
SAME-AC AUDIT = PASS/distinct (ac==undo_log.len() unique per action; dup-ac ⇒ same delta re-sent).
LAYERS FIXED (client + ActionLog primitive):
 1 OUT-OF-ORDER ARRIVAL → ActionLog::insert_sorted (re-sort by ac).
 2 CURSOR PASSES UN-ARRIVED GAP → apply bounded by max_received_choice_ac (completeness watermark = principled L4 block-on-miss, client-only).
 3 IDEMPOTENT RE-SEND (shared_reveal_index immediate-pusher + collect_reveals) → push_state_sync dedups dup-ac via state_sync_entries_equivalent (drop same delta; DIFFERENT delta @same ac = fatal; new delta behind cursor = lost = fatal; cmp >= since opening_reveal_ac(0)==0).

HARD STOP — LAYER 4 (server-side dual-ac-stamp): the SAME opponent-cast reveal is stamped at TWO acs by two server paths. P1 casts Obliterating Bolt (seq77@380): reaches P2 via (A) OpponentChoice path server.rs ~3040 (mtg-610 "bundled") @ choice ac 380, reason Played (INSTANTIATION for remote_controller replay) AND (B) collect_reveals_since_last_choice @ its OWN ac 376. Two distinct acs for one reveal ⇒ second is lost-delta fatal (dedup-by-ac can't catch two acs). Clean fix is SERVER-side (one ac per reveal): (i) stamp OpponentChoice reveal at own ac (path lacks it), (ii) drop OpponentChoice reveal + rely on collect@376 (timing risk: card must be instantiated before remote_controller replays the cast), or (iii) collect_reveals skips reveals already sent by OpponentChoice. ALL touch LOCKED mtg-610 + SHARED native/server code (native GREEN, must not regress). Blanket client "apply-ASAP" unsafe (late draw-reveal after intervening reorder desyncs). DEFERRED to team-lead/user.

EXCLUSION LEFT IN (state_hash.rs untouched, probe reverted). NOT the closing commit.
E2E: monored seed13 now reaches deep turn-4 with client_hash==server_hash everywhere (layers 1+2 cleared); counterspells seed5 PASSES e2e. Repro: cd web && node test_network_multideck.js --quick.
NEXT: pick server dual-stamp fix (i/ii/iii) → re-run un-excluded full canary (native sweep + WASM multideck + 4 DIVERGED + cycle + make validate) → exclusion-revert closing commit.

NETARCH reveal-as-choice unification (branch netarch-reveal-actionlog-unify). Principled successor to the action_count exclusion (state_hash.rs); CLOSING commit reverts that exclusion + restores action_count as a cross-replica invariant. Design: ai_docs/REVEAL_ACTIONLOG_UNIFICATION_DESIGN_20260603.md + SEARCHED_REVEAL_SUBSUMPTION_CODESIGN_20260603.md. Detailed live recipe: worktree debug/4a_impl_plan.md (slot01).

## STATUS 2026-06-04 (slot01-3) — branch @025e9eba pushed (3 commits on d0e288ba)
DONE+PUSHED: L1 protocol, L2a reorder-ac threading (prior), then:
- 1baac7dc L2bcd+L3: server emits opening-hand reveals at real per-draw ac (3*k), shuffle_library emits LibraryReordered at ShuffleLibrary ac, SearchCandidates as one Vec entry at search ac (native expands to N reveals); WASM client state_sync keyed DIRECTLY by game ac (deleted next_state_sync_ac/state_sync_effective_ac/state_sync_unstamped/push_state_sync_stamped/stamp_pending_state_sync/effective_ac_of); apply_state_sync_at(target) replaces greedy up_to_frontier; initial_library_orders buffer for ac-0 game-start (two-per-client collision); searched_card_for+unwind read key directly; L4 RED test pins Searched-dummy resolution-ac selection.
- b744b112: qualify crate::core::CardId for wasm-network feature.
- 025e9eba L2c fix: opening-hand reveal index = cards * OPENING_DRAW_UNDO_ACTIONS(3) ACTION span (was CARD count → re-collected dups → fatal push).

GATE PROGRESS:
- NATIVE un-excluded GREEN (acceptance prize): netarch_lockstep_oracle_e2e full 13-seed sweep PASSES with action_count RE-INCLUDED in compute_network_state_hash — CLASS_A [1,2,5,6,7,9,11,18,19,20] (incl mtg-yexvc 2/5/1/6) + CONTROL [3,13,16] (Hallows-3). native_wasm_equiv_sweep 15/0 DIVERGED.
- WASM networked (test_network_multideck): NOT green. ONE remaining bug (#2): mid-game ActionLog::push panic last=380,new=376. client_hash==server_hash at every surrounding choice => NO LOGIC DESYNC; purely a client log-STRUCTURE violation — server emits state_sync via 2 uncoordinated paths (coordinator LibraryReordered broadcast + handler reveals loop) so entries can ARRIVE out of ac-order; game-ac push requires strictly-increasing arrival. NOT a race (no L4 NeedsInput), NOT native-block HARD STOP.

## NEXT STEP (resume here)
Fix bug #2 (recommended option a): make the WASM client state_sync log tolerant of out-of-order ARRIVAL — insert sorted by ac (ActionLog get/iter/frontier already use a sorted Vec); add an insert-sorted method used ONLY for state_sync (NOT opponent_choices); PANIC only on EXACT-dup ac (genuine collision) and assert inserted ac > last_applied_state_sync_ac (else a needed entry arrived after the cursor passed). FIRST audit: can a reorder + a reveal land at the EXACT same ac (scry-reveal+ReorderLibrary, or shuffle reorder_ac coincident with a reveal)? Δ4 here suggests distinct, but confirm; if a true same-ac collision exists it is a genuine atomic-multi like SearchCandidates → combine or (ac,subseq). Option b (server ac-sorted merged send) is the alternative.
Then rebuild wasm + re-run test_network_multideck + 4 DIVERGED legs + cycle test + full make validate UNDER systemd-run --user --scope, ALL with the un-exclusion probe, before the closing commit.
Also: re-confirm cycle_ability_network_sync_e2e seed315 under a clean scope (pkill 'mtg server|http.server|chromium'; run under systemd-run --scope) — pre-existing FATAL on bare integration 4d841c33 (bisected, NOT 4a); team-lead expects mtg-ibj22 port-collision false-positive. pass=false-positive (not blocker); fail=real mtg-420 regression.

## CLOSING COMMIT (only when native+WASM un-excluded-green AND clean scoped make validate)
In state_hash.rs compute_network_state_hash, after the current_step().as_hash_u32().hash line, ADD: (view.action_count() as u64).hash(&mut hasher);  and delete the 'DELIBERATELY EXCLUDED' NOTE block (~415-427). Flip the state_hash.rs RED test that asserts the exclusion.

## LOCKED RULINGS
- SearchCandidates = ONE StateSyncEntry{searcher,cards:Vec<CardReveal>} at the search-RESOLUTION ac.
- Searched-dummy STAYS at the search-resolution ac (load-bearing for searched_card_for; never re-stamp earlier → mtg-mb668 regress). RED test pins it.
- Distinct-ac per delta; SearchCandidates is the only atomic-multi.
- desync ALWAYS fatal; never paper over. 4b (native game-ac keying + wait_for_state_sync_frontier block) is a SEPARATE later unit; if a NATIVE canary requires it → HARD STOP, exclusion stays.
