---
title: 'netarch Phase 2 steps 3-4: native parity + engine choose_at unification'
status: open
priority: 2
issue_type: task
created_at: 2026-05-30T04:45:39.533970539+00:00
updated_at: 2026-05-30T15:06:05.244437210+00:00
---

# Description

netarch Phase 2 steps 3-4 — native parity (ActionLog) + engine choose_at unification.

== STATUS (2026-05-30, branch netarch-phase2-native-parity @ base e2bf1e94) ==
Step 3 DEFERRED with full design note (below). Step 4 DEFERRED (depends on Step 3).
No production code changed this pass — see "Why no mutation this pass". Baseline
build verified green (cargo build --features network -p mtg-engine, EXIT=0).

Confirmed starting facts (corrected from an earlier bad-grep mis-read):
- WASM side IS converted (commits c2d59de0 step1, 49b12ca4 step2). wasm/network/
  client.rs uses ActionLog<StateSyncEntry> (state_sync) + ActionLog<ChoiceEntry>
  (opponent_choices); legacy reveal/reorder VecDeques + drain_* are gone there.
  Invariant #10 (same primitive native+WASM) currently UNMET only because native
  still uses the legacy model.
- Native side is fully legacy, exactly the 8 DELETE-table rows:
  SharedNetworkState (network/client.rs) drain_reveals_up_to:495,
  drain_all_reveals:517, drain_all_reveals_if_ready:534, wait_for_library_reorders:569
  (timeout-block), drain_all_library_reorders:611; fields pending_reveals:342,
  pending_library_reorders:347, library_reorder_condvar:351, choice_pending:376;
  take_remote_choice:643, take_choice_accepted_for_seq:668.
- Consumers: run_game sync_callback (client.rs:1817-1871) calls
  drain_all_library_reorders + drain_all_reveals greedily; RemoteController
  (remote_controller.rs:100,211) blocks on take_remote_choice; NetworkLocalController
  (local_controller.rs:839) blocks on take_choice_accepted_for_seq.

== KEY ARCHITECTURAL FINDING (why native ≠ "mirror WASM with Arc<Mutex>") ==
The native client is a BLOCKING-THREAD coroutine, not the WASM single-threaded
stack-unwind coroutine:
  - Engine runs in tokio::task::spawn_blocking (client.rs:1788); it runs
    GameLoop::run_game to completion synchronously on that thread.
  - WS messages arrive on a SEPARATE tokio reader task (run_ws_reader_shared).
  - Synchronisation is MVar (mvar.rs): Condvar-backed BLOCKING take(). When the
    engine needs input not yet arrived, the engine THREAD BLOCKS on the Condvar
    (mvar.rs:69-81) until the reader pushes. There is NO event loop to unwind to.
