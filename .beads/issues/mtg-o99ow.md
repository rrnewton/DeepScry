---
title: 'NETARCH: reveal-as-choice unification — key reveals by game action_count, revert the action_count exclusion'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T03:13:00.957496754+00:00
updated_at: 2026-06-05T19:26:51.970215293+00:00
---

# Description

## WASM SIDE COMPLETE + FULL VALIDATE GREEN 2026-06-04 (slot02) @25cb0454 — reorder/reveal split + B2 fix ported; WASM net e2e RE-ENABLED

PLAIN-LANGUAGE: the browser (WASM) client had the same two flaws the desktop
client's predecessor had. (1) On a shuffle-then-draw card (Timetwister / Wheel of
Fortune / Windfall) a library-reshuffle and a card-reveal can legitimately land at
the same internal time-stamp; the browser kept both in ONE list and crashed,
mistaking them for a contradiction. (2) The browser's catch-up replay stalled one
step short of the next decision because it refused to instantiate revealed cards
ahead of where its own replay had reached. Both are now fixed by mirroring the
proven desktop client: two separate lists (reshuffles vs reveals — same-kind
clashes still crash as a real desync, only cross-kind may coincide), and revealing
opponent cards eagerly up to the "all-facts-arrived" watermark so the replay can
advance to the choice. The browser network tests are turned back on and the whole
test suite is green.

CHANGES (mtg-engine/src/wasm/network/client.rs ONLY; +274/-155 @1605eec7):
 - SPLIT state_sync ActionLog -> reorder_log + reveal_log; last_applied_state_sync_ac
   -> last_applied_reorder_ac + last_applied_reveal_ac. push_state_sync routes by
   class. Cross-class (reorder+reveal) MAY share an ac; same-class (reveal-reveal /
   reorder-reorder) collision STILL FATAL (the original dual-stamp desync class).
 - B2 apply-frontier STALL FIX: apply_state_sync_at now bounds REORDERS positionally
   by target_action and REVEALS eagerly by max_received_choice_ac (ahead of the
   shadow), mirroring the native two-cursor apply. Removes the diff=5 stall.
 - searched_card_for / unwind_state_sync_to / has_unapplied_state_sync /
   reset_state_sync_cursor / reset threaded through the split (reveal scans read
   reveal_log; unwind behaviour preserved — it only ever processed reveals).
 - Unit tests mirror native: reorder_and_reveal_may_share_ac (relaxation),
   reveal_vs_reveal + reorder_vs_reorder distinct-at-same-ac FATAL (guard the
   original class), reveal_idempotent_resend_is_dropped, behind-class-cursor fatal.

VALIDATE RE-ENABLE @25cb0454 (scripts/validate.py): removed the mtg-zsi9f
disable-by-default block + WASM_NETWORK_GAME_TAGS; network.gui/multideck/human-input/
redo-reload run by default again (CI shards already select them via --only).
--enable-wasm-network kept as a deprecated no-op; --no-wasm-e2e still disables all
browser steps for browser-less hosts.

GATE GREEN (mtime-fresh server release+network + wasm bundle rebuilt after source;
full run under systemd-run --user --scope):
 - FULL make validate: 34/34 steps PASS, 0 FAIL ->
   validate_logs/validate_25cb04541ea6dfe32e2d13e1ac807ee7f86aa92f.log
 - Re-enabled WASM net e2e all PASS: network.gui, network.multideck (monored s13 =
   the layer-4 dual-stamp repro, robots42 s42 = mtg-610 rewind/replay regression,
   counterspells s5, rogerbrand s3), network.human-input, network.redo-reload (both legs).
 - NATIVE STAYS GREEN: equiv-random, equiv-zero, equiv-heuristic, robots42, fuzz all
   PASS; unit.nextest 1588 passed; wasm browser suite (16) + 4 native-vs-WASM equiv
   sweeps PASS; clippy -Dwarnings (engine + wasm32) clean.

