---
title: 'NETARCH: reveal-as-choice unification — key reveals by game action_count, revert the action_count exclusion'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T03:13:00.957496754+00:00
updated_at: 2026-06-04T21:13:49.757737420+00:00
---

# Description

## LAYER-4 RE-DECISION 2026-06-04 (slot01) — re-stamp IMPOSSIBLE (proven), need iii/iv/v
User picked re-stamp the eager OpponentChoice reveal at the reveal own-ac. IMPLEMENTED+tested+REVERTED=no-op. PROOF (DIAG_OPPREVEAL seed13): every opponent reveal found_revealcard=false — at OpponentChoice-forward time the move has NOT executed, the RevealCard (canonical own position) is NOT in the undo log yet, so the eager path can only use the CHOICE ac. Cannot label an event with a position that does not exist yet. Ex: Mountain card59 eager@choice_ac375 (found=false); real RevealCard@376; collect_reveals re-sends@376 bundled with recipient next request (after seq77@380) → behind cursor=380 → lost-delta fatal. Dual-stamp is REAL (375 eager vs 376 collect); fix is the OTHER direction: (iii) collect_reveals SKIPS cards already sent eagerly via OpponentChoice [server, share already-sent state]; (iv) send post-exec reveals eagerly at own-ac [bigger transmission-timing restructure]; (v) CLIENT drop late reveal for an already-revealed card by identity not ac [smallest, no native/server touch, weakens strict guard]. Reverted to clean 0e905f8e (probe+server changes out), exclusion untouched, 3 client fixes intact. AWAITING user re-decision before further code.

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

## STATUS 2026-06-04 (slot01-2) — MINIMAL LAZY PROTOCOL; user chose OPTION B; cycling KNOWN-RED until Phase 2
Branch rebased onto integration @1763ffa5 (tip a9285342), CLEAN (10 commits, 0 conflicts). NOT pushed.

CYCLING DESYNC DIAGNOSED (the layer-4 dual-stamp generalized): on the cycling/library-search path the
server stamps TWO DISTINCT state-sync deltas at the SAME game action_count — SearchCandidates{cards}
(at the search-CHOICE ac, server.rs ~2974-2978) AND RevealCard{reason:Searched, found-card} (ALSO at
that ac, the ChoiceAccepted self-search reveal, server.rs ~3117-3123). Repro avatar-draft Mountaincycling:
collision @ac=202. WASM (strict ac-keyed log) → push_state_sync "two DIFFERENT deltas share ac" panic
(client.rs:1291). Native (synthetic-keyed) → found card never reaches hand → true state divergence
(server P2 hand [41,70,73] vs client [70,73]) → off-by-one ac → FATAL hash mismatch. TRUE state divergence
(action_count already excluded from compute_view_hash @state_hash.rs:415, hash still mismatches).
PRE-EXISTING in slot01 (fails pre- AND post-rebase identically); origin/integration PASSES (byte-identical).
Root = SERVER ac-SOURCE (MEDIUM-6 generalized: two distinct deltas must never share an ac). Buffer
rearchitecture reuses these ac sources, so correct buffer-acs FIX it → Option B premise CONFIRMED.