The design's "K > frontier ⇒ return NeedsInput, unwind to the JS event loop"
(NETWORK_ACTION_LOG.md §2.2) is the WASM trampoline. The native trampoline-
equivalent is "block the engine thread on a Condvar until frontier ≥ K." The
design doc explicitly anticipates this (§3.3/§4: native uses "a Condvar notified
on every push" as the sync primitive wrapping the OWNER). So a native ActionLog
port keeps a Condvar — it just must be the NO-TIMEOUT frontier wait. The forbidden
thing about wait_for_library_reorders is its TIMEOUT (client.rs:575-583, returns
false on deadline), not that it blocks. This is why native was correctly left
untouched in steps 1-2: it is NOT a mechanical Rc<RefCell>→Arc<Mutex> swap.

== STEP 3 DESIGN (native parity, ordered, each its own validate-green commit) ==
3a. state_sync log. Add `state_sync: ActionLog<StateSyncEntry>` +
    `last_applied_state_sync_ac: u64` to SharedNetworkState (behind the existing
    Mutex; the lock wraps the owner, not the log — design §3.3). Reuse the SHARED
    StateSyncEntry (network/state_sync.rs) — do NOT define a native copy.
    - WS reader (run_ws_reader_shared, ~client.rs:1996+): on CardRevealed push
      RevealCard, on LibraryReordered push LibraryReorder, keyed by the server
      action_count it already tracks (server_action_count), under the Mutex, then
      notify a NEW Condvar `state_sync_notify`. Replaces push_reveal/
      push_library_reorder into the VecDeques.
    - sync_callback (client.rs:1817): replace the two drain_all_* calls with an
      apply_state_sync_up_to_frontier(&mut game, our_player_id, &card_db) that
      walks entries last_applied_state_sync_ac < ac ≤ frontier(), applies reorder-
      before-reveal ordering by action_count (kills the implicit cross-channel
      ordering), and bumps the cursor. Non-destructive: enables rewind/replay.
    - DELETE drain_reveals_up_to / drain_all_reveals / drain_all_reveals_if_ready /
      drain_all_library_reorders / wait_for_library_reorders, the pending_reveals /
      pending_library_reorders VecDeques, library_reorder_condvar, choice_pending.
    - The pre-game-loop initial-reorder application (client.rs:1882-1907, the
      `pending_library_reorders` captured during wait_for_game_start) folds into
      the same state_sync log: push those reorders at their action_count before the
      loop starts; the first sync_callback applies them. Removes the
      NetworkClient::pending_library_reorders Vec field too.
    - The forbidden timeout wait disappears: the initial DrawCard's sync point
      blocks (if needed) on state_sync_notify with NO timeout — the frontier-driven
      wait. (Engine thread blocking here is the native trampoline; correct per §4.)
3b. remote choice buffer. RemoteController gets
    `buffer: ActionLog<ChoiceEntry>` (reuse SHARED ChoiceEntry, keyed by choice_seq
    per the mtg-sfihb dup-action_count rule already encoded in choice_entry.rs).
    WS reader pushes OpponentChoice → buffer (under Mutex) + notify. RemoteController
    reads by the engine's current action_count/choice_seq instead of
    take_remote_choice; blocks on the Condvar when behind frontier. DELETE
    take_remote_choice + RemoteChoiceInfo MVar. Non-destructive read replaces the
    destructive MVar pop, giving native the same replay property as WASM.
3c. local ChoiceAccepted. Replace take_choice_accepted_for_seq (local_controller.rs
    :839) — already a seq-matched destructive MVar — with an ActionLog<…> buffer
    keyed by choice_seq, read non-destructively. DELETE take_choice_accepted_for_seq.
After 3a-3c: all 8 native DELETE-table rows gone; invariant #10 holds; native +
WASM share ActionLog + StateSyncEntry + ChoiceEntry (one primitive, one payload set).

== STEP 4 DESIGN (engine choose_at unification — do AFTER 3) ==
Introduce Controller::choose_at(view, action_count) -> ControllerResult<ChoiceEntry>
+ NetworkClient::apply_state_sync_at(state, K) as the uniform engine API.
- network_choice.rs has the get_controller_type()==ControllerType::Remote special-
  casing at 13 sites (68,132,171,209,253,299,346,421,494,536,569 — more than the 4
  the brief lists; verified by grep). Each currently branches "is this the remote
  opponent? read from the network path : run the local controller." Once BOTH
  RemoteController (3b) and the AI/local controllers answer choose_at(view, ac)
  uniformly — remote returns buffer.get(ac), AI re-derives from the view — the
  engine stops switching on controller type. Retire all 13 is_remote branches.
- WASM side: retire pop_opponent_choice + next_opponent_choice_cursor FIFO shim +
  OpponentChoiceData re-materialisation (wasm/network/client.rs ~939-991, 75-95) in
  favour of the controller answering choose_at directly. Delivers invariant #9
  (engine controller-agnostic). NOTE: choose_at must thread through BOTH the WASM
  unwind path and the native blocking path — design the trait return so a controller
  can signal "not yet at frontier" (NeedsInput) on WASM and block-then-return on
  native, OR keep the native block inside the controller and only surface NeedsInput
  on WASM. This is the genuinely hard part and why Step 4 must follow a green Step 3.

== Why no mutation this pass ==
Step 3 is a large, delicate rewrite of fatal-desync code (SharedNetworkState, both
controllers, the run_game sync wiring, the WS reader) that MUST pass the full
multi-process network E2E suite (network_vs_local_equivalence_e2e.sh,
robots42_state_sync_e2e.sh, fuzz_determinism_netequiv_e2e.sh) under make validate
to be safe. A partial or unvalidated conversion of this code is strictly WORSE than
the current clean-but-legacy state (it would half-break a working blocking model and
leave invariant #10 violated in a new way). Per the task brief's explicit guard
("a clean Step 3 + solid Step 4 plan is a valid deliverable; do NOT ship a half-
wired Step 4") and CLAUDE.md's "desync is ALWAYS fatal / no recovery hacks", the
correct deliverable for this pass is the precise design above, not a rushed edit.

== Perf nit (item 3, low) ==
apply_state_sync_up_to_frontier in WASM (wasm/network/client.rs ~1085) clones each
windowed entry out of the log (entry.clone()) to dodge the borrow checker. Apply-by-
index (read len/frontier, then index entries[i] inside the apply loop) or a split
borrow removes the clone. Mirror the fix into the native 3a apply path so the new
code is born clone-free.

== Wire-protocol step 5 (item 4, optional) ==
WASM state_sync uses a synthetic next_state_sync_ac counter for reveal/reorder acs
(client-derived). Promote CardRevealed/LibraryReordered to carry server-authoritative
action_count (protocol.rs:406,425); native 3a can then key state_sync by the real
wire ac directly (no synthetic counter) and assert received_ac == expected for an
in-data desync tripwire. Leave for follow-up.

== Next actor ==
Implement Step 3 (3a→3b→3c, validate-green between each) on this branch, treating
the native MVar Condvar as the legitimate native frontier-wait (no timeout). Then
Step 4. Coordinate with wasm-rewind-replay worktree (mtg-614) which also reworks the
WASM blocking/replay path — Step 4's choose_at NeedsInput threading overlaps it.