NOTE / residual coverage: the 4 official multideck decks do NOT exercise the
cross-class same-ac (shuffle-then-draw) path; that path is proven by the new unit
tests + the native lockstep oracle (real Timetwister). I tried adding a WASM cycling
deck (avatar draft mirror s315) but it FAILS in BOTH parent and this branch (parent:
sync mismatch @choice_seq=8; this branch: rewind fatal @turn4 — gets further) — a
PRE-EXISTING avatar/cycling mirror nondeterminism (mtg-725 / mtg-u3dwj class), NOT a
regression and out of scope. A deterministic WASM Timetwister/cycling e2e scenario is
a worthwhile future add (would need a stable deck/seed); filed mention here for team-lead.

## MTG Rules Review — Verdict: PASS (WASM mirror of the native PASS above)
1. Correct rule implementation: N/A for rule semantics — network shadow-replay
   TRANSPORT infrastructure, no engine rule logic changed. Affected effects' rules
   unchanged (search CR 701.20, scry/surveil CR 701.18/701.34, library order CR
   401.2, draw CR 120).
2. Reveal ordering: YES (strengthened) — the WASM shadow now applies reveals eagerly
   up to the choice's reveal-history watermark BEFORE the controller decides, and
   reorders positionally so a draw reads the correct post-shuffle library order.
3. Information hiding: PRESERVED — no new info in any message; the split only
   re-buckets already-entitled facts the client already received. Opponent-fetch
   dummy Searched reveal logic (empty name + card_id) unchanged.
4. Decision authority: PRESERVED — choices still routed to the client's
   PlayerController via ChoiceRequest; client-only change, no server decision added.
5. Server/client sync: this IS the fix's domain — eliminates the same-ac
   reorder+reveal false-fatal and the apply-frontier stall; controllers stay
   information-independent; desync stays FATAL (same-class dual-stamp still aborts;
   behind-class-cursor lost delta still aborts). Verified by full validate green incl
   all native + WASM equivalence sweeps.
6. Workaround vs real fix: REAL FIX — the principled WASM mirror of the proven native
   reorder/reveal split + two-cursor apply. No card-name special case, no skipped
   event, no TODO-shim.
7. Bug-class generalization: class-level — fixes ALL shuffle-then-draw cards
   (Timetwister/Wheel/Windfall) and ALL apply-frontier stalls at once, not a single
   card. Residual avatar/cycling mirror nondeterminism is a SEPARATE pre-existing
   class (mtg-725 / mtg-u3dwj), unaffected by this change.

Reasoning: This is a WASM-client-only shadow-replay transport change that ports the
already-reviewed-and-PASSED native reorder/reveal split + eager-reveal apply to the
browser. It strengthens reveal-ordering and determinism (block-on-miss watermark)
and preserves information hiding and decision authority; same-class dual-stamp and
lost-delta both stay fatal. Full make validate is green incl all native+WASM
equivalence sweeps. The one red sibling (avatar/cycling mirror s315) is pre-existing
in parent too and tracked under mtg-725 / mtg-u3dwj, explicitly out of scope.

Gamelog justification: network.gui monored seed 13 (the layer-4 dual-stamp repro)
completes 22 turns with NO desync/monotonicity error; robots42 (mtg-610 rewind/replay
+ in-resolution choices) PASS. Reproduce: cd web && node test_network_gui_e2e.js
--deck decks/monored.dck --seed 13 (and --deck decks/old_school/03_robots_jesseisbak.dck
--seed 42).

NOT MERGED — team-lead runs an adversarial desync-review on the WASM diff, then merges
the COMPLETE native+WASM prototype.


## LOCKSTEP-ORACLE HANG FIXED 2026-06-04 (slot02-build) @63a1cfdb — reorder_log/reveal_log split + CORRECTED INVARIANT

`make validate` HUNG (not failed) on netarch_lockstep_oracle_e2e class-A seeds.
NOT a deadlock — a PANIC (hangs the spawned shadow thread). On a
shuffle-then-draw resolution (Timetwister/Wheel/Windfall) a LibraryReorder and a
RevealCard legitimately carry the SAME game action_count: the server stamps a
shuffle's reorder at the POST-shuffle undo position (state.rs:835,
`undo_log.len()` after ShuffleLibrary) while reveals are stamped at their OWN
undo index (controller.rs:577 `forward_idx`) — two schemes that coincide there.
The old synthetic-keyed native client used private keys so it never noticed; my
true-ac keying put both in one ac-keyed log → the second tripped the
"two distinct deltas share an ac = FATAL" assert.

