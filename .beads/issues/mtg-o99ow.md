---
title: 'NETARCH: reveal-as-choice unification — key reveals by game action_count, revert the action_count exclusion'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T03:13:00.957496754+00:00
updated_at: 2026-06-04T21:50:45.553624779+00:00
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

## REFINEMENT 2026-06-04 (slot01-phase2) — net-improvement CONFIRMED; Blocker B root = duplicate-ac push (RefCell panic is cascade); A likely subsumed by Piece 2
- NET-IMPROVEMENT PROVEN: built PARENT a23354b7 (pre-fix) → equiv-zero seed3 fails at TURN 5 (Mountaincycle ac=253). My fix @4db6e056 → advances to TURN 12 (ac=739). So the fix is correct and beneficial; turn-12 is a facet it EXPOSED, not caused.
- BLOCKER B TRUE ROOT (corrected): multideck monored s13 first panic = `ActionLog::push: action_count must be strictly increasing (last=1,new=1)` (action_log.rs:102) — the buffer-driven WASM pushes TWO state-sync deltas at the SAME ac=1 (game-start). The exports.rs:30 with_client "already borrowed" + fancy_tui:129 panics are CASCADE (first panic poisons the borrow). So B is the SAME dual-stamp class as cycling, at game-start. Fix = find/eliminate the duplicate-ac source in assemble_choice_buffer's early-game facts (+ the runtime assertion the brief wants IS effectively ActionLog::push).
- BLOCKER A likely SUBSUMED BY PIECE 2: A lives in the native SYNTHETIC-keyed reveal path (client.rs ignores server action_count for CardRevealed, re-keys via a monotonic counter). Piece 2 (native buffer shim) makes native consume the game-ac buffer, retiring the synthetic path → A resolved as a consequence. Dependency order: fix buffer foundation (B) → native buffer shim (Piece 2) → A resolved. Patching the synthetic path in isolation is likely throwaway.
- RECOMMENDED SEQUENCE: (1) Blocker B duplicate-ac in buffer (game-start) + any other same-ac reveal sources; (2) verify multideck buffer-driven GREEN; (3) Piece 2 native buffer shim; (4) re-verify all native cycling (incl equiv-zero turn-12 = A) GREEN; (5) WASM cycling coverage; (6) merge gate.

## HANDOFF SPEC 2026-06-04 (slot01-phase2) — clean checkpoint @bb26ae3f; verified RED baseline; B = fresh-context replay-driver work
CLEAN CHECKPOINT: bb26ae3f (= searcher fix 4db6e056 + beads), pushed to origin. Worktree clean. mtime-discipline verified (server+wasm built AFTER source; multideck/cycle ran on fresh binaries).
VERIFIED RED BASELINE (fresh server+wasm): multideck 0/4. FIRST panic = ActionLog::push last=1,new=1 (action_log.rs:102) = opponent_choices choice_seq=1 DOUBLE-PUSH. RefCell "already borrowed" (exports.rs:30, fancy_tui:129) are CASCADES of that first panic (wasm trap leaves the borrow held).

BLOCKER B FIX SPEC (do in fresh context — sacred replay-driver):
B1 (double-push, first panic): an eager OpponentChoice(seq=N) is processed BEFORE the first ChoiceRequest flips buffer_is_authoritative (client.rs:946 early-return), pushing seq=N; then that ChoiceRequest's buffer re-adds Choice(seq=N) via apply_choice_buffer (client.rs:1389) → second push of key=N → panic. FIX = make BOTH push sites dedup by choice_seq (idempotent), mirroring push_state_sync's by-ac dedup: route client.rs:973 (eager arm) AND client.rs:1389 (buffer apply) through a record_opponent_choice(entry) helper that drops if get(choice_seq).is_some(). slot01-2's helper (saved: scratch/slot01-phase2-20260604/full_worktree.diff lines ~212) is SOUND in spirit BUT uses opponent_choices.insert_sorted — which action_log.rs DOCUMENTS as state-sync-log-ONLY, NEVER the choice buffer (owner #1). RESOLVE: either justify insert_sorted here (new seqs are monotonic so it ≈ push; dedup handles the only out-of-order case) OR keep .push() and instead guard the buffer-apply site to skip seqs already present. ADD a debug_assert that the duplicate's choice_indices match the existing (desync-detection, not silent drop). Do NOT touch the eager-arm watermark bump (client.rs:958) as part of B1 — it's B2 territory.
B2 (apply-frontier STALL, the load-bearing one): after B1, expect multideck to advance to the diff=5 stall. apply_state_sync_at bounds apply by target.min(frontier).min(max_received_choice_ac) (client.rs:1568); when a whole bundle arrives at once, state_sync frontier = last REVEAL ac (e.g. 50) while the ChoiceRequest ac is higher (55) and a buffered opponent Choice sits at 54 — the shadow forward-replay won't advance PAST the last reveal to consume that choice. This realizes "L4 block-on-miss": shadow must BLOCK until it has the facts then replay forward to the CR action_count, past the last reveal. Driver locus (slot01-2): ai_harness.rs sync_callback + apply_state_sync_at + GameLoop run_until_input + WasmRemoteController NeedsInput trampoline. DESIGN INTENT: docs/NETWORK_ARCHITECTURE.md ~159-227; ai_docs/NETARCH_minimal_protocol_PLAN_20260604.md (L4 watermark); ai_docs/REVEAL_ACTIONLOG_UNIFICATION_DESIGN_20260603.md. Suspect the eager-arm watermark bump (client.rs:958) is errant per slot01-2 (bumping to opponent's larger ac moves cursor past un-arrived reveals → lost delta).

