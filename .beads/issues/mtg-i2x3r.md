---
title: 'netarch Phase 2 steps 3-4: native parity + engine choose_at unification'
status: open
priority: 2
issue_type: task
created_at: 2026-05-30T04:45:39.533970539+00:00
updated_at: 2026-05-30T17:36:47.736339157+00:00
---

# Description

netarch Phase 2 steps 3-4 — native parity (ActionLog) + engine choose_at unification.

== STATUS (2026-05-30, branch netarch-phase2-native-parity) ==
STEP 3: DONE @ commit f30c2d9d (native SharedNetworkState converted to the shared
ActionLog primitives; cargo build --release --features network + clippy
(-D warnings) + fmt all green; 76 network unit tests pass; full make validate
green — see validate_logs artifact cited in the agent report).
STEP 4: STILL PENDING (deferred — design below).

== STEP 3 — WHAT LANDED ==
Native SharedNetworkState (mtg-engine/src/network/client.rs) converted off the
legacy pending_*/drain_*/MVar model onto the SHARED ActionLog primitives the
WASM side already uses, reusing the SAME payload types StateSyncEntry /
ChoiceEntry (no parallel native-only copies). Design invariant #10 (one
primitive native + WASM): MET.

All 8 native DELETE-table rows (docs/NETWORK_ACTION_LOG.md § 5) GONE:
 3a state-sync log:
  - drain_reveals_up_to / drain_all_reveals / drain_all_reveals_if_ready /
    drain_all_library_reorders DELETED
  - wait_for_library_reorders DELETED (was dead code — never called; its
    forbidden TIMEOUT is gone). Replaced by wait_for_state_sync_frontier, a
    NO-timeout Condvar frontier-wait (native trampoline-equivalent, § 4),
    released only on data arrival or terminal disconnect.
  - fields pending_reveals / pending_library_reorders VecDeques DELETED
  - field library_reorder_condvar DELETED
  - field choice_pending AtomicBool DELETED
 3b: take_remote_choice + remote_choice_mvar DELETED -> opponent_choices:
    ActionLog<ChoiceEntry> keyed by choice_seq (mtg-sfihb) + FIFO read cursor
    (native mirror of WASM next_opponent_choice_cursor). take_opponent_choice
    reads non-destructively.
 3c: take_choice_accepted_for_seq + choice_accepted_mvar DELETED ->
    choice_accepted: ActionLog<ChoiceEntry> keyed by choice_seq;
    wait_for_choice_accepted reads non-destructively by seq.

Also removed now-dead enums RemoteChoiceInfo / ChoiceAcceptedInfo / legacy
ChoiceInfo and struct PendingReveal. run_game sync_callback now calls
apply_state_sync_up_to_frontier (reorder-before-reveal, mtg-589, non-destructive
-> rewind/replay via reset_state_sync_cursor). Initial reorders captured in
wait_for_game_start are folded into the same state-sync log before the loop via
push_library_reorder. Desync stays FATAL (ActionLog::push panics on
non-monotonic choice_seq). No new timeout-blocks/sleeps on the client path.
Files: network/{client.rs,remote_controller.rs,local_controller.rs} (+ a one-line
allow(dead_code) on the now-unused mvar::is_exit_signaled accessor).
MTG rules review: PASS (transport-only; no game-visible semantics change;
determinism / information-hiding / network-parity preserved).

== STEP 4 DESIGN (engine choose_at unification — do AFTER 3, still pending) ==
Introduce Controller::choose_at(view, action_count) -> ControllerResult<ChoiceEntry>
+ NetworkClient::apply_state_sync_at(state, K) as the uniform engine API.
- network_choice.rs has the get_controller_type()==ControllerType::Remote
  special-casing at 13 sites (verified by grep). Once BOTH the native
  RemoteController (now on the opponent_choices ActionLog from 3b) and the
  AI/local controllers answer choose_at(view, ac) uniformly — remote returns
  buffer.get(ac), AI re-derives from the view — retire all 13 is_remote branches.
- WASM side: retire pop_opponent_choice + next_opponent_choice_cursor FIFO shim
  + OpponentChoiceData re-materialisation. Delivers invariant #9 (engine
  controller-agnostic). choose_at must thread through BOTH the WASM unwind path
  and the native blocking path (native block can stay inside the controller via
  the no-timeout frontier-wait added in 3a/3b). This is the genuinely hard part
  and why Step 4 must follow a green Step 3.
- Coordinate with the wasm-rewind-replay worktree (mtg-614), which also reworks
  the WASM blocking/replay path — Step 4's choose_at NeedsInput threading overlaps it.

== Perf nit (low, still open) ==
apply_state_sync_up_to_frontier (both WASM and the new native impl) clones each
windowed entry out of the log to release the lock before mutating GameState. The
native window is tiny (one reveal/reorder per sync point); an apply-by-index
split-borrow could remove the clone later.

== Wire-protocol step 5 (optional, still open) ==
Promote CardRevealed/LibraryReordered to carry a server-authoritative
action_count; native 3a could then key state_sync by the real wire ac (no
synthetic next_ac counter) and assert received_ac == expected as an in-data
desync tripwire. Left for follow-up.

== Next actor ==
Implement Step 4 (choose_at unification) on a fresh worktree off the merged
Step-3 integration, coordinating with mtg-614.