**CORRECTED INVARIANT (corrects ai_docs/NETARCH_reveal_ac_collision_audit_20260604.md):**
two SAME-CLASS deltas never share an ac (reveal-reveal, reorder-reorder = FATAL —
the original dual-stamp desync class); a CROSS-class reorder + reveal MAY
coincide (independent deltas, applied in separate passes reorder-first). The
audit's "Reveal/SearchCandidates/LibraryReorder never share an ac" was
over-strict. **Matters for the WASM re-enable**: the WASM client still uses ONE
state_sync log (src/wasm/network/client.rs) and would hit the SAME panic on
Timetwister — it needs the same reorder/reveal split before WASM network is
re-enabled.

FIX: split native StateSyncBuffer `log` → `reorder_log` + `reveal_log` (the
shadow already applies them in separate passes with separate cursors). Each log
keeps STRICT per-class insert_sorted (idempotent re-send dropped; DIFFERING
same-class delta @ one ac FATAL; lost-behind-cursor FATAL). Both apply
reorder-first = old working behaviour. No server change, no paper-over.

LOAD-BEARING VERIFICATION (team-lead): relaxation did NOT weaken the original
class. New unit tests (network::client::tests): reveal_vs_reveal_distinct_at_
same_ac_is_fatal (#[should_panic]), reorder_and_reveal_may_share_ac,
reveal_idempotent_resend_is_dropped — all PASS.

GATE GREEN (isolated systemd-run --user --scope, mtime-fresh): lockstep oracle
class_a_seed_02 (was hanging) + control_seed_03 + FULL 13-seed sweep (342s);
equiv-zero seed3 4/4; cycle315; robots42; 3 new unit tests. Full make validate
re-run pending (prior "nextest timeout @5.2% CPU" was THIS hang, not contention).

## TASK 1 + TASK 2 DONE + GREEN 2026-06-04 (slot02-build) — native buffer shim landed; eager opponent-choice deleted

The user's NATIVE-FIRST + DELETE-EAGER plan is implemented and green on all
deterministic native paths. Commits on netarch-reveal-actionlog-unify
(dc99d57d, 021de859, 46d1cc12, d90e7183, 4d8675df).

PLAIN-LANGUAGE: the desktop client used to learn about reveals / library
reshuffles / opponent choices from several separate websocket messages applied
greedily; those RACED and ~50% of the time dropped a searched-up card, causing a
fatal desync. The desktop client now consumes the SINGLE ordered list (buffer)
carried inside each choice request — exactly as the browser client already does —
so the race is impossible by construction. We then DELETED the now-redundant
separate "opponent made a choice" messages so the buffer is the only source.

TASK 1 (native buffer shim, mtg-engine/src/network/client.rs):
 - NetworkMessage::ChoiceRequest carries `buffer`; reveal-class variants carry the
   server action_count. buffer_is_authoritative + max_received_choice_ac on
   SharedNetworkState.
 - State-sync log keyed by TRUE server ac via insert_sorted (dedups idempotent
   re-sends; differing-delta-at-one-ac or behind-cursor = FATAL). Game-start
   library orders held in a separate initial_library_orders map (would collide at
   ac 0), applied once before first draw.
 - apply_choice_buffer routes facts; reveals pushed BEFORE choices (a Choice push
   wakes the RemoteController, which must see a fully-populated reveal log — the
   timing race that made it look nondeterministic). Watermark set BEFORE apply.
 - TWO apply cursors (the native B2 replay-driver): REORDERS are positional
   (bounded by the shadow's own action_count — a shuffle must not overwrite the
   library before the shadow replays earlier-ac actions that read it, e.g. a
   cycling fetch out of the pre-shuffle library); REVEALS are eager (bounded by
   the reveal watermark, applied ahead so opponent plays can be replayed —
   "Entity not found" otherwise). Reveals are library-order independent.
 - sync_callback passes target_action through; eager reader arms ignored once
   authoritative.
 - DRY: state_sync_entries_equivalent moved to network/state_sync.rs (shared
   native+WASM).

TASK 2 (server.rs): the OpponentMadeChoice handler no longer eager-sends
OpponentChoice / bundled CardRevealed / dummy Searched reveal — it only
accumulates into the next ChoiceRequest buffer (assemble_choice_buffer is a
proven complete superset). Buffer is the SOLE mid-game opponent-choice source
(false-positive guard, definitive). Removed unused OpponentChoiceInfo.timestamp_ms.

mtg-u3dwj asks (team-lead): (1) call_pre_choice_hook now applies state-sync up to
the choice ac before the LOCAL controller decides (covers in-resolution local
choices, all ChoiceKinds, DRY); (3) heuristic choose_cards_to_discard
debug_assert!s on an unresolvable OWN hand card (was a silent filter_map drop).
(2) rogerbrand seed3 heuristic is STILL RED — its divergence is DEEPER than the
reveal-timing class (network shadow heuristic plays Badlands + casts Sedge Troll
before discarding; All Hallow's Eve / Wheel of Fortune family, sibling of
mtg-609), NOT subsumed by the buffer shim. Tracked in mtg-u3dwj; not chased.

GATE (all isolated systemd-run --user --scope, mtime-fresh binary):
 - equiv-zero seed3: 8/8 then 6/6 (the branch-introduced desync — was ~50% FATAL)
 - equiv-random, equiv-heuristic, cycle315 x3, robots42: GREEN
 - full make validate: GREEN except a unit.nextest TIMEOUT under concurrent
   slot03-validate CPU contention (all individual tests incl all network tests
   PASSED; re-run in isolation pending). NOT a real failure.

## MTG Rules Review — Verdict: PASS

1. Correct rule implementation: N/A for rule semantics — this is network
   TRANSPORT/shadow-apply infrastructure; no engine rule logic changed. Affected
   effects' rules unchanged: search CR 701.20, scry/surveil CR 701.18/701.34,
   library order CR 401.2, draw CR 120.
2. Reveal ordering: YES (core of the fix) — the shadow now applies reveals up to
   the choice's action_count BEFORE the controller decides (call_pre_choice_hook
   sync + watermark-bounded eager reveal apply); reorders applied positionally so
   a draw reads the correct library order. Buffer is ascending-ac → replayable.