BLOCKER A (native, independent of B): equiv-zero seed3 turn-11/12, SEARCHER's own shadow has valid_cards=0 + never instantiates the fetched candidate (card 46) → off-by-one hand → fatal hash. team-lead: real shadow-replay correctness bug, fix regardless. NOT subsumed by Piece 2 per team-lead. Native eager path (client.rs ignores server action_count for CardRevealed, synthetic-keyed). My searcher fix is a NET IMPROVEMENT here (parent fails turn5 → fix advances to turn12), proven by building parent.

SEQUENCE (team-lead): reconcile[DONE] → verify RED baseline[DONE] → B1 → B2 (multideck GREEN, fresh binaries) → A → native buffer shim (Piece 2) → WASM cycling coverage → merge gate (full validate green incl 4 cycling + multideck + gui). mtime-discipline on EVERY network test. HARD STOP if family won't converge.

## B1 DONE 2026-06-04 (slot01-phase2) @9414e68e — double-push fixed + VERIFIED; B2 stall reproduced exactly
record_opponent_choice dedup-by-choice_seq landed (push, not insert_sorted; debug_assert on choice_indices; keep-first verified content-equivalent). Fresh mtime-verified wasm+server: the choice_seq=1 double-push panic is GONE. multideck still 0/4, now blocked solely by B2 apply-frontier stall: WASM_HASH_DEBUG ACTION COUNT MISMATCH server=55 local=50 (diff=5) at choice_seq=2 ac=55 (monored s13) — EXACTLY the predicted case (shadow stops at last reveal @50, 5 short of choice @55). B2 = the deep replay-driver fix (L4 block-on-miss; advance shadow forward-replay past the reveal frontier to the CR ac). Recommended for fresh context from @9414e68e.

## REFRAME 2026-06-04 (slot01-phase2) — "Blocker A" is NONDETERMINISM, not a cycling desync
Native-first pivot landed the WASM-disable (@b67d8428, mtg-zsi9f). Characterizing the native baseline revealed "Blocker A" (equiv-zero seed3) is NOT a deterministic cycling/buffer desync:
- equiv-zero seed3, 5x ISOLATED (systemd-run scope), LOW load (0.85), ZERO concurrent mtg procs: 2/5 PASS, 3/5 FAIL with VARYING diff (147/100/94 lines) → genuinely NONDETERMINISTIC, not contention.
- Failing runs have ZERO network hash-mismatches (no server↔client desync). The LOCAL game is STABLE (24 turns every run); the NETWORK SERVER game ends early + nondeterministically (6/11/12/24 turns). So the nondeterminism is in the NETWORK-SERVER game path, exposed by the ZERO controller (picks index 0 of what is likely a nondeterministically-ORDERED options/abilities list → mtg-725 try_get(None)/HashMap-ordering class).
- equiv-random seed3: 3/3 deterministic GREEN. equiv-heuristic: GREEN. robots42: 4/4 GREEN. cycle315: 3/3 GREEN (my searcher fix). So the CYCLING/buffer work is sound where deterministic; the only native red is the zero-controller network nondeterminism.
- My earlier single-run "Blocker A = searcher-shadow valid_cards=0 / found card not materialized" was a SYMPTOM of this nondeterminism (different library order → different/empty search), NOT a deterministic bug.
OPEN QUESTION (needs an integration build to settle): is this network-server nondeterminism PRE-EXISTING (mtg-725, orthogonal to the branch) or BRANCH-INTRODUCED? The branch changed what's TRANSMITTED (reveals/buffer), not the server's game-choice logic, so PRE-EXISTING is most likely — but unverified. The native-first "rock solid" gate cannot be green while equiv-zero is nondeterministic; that's a separate nondeterminism root-cause (mtg-725 class), not buffer work.