KNOWN-RED until Phase 2 (tracked per team-lead rail #1, NOT a paper-over):
 - validate step network.equiv-random (tests/network_vs_local_equivalence_e2e.sh 3 random)
 - validate step network.equiv-zero  (tests/network_vs_local_equivalence_e2e.sh 3 zero)
 network.equiv-heuristic PASSES (heuristic avoids the cycling line).
Coverage gap that hid it: web/test_network_multideck.js has ZERO cycling decks; native lockstep oracle
used Hallows (no cycling). Phase 2 adds a permanent WASM cycling scenario.

MERGE DISCIPLINE (rails): integration STAYS GREEN — NO merge while cycling red. Phases 0-1-2 accumulate
on branch. FIRST integration merge only when FULL make validate (incl equiv-random/zero) green = Phase 2 end.
PHASE 2 = the gate + proof: native shim + correct buffer acs MUST turn cycling green, else HARD STOP (Option C).
Interim gate for Phases 1-2: WASM multideck + lockstep oracle.

PLAN: ai_docs/NETARCH_minimal_protocol_PLAN_20260604.md. Sequence: diagnose(done)→Phase1(additive buffer,
dual-emit, prove buffer-drives-WASM on interim gate)→Phase2(native shim→cycling green)→ff-merge→Phase3-4.

## PHASE 1 COMPLETE 2026-06-04 (slot01-2) — additive buffer; buffer alone drives WASM (interim gate GREEN)
Implemented the additive minimal-lazy buffer (dual-emit), proved buffer-drives-WASM:
- protocol.rs: BufferedFact enum {Reveal,LibraryReorder,SearchCandidates,Choice} + buffer:Vec<(u64,BufferedFact)>
  field on ServerMessage::ChoiceRequest (#[serde(default)]).
- server.rs: handler accumulates eager OpponentMadeChoice + LibraryReordered (Phase-1 dual-emit) and
  assemble_choice_buffer() builds the single ascending-ac catch-up buffer (reveals from choice_request.reveals
  at own ac — NO eager opponent-cast re-emit, killing the dual-stamp; reorders from coordinator broadcast
  (BLOCKER 2); choices from retained OpponentChoiceInfo (BLOCKER 1) + dummy Searched reveal for opp searches).
  Eager messages STILL sent (native consumes them; native ignores buffer via serde default).
- client.rs (WASM): buffer_is_authoritative flag set on first ChoiceRequest; apply_choice_buffer() routes the
  buffer into state_sync + opponent_choices; eager CardRevealed/LibraryReordered/SearchCandidates/OpponentChoice
  arms IGNORED once authoritative (opening-hand/initial pre-first-choice still processed). This makes the buffer
  the SOLE mid-game source → eliminates the eager opponent-cast dual-stamp at the WASM consumer.

INTERIM GATE GREEN:
- WASM multideck 4/4 PASS (monored s13 [the layer-4 dual-stamp repro], counterspells s5, rogerbrand s3 combat,
  robots s42 clone/balance) — buffer alone drives WASM, no desync/monotonicity panic.
- netarch_lockstep_oracle_e2e PASS (control_seed_03, class_a_seed_02). native equiv-heuristic PASS (18/18 identical).
- native clippy -Dwarnings clean, fmt clean, wasm-network bundle builds. (wasm-network clippy lints are pre-existing,
  NOT in MY code, and NOT CI-gated — validate's clippy-wasm lints wasm-tui only.)
- make validate --keep-going: 12 steps pass, ONLY cycling fails (see known-red). agentplay determinism FAIL was
  FLAKY (empty log under heavy-parallel resource contention; passes 3/3 in isolation; network changes can't affect
  local mtg tui).

KNOWN-RED until Phase 2 (cycling SearchCandidates+Searched ac collision), tracked:
 - validate network.equiv-random, network.equiv-zero
 - unit.nextest shell_scripts__cycle_ability_network_sync_e2e (mtg-420, seed 315)
 - unit.nextest shell_scripts__network_vs_local_equivalence_e2e
 cycle_ability_network_sync_e2e PASSES on bare integration 1763ffa5 (29/29 IDENTICAL) → it is slot01's cycling
 regression, NOT pre-existing (predecessor's "pre-existing on integration" note was outdated/wrong). All 4 are
 Phase-2 merge-gate items.

DISCIPLINE: NOT merged (rails: integration stays green; first merge only at Phase-2 full-validate-green). Branch
checkpoint committed + pushed (force-with-lease, post-rebase, authorized). NEXT = Phase 2: native shim unpacks
buffer into MVar/sync_to_action (+ reorders, reveals-before-choice ordering) AND correct buffer acs (SearchCandidates
vs Searched-resolution distinct ac) → cycling GREEN = full validate green = merge gate.

## STATUS 2026-06-04 (slot01-phase2) — searcher-side dual-stamp FIXED @4db6e056; 2 bigger blockers found
WORKTREE CONFLICT RESOLVED: slot01-2 was a runaway (still editing this worktree after handoff); team-lead killed it. slot01-2's client.rs "dedup-by-choice_seq band-aid" was DISCARDED (inferior client-only strategy). I own slot01 exclusively now.

COMMITTED @4db6e056 (searcher-side fix, PROVEN): dropped the redundant eager ChoiceAccepted found-card reveal (server.rs ~3147). It re-stamped the found card at the search-CHOICE ac = same ac as SearchCandidates = the dual-stamp. The found card is ALREADY delivered at its TRUE resolution ac by collect_reveals_since_last_choice (generic flush server.rs ~2933). Re-stamping "via ChoiceAccepted" is IMPOSSIBLE (move not executed at forward time — same wall as layer-4). So DROP (option iii), not re-stamp. PROOF: cycle_ability_network_sync_e2e seed315 GREEN 3/3 isolated (was RED); network.equiv-random GREEN; equiv-heuristic GREEN.

BLOCKER A (native, search-family, NOT the dual-stamp): equiv-zero seed3 now advances past the old turn-5 failure to a turn-11/12 cycling desync. ROOT: the SEARCHER's own shadow has valid_cards.len=0 at choose_from_library (client2/Gabriel Plainscycle turn12, found card 46 NEVER instantiated in shadow) → fetched card can't reach hand → hand off-by-one → fatal hash mismatch. This is a DIFFERENT facet: the searcher's shadow library doesn't materialize the fetched candidate. cycle315 (random, P0 searcher) does NOT hit it, so it's path/seed-specific (P1 mirror and/or zero-controller and/or candidate-instantiation). Needs its own fix.

BLOCKER B (WASM foundation, INHERITED from Phase 1 — bigger): Phase-1 "buffer drives WASM (multideck 4/4)" was a FALSE POSITIVE (ran vs a STALE pre-buffer server binary — confirmed slot01-2's retraction). FRESH build (server@4db6e056 + wasm@a23354b7): multideck 0/4, deterministic RefCell "already borrowed" PANIC at wasm/network/exports.rs:30 (with_client borrow_mut) + fancy_tui.rs:129. Re-entrancy: on_message() holds the client borrow_mut (exports.rs:221) then drives the shadow replay / apply_choice_buffer which re-borrows the same client. multideck + gui are in `make validate`, so this blocks the merge gate independent of cycling.

NEXT (team-lead deciding allocation): (1) Blocker B = real WASM buffer-foundation work (re-entrancy + likely the apply-frontier stall slot01-2 flagged); (2) Blocker A = searcher-shadow candidate materialization; (3) then native buffer shim + WASM cycling coverage; (4) merge gate.