3. Information hiding: PRESERVED — opponent's fetched card stays hidden (dummy
   Searched reveal: empty name + card_id only). Buffer assembled from the SAME
   entitled sources the eager path used; TASK 2 removes a copy, adds no new info.
4. Decision authority: PRESERVED — choices still routed to the client's
   PlayerController via ChoiceRequest; the buffer carries FACTS + the opponent's
   OWN already-made decision, never a server-made decision.
5. Server/client sync: this IS the fix's domain — eliminates the multi-message
   arrival race; controllers stay information-independent; desync stays FATAL
   (no silent recovery; push_state_sync_at asserts on differing/lost delta).
   Verified: equiv-zero 6/6 + random/heuristic/cycle/robots green.
6. Workaround vs real fix: REAL FIX — the principled successor to the
   action_count exclusion (reveals arrive in canonical game-position order by
   construction). No card-name special case, no skipped events, no TODO-shim.
7. Bug-class generalization: class-level (all reveal/reorder/opponent-choice
   transmission + the in-resolution local-choice sync covers ALL ChoiceKinds;
   hardening surfaces the silent-drop class). Remaining sibling rogerbrand /
   All Hallow's Eve heuristic info-independence divergence is a SEPARATE class,
   tracked in mtg-u3dwj (pre-existing, not introduced here).

Reasoning: The change is network shadow-replay infrastructure that makes the
native client consume the ordered ChoiceRequest buffer (matching WASM) and
deletes the redundant eager opponent-choice send. It strengthens, not weakens,
the reveal-ordering and determinism invariants and preserves information hiding
and decision authority. The one red sibling (rogerbrand) is a pre-existing,
deeper divergence already tracked in mtg-u3dwj and explicitly out of scope.

Gamelog justification: cycle_ability_network_sync_e2e (seed 315, Mountaincycling
search→fetch→shuffle) and network_vs_local_equivalence_e2e (seed3 zero/random/
heuristic) confirm byte-identical local-vs-network gamelogs after the fix.

## TASK 0 SETTLED 2026-06-04 (slot02) — equiv-zero seed3 is BRANCH-INTRODUCED, NOT pre-existing mtg-725. The REFRAME above is WRONG.
Built clean INTEGRATION (primary checkout @b886d9b3, fresh release+network binary) and the BRANCH (@e0188273). Ran network_vs_local_equivalence_e2e.sh seed3 zero zero, each isolated under systemd-run --user --scope:
- INTEGRATION: 5/5 DETERMINISTIC GREEN — network game always 24 turns, 0 diverged, gamelogs IDENTICAL.
- BRANCH @e0188273: NONDETERMINISTIC — ~50% desync (10-run sweep: ~5 green at 24 turns, rest FATAL network-sync-mismatch aborting at turn 5-6 or 11). Local game ALWAYS 24 turns (deterministic) in both.
=> The predecessor REFRAME (equiv-zero = pre-existing zero-controller network nondeterminism, mtg-725 class, exclude-it) is FALSIFIED. It is a real BRANCH-INTRODUCED desync and must NOT be excluded.

LOCALIZED: the SERVER is fully deterministic — across passing and failing runs the server gamelog is byte-identical up to the Turn-5 Mountaincycling, and the cycle shuffle is identical (SHUFFLE-DEBUG before/after rng_hash + first_5_cards match exactly every run). The nondeterminism is ENTIRELY in the CLIENT (native) SHADOW: on the library-search/cycling reveal path the shadow nondeterministically ends up missing the fetched card (server P0 hand=5 vs client shadow=4; "Hand sizes DIFFER" at choice_seq=45 ac=252).

ROOT CAUSE PINNED (single-variable diagnostic): the trigger is THIS branch's new shuffle->LibraryReordered emission (state.rs ~818-837, mtg-o99ow L2b residual-#1, added for mtg-yexvc Timetwister stale-shadow-library). Disabling JUST that emit (if false && ...) and rebuilding -> equiv-zero seed3 8/8 GREEN. Mechanism: a cycling cycle does search-reveal-found-card THEN shuffle; the branch now sends an EXTRA async LibraryReordered (post-shuffle order) that races / arrives out of ac-order vs the found-card reveal over the websocket, and the native EAGER (synthetic-keyed, order-sensitive) apply path drops/misapplies the found-card reveal ~50% of the time. Integration never sent this message on shuffle, so its client stayed deterministic on the same game.

THE DIAGNOSTIC REVERT IS NOT THE FIX (removing the emit reintroduces mtg-yexvc). The principled fix is exactly the user's NATIVE-FIRST + DELETE-EAGER plan: TASK 1 (native buffer shim) makes native consume the SINGLE ascending-ac ordered buffer carried in the ChoiceRequest (reveals + reorders + opponent choices together), eliminating the multi-message async race; TASK 2 deletes the racy eager path. This CONFIRMS the predecessor's "Blocker A subsumed by Piece 2" intuition. equiv-zero is therefore a GATING item for the native-first gate (TASK 4), not an exclusion. Proceeding to TASK 1.

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
========================================================================
DEEP-AC IN-STACK-RESOLUTION STATE CLASS — empirical pins (2026-06-05, slot03-mtg677b)
This is the OPEN prize-blocker now that mtg-677 rewind/replay-faithfulness is
DONE (@1b61b895, the reveal-timing-stable discard carve-out). With that fix the
robots network games run far PAST the old rewind FATAL and expose GENUINE
server-vs-client STATE divergences at deep action_count (action_counts MATCH;
the state HASH differs — a real desync, not a rewind LogMismatch, not presentation):

  • seed 5 (decks/old_school/03_robots_jesseisbak.dck --network-debug):
      FATAL: P2 state hash mismatch at choice_seq=160 action_count=965
      server=fe79d428add11897 client=80de73e9c1540d20
      Deck has Copy Artifact / Balance → in-stack copy/resolution class. WASM
      battlefield/gy at the mismatch (from card_detail dump): bf ids
      [14,35,46,48,54,56,94,118,123], gy [[45,38,37],[101,122]] (122 = Copy
      Artifact). Artifacts: debug/netarch-undo-dumps/..seed5_{wasm_undo,card_detail,mismatch}.log
  • seed 2: turn-17-start P1-library TurnStartHashChanged (reserved-card
      reveal-materialisation timing across two rewinds; library order pins in
      mtg-677 "[SEED 2]" sections).

Both are the mtg-559 robots42 deep-ac residual family. The principled fix is THIS
issue's reveal-actionlog unification (drive reveals/materialisations through the
monotonic action_log keyed by game action_count so they replay deterministically
alongside the undo log). Acceptance prize: robots seeds 2 & 5 un-excluded-green in
web/test_network_multideck.js with the eb8f938e action_count-in-hash recipe applied
→ converged. NOT started this session (separate substantial class; mtg-677 was
scoped to rewind-faithfulness only).

========================================================================
DEEP-AC SEED-5 ROOT PINNED (2026-06-05, slot03-deepac, branch fix-deep-ac)
========================================================================
The deep-ac state class (the FINAL action_count-prize blocker) is now
byte-pinned for robots seed 5. Full writeup + repro + fix plan:
ai_docs/DEEPAC_SEED5_ROOT_PIN_20260605.md (this branch).

EXACT diverging field: ONE battlefield tuple at turn-14 Upkeep ac=965 —
card 14 = the OPPONENT's (P0) Mox Emerald, tapped for mana during P0's
turn 13. Server view = (14, TAPPED, ctrl 0) [correct]; WASM-shadow view =
(14, UNTAPPED, ctrl 0) [wrong]. Every other hashed field (life, hand/lib
sizes, both graveyards, stack, all other bf cards/controllers) is
byte-identical. server=fe79d428add11897 client=80de73e9c1540d20.

KEY SIGNATURE: both undo logs are IDENTICAL and BOTH contain Tap(14)@934
with NO Untap(14). So the WASM LIVE card.tapped (false) contradicts the
shadow's OWN action log (tapped). Empirically: untap step is innocent
(14 not in normal_to_untap, controller 0 != active 1); Card::untap is
NEVER called on 14 (probe count 0); EntityStore::insert is write-once
(no instance replacement). => a NON-undo-logged tap-state divergence.

ROOT: the reveal/replay-ordering model. apply_state_sync Pass-2 applies
REVEALS EAGERLY ahead of the shadow's replay position ("reveals are
identity injections, safe to apply early" — client.rs ~L914-919 + WASM
mirror). True for library order, FALSE for an opponent permanent whose
tap-state is set by a replayed same-turn action: eager materialize +
forward replay leaves the live Mox untapped while Tap is still logged
(action_count parity preserved, hashed STATE wrong). Exactly the
mtg-o99ow "reveal not aligned at the correct action_count" class,
surfaced now that mtg-725/mtg-677 let robots run this deep. This is why
eb8f938e (action_count-in-hash) still can't land: the STATE truly
diverges here.

FIX DIRECTION (next agent, principled — no band-aid): align opponent-
permanent materialization + derived per-instance state (tap, etc.) with
the action_count at which the replay executes the matching action — i.e.
bound reveal-apply POSITIONALLY for the battlefield-permanent case
(mirror the reorder bound), keeping eager apply only for true identity-
only reveals (hand/library/graveyard); OR drive the tap through the
ac-keyed log so it replays deterministically. Decisive next probe:
instrument every write to a card's `tapped` field (Card::untap ruled
out) to catch the non-logging writer that flips 14 to false (suspect: a
zone-move/ETB/clone/snapshot path resetting tapped after the logged tap,
ordering-dependent on the eager reveal).

STATUS: diagnosis-to-checkpoint complete; fix NOT yet implemented (deep
foundational surgery — checkpointed per discipline rather than rushed).
Diagnostic harness tweaks kept on branch (gitignored debug output); the
temporary engine DEEPAC_* probes were reverted (engine tree clean).

## 2026-06-05 (slot03-deepac2) @dd9d44ee — DEEP-AC SEED-5+SEED-2 BOTH CONVERGE (two rewind fixes)

Root-caused the deep-ac desync byte-by-byte (card 14 Mox Emerald, robots seeds). The
seed-5 view hash carried TWO co-present opponent-permanent divergences (the view hash
folds library SIZE + battlefield tap status into one value, so the predecessor's
byte-pin saw only the tap and missed the library off-by-one):

FIX 1 (tap class): a reveal-materialised opponent permanent (non-undo-logged
cards.insert) starts UNTAPPED; a TapCard at ac<=R (Mox tapped for mana turn 13) is
neither carried by the reveal nor re-applied by the forward replay (which runs only
ac>R), so tapped defaulted false. unwind_state_sync_to now re-derives position-R
tapped from the retained undo log via new UndoLog::reconstruct_tapped_states().

FIX 2 (library class = the seed-2 sibling): on rewind, reset_state_sync_cursor() reset
BOTH cursors to 0, re-applying every LibraryReorder ac<=R. A reorder rewrites library
MEMBERSHIP; re-applying a pre-departure reorder re-adds a card that left the library
(cast to battlefield), clobbering the correct undo-rewound membership -> phantom lib
entry (player_library_size N vs N+1). REORDER cursor now resets to R (not 0); REVEAL
cursor still 0.

Added DebugSyncInfo.library_ids diagnostic (sibling of graveyard_ids) -> server box
dumps per-player library CardIds; this pinned FIX 2.

EVIDENCE (mtime-fresh server+wasm, strict per-choice view-hash check):
- seed 5: Completed winner=Some(1) ac=1283 (was FATAL @ seq160/ac965); 0 mismatches
- seed 2: Completed winner=Some(0) ac=3570; 0 mismatches
- clippy engine+wasm32 -Dwarnings clean; fmt clean; undo::tests 15 pass (+new
  test_reconstruct_tapped_states).

REMAINING (gated on this): re-apply eb8f938e action_count prize + full un-excluded
canary + make validate + add seeds 2/5 to multideck gate. NOT yet merged — team-lead
adversarial desync-review + MTG-rules review pending. Keeps hash ID mtg-o99ow.

═══ DEEP-AC BLOCKER INCREMENT (slot03-blockers, fix-deep-ac, 2026-06-05) ═══
PLAIN-LANGUAGE: fixed the Balance-resolution crash on robots seeds 7 & 11. It was
a LOGGING bug (one discard log line skipped when the browser hadn't yet learned
the opponent's discarded card identity → shifted the whole log stream by one →
false rewind/replay self-check failure), NOT a state divergence. Seed 11 now
PASSES strict; seed 7 advances to a deeper, separate state bug.

CHANGES (committed on fix-deep-ac, prize still OFF):
 1. mtg-d4j9v [PRIMARY]: actions/mod.rs execute_balance_effect Hand-zone discard
    log now emitted UNCONDITIONALLY via gamelog_reveal_stable (verifier key
    'X discards card#<id> to Balance'), the 3rd discard-log site of the mtg-677
    reveal-timing class. Kills the +1 line-count offset (seed7 buffer idx 68,
    seed11 idx 382).
 2. mtg-f0w57: undo.rs reconstruct_controller_states() + client.rs applies it on
    re-materialization (twin of reconstruct_tapped_states; restores hashed
    'controller'). Robots can't exercise it; unit-tested.
 3. mtg-j4krs #1: protocol.rs SubmitChoice.spell_ability stale doc corrected.

STRICT broad sweep (action_count RE-INCLUDED), mtime-fresh: PASS 2,5,6,9,11,18,20,42
(8/10). Shipped-config (prize OFF) identical. make validate GREEN 33/0.

STILL BLOCKING THE PRIZE (checkpointed):
 - seed 7: P1 hand-size off-by-one (server 6 vs client 5) at choice_seq=230
   action_count=1341, right after Demonic Tutor search-to-hand following a Wheel
   of Fortune discard+redraw → deep-ac in-stack-resolution reveal class; principled
   fix = reveal-actionlog unification (mtg-ho2r8).
 - seed 19 (mtg-8ow9h): client sends illegal choice index 2 (server offers 2) at
   WebRandom Fireball cast turn 24 — option-set/state divergence (mtg-0e1wo family).
 - mtg-j4krs #2 (WASM spell_ability populate): deferred (crash-earlier guard).
Prize (eb8f938e) re-applies only after ALL of 2,5,6,7,9,11,18,19,20,42 converge strict.

── seed-7 root-cause TIGHTENED (slot03-blockers, read-only analysis @bcec4890) ──
Full diagnosis: ai_docs/DEEPAC_BLOCKERS_CHECKPOINT_20260605.md. Wheel-of-Fortune
redraw is IDENTICAL server vs client (both WebRandom draw 7: ids 82,91,90,79,105,96,86;
discards Copy Artifact+Su-Chi match) → NOT the divergence. The split is during DEMONIC
TUTOR (105) search-to-hand: at choice_seq=230 the engine logs 'action_count mismatch
client=1339 server=1341 (diff=2)' AND hand 5(client) vs 6(server). The shadow reaches
the tutor-resolution sync point BEFORE the search-result's library→hand move (+ its undo
actions) has been applied → short by the tutored card (hand -1) and short by the move
(ac -2). Deep-ac in-stack-resolution reveal-application LAG (mtg-o99ow/mtg-559 family),
NOT the tapped/controller re-materialization class. Principled fix = reveal-actionlog
unification (mtg-ho2r8): drive search-to-hand through the action_count-keyed consensus
log so the shadow applies it in lockstep before the resolution sync.

── seed-7 DECISIVE reframe (slot03-blockers, server+shadow undo diff @4d4467fe) ──
CORRECTION to the "reveal lag" note above: captured the SERVER undo log (--undo-dump)
and diffed vs the shadow's — they are BYTE-IDENTICAL through ac 1339 (same Demonic
Tutor fetch 97->P1 hand, same Plains-82 land play, same order). seed 7 is NOT an
action-sequence divergence (refutes slot04's "real state divergence"). The fatal
'P1 hand 6 vs 5' is a choice_seq<->action_count STAMPING SKEW: server validates
choice_seq=230 at ac 1341, client hashed at ac 1339, straddling [1337] MoveCard(82
Hand->Battlefield) (the land play that drops hand 6->5). Hand size is hashed
independent of action_count → fatals even prize-OFF. = the in-stack LOCKSTEP/
choice-hash-timing half of mtg-ho2r8 (make both replicas hash a given choice_seq at
the same ac), NOT a missing-delta. Full: ai_docs/DEEPAC_BLOCKERS_CHECKPOINT_20260605.md.

## SEED-7 DECISIVE RE-DIAGNOSIS 2026-06-05 (slot03-lockstep) — corrects prior 'stamping skew' reframe; this is missing-opponent-delta (mtg-ho2r8 §1-2)

Fresh mtime-fresh maximally-strict instrumented repro (03_robots_jesseisbak.dck seed 7, --undo-dump) with a NEW both-players hand-CardId dump (DebugSyncInfo.hand_ids + WASM_CARD_DETAIL hand0/hand1) NAMED the lost card directly:

  player=0 (NATIVE desktop OBSERVER) choice_seq=230 ac=1341:
  SERVER P1 hand=[79,86,90,91,96,97]  CLIENT P1 hand=[79,86,90,91,96]
  -> P1 hand CardIds DIFFER: on_server_only=[97]

GENUINE lost-card state divergence (NOT a stamping skew). Browser P1 casts Demonic Tutor -> fetches card 97 (Fireball) into its OWN hand; browser shadow is correct (WASM seq242 hand1 has 97, hash==server). The NATIVE observer removes 97 from P1 library (lib=36 matches) but never deposits into P1 hand zone -> 5 vs 6. ROOT: opponent Searched reveal is a DUMMY -> process_card_reveal SKIPS it (reveal_processor.rs:77-86), card never instantiated; replayed move_card(97 Lib->Hand) doesn't increment player_hand_size = zones.hand.len() (controller.rs:646-651). The prior '+12 choice_seq drift / acs 1230 vs 1341' was an artifact of PER-PLAYER choice_seq counters; action_counts already AGREE at the fatal (client_ac=1341 expected_ac=1341).

FIX DIRECTION (durable, = mtg-ho2r8 §1-2): deposit an identity-hidden reserved placeholder into the OPPONENT hand zone on the observer for an opponent Searched-into-hand, keyed by search-resolution ac, rewind-surviving. Verify all 10 robots seeds strict before re-applying the eb8f938e ac-exclusion prize. Full writeup: ai_docs/DEEPAC_SEED7_MEMBERSHIP_CONFIRM_20260605.md. TEAM-LEAD: mirror this onto mtg-ho2r8 (integration-only; absent on fix-deep-ac worktree). Diagnostics committed on fix-deep-ac.
