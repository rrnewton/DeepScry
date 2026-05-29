# Network Action Log — Phase 2 Migration Plan

**Status:** Plan only; no consumer code changes land in Phase 1.
**Phase 1 in this branch** ships only the `ActionLog<T>` primitive
(`mtg-engine/src/network/action_log.rs`) and the two design docs:
this one and [`NETWORK_ACTION_LOG.md`](NETWORK_ACTION_LOG.md).

This document is the **ordered** Phase 2 rewrite plan. Each step has:

- **Deletes** — legacy paths removed.
- **Adds** — new code (small, mostly owner-adapter fields).
- **Green check** — what must still pass before moving on.
- **Rollback** — how to back out if a step regresses.

The driving target — the one bug the whole refactor must fix on the way
in — is **`robots42` (mtg-559)**: a WASM shadow-mode reveal/reorder
arrival race that destructive `drain_*` calls cannot detect.

## 0. Pre-flight (before any consumer changes)

1. Confirm the primitive (`ActionLog<T>`) and its unit tests pass under
   `make validate` on `integration` after Phase 1 lands. This is the
   green baseline.
2. Add a `#[cfg(debug_assertions)]` post-apply state-hash assert hook
   on `GameState` (or wherever `compute_state_hash` already lives), so
   subsequent steps can assert "after applying state-sync at K, the
   shadow hash equals the server-reported hash for K." This is the
   detect-then-crash desync gate that makes step 1 honest. **No
   recovery hook** — per `NETWORK_ARCHITECTURE.md` § *Desync is ALWAYS
   a Fatal Error*.

## 1. First migration target: `robots42` / mtg-559 reveal+reorder

This is the **first concrete Phase 2 step.** It is intentionally narrow.

The bug: in WASM shadow mode, `Sensei's Divining Top` + library
reorderings can cause `CardRevealed` and `LibraryReordered` to arrive
in a non-deterministic order relative to the engine's drain points.
`drain_all_reveals_if_ready` then sees the wrong subset. With
`ActionLog<StateSyncEntry>` keyed by `action_count`, arrival order is
irrelevant — the engine reads the entries at the `action_count` it
asks for, and yields `NeedsInput` if they aren't there yet.

### 1.1 Deletes

- `WasmNetworkClient::pending_reveals` field
  (`mtg-engine/src/wasm/network/client.rs:99`).
- `WasmNetworkClient::pending_library_reorders` field
  (`mtg-engine/src/wasm/network/client.rs:108`).
- `WasmNetworkClient::drain_reveals`
  (`mtg-engine/src/wasm/network/client.rs:852`).
- `WasmNetworkClient::drain_library_reorders`
  (`mtg-engine/src/wasm/network/client.rs:859`).
- The "drain reveals before processing choice" call site in
  `WasmRemoteController::choose_*` paths
  (`mtg-engine/src/wasm/network/remote_controller.rs`).

### 1.2 Adds

- A `pub state_sync: ActionLog<StateSyncEntry>` field on
  `WasmNetworkClient`.
- A `StateSyncEntry` enum in `mtg-engine/src/network/state_sync.rs`
  with variants `RevealCard { owner, card, reason }` and
  `LibraryReorder { player, new_order }`.
- WS receive handler for `ServerMessage::CardRevealed` / `LibraryReordered`:
  call `state_sync.push(server_action_count, entry)` instead of
  pushing to a `VecDeque`. (Wire-protocol option (b) from
  `NETWORK_ACTION_LOG.md` § 6 — derive `action_count` on receipt.)
- A thin `WasmNetworkClient::apply_state_sync_at(&mut shadow, ac)`
  method that returns `Ready` or `NeedsInput`.
- Engine pre-choice hook: before `controller.choose_*` at
  `action_count = K`, call
  `network_client.apply_state_sync_at(&mut shadow_state, K)`. If
  `NeedsInput`, the engine unwinds to the JS event loop.

### 1.3 Green check

- All existing native and WASM tests pass under `make validate`,
  including `web/test_*.js` browser E2E.
- A new dedicated regression test for `robots42`: an `agentplay`
  script reproducer that exercises Sensei's Divining Top + a library
  search, run twice with reveals delivered in opposite order on the
  wire. Both runs must produce the same gamelog.
- `#[cfg(debug_assertions)]` post-apply state-hash assert (from step 0)
  must not fire on any test.

### 1.4 Rollback

Revert the single commit; `state_sync` field becomes dead code (the
`VecDeque`s are restored from git). No protocol change yet, so no
client/server skew risk. The branch isolates this step from the
controller-buffer step (1.5) below.

## 2. Per-controller choice buffer for the remote path

Generalise `OpponentChoice` consumption to use `ActionLog<ChoiceEntry>`
on `NetworkRemoteController`.

### 2.1 Deletes

- `WasmNetworkClient::opponent_choices` field
  (`mtg-engine/src/wasm/network/client.rs:111`).
- `WasmNetworkClient::pop_opponent_choice`
  (`mtg-engine/src/wasm/network/client.rs:827`).
- Bounds-check / partial-drain defensive code in
  `WasmRemoteController` (its raison d'être evaporates once reads are
  non-destructive and frontier-driven).
- `SharedNetworkState::take_remote_choice` (native equivalent).

### 2.2 Adds

- `ChoiceEntry` enum (or struct) holding `{ choice_indices,
  description, spell_ability, library_search_result, target_card_ids,
  choice_seq }` — same fields as today's `OpponentChoice` message.
- `NetworkRemoteController::buffer: ActionLog<ChoiceEntry>`.
- WS receive handler for `ServerMessage::OpponentChoice` /
  `ChoiceAccepted`: push to the right controller's buffer at the
  server-reported `action_count`. (Native: same.)
- `Controller::choose_at(view, ac) -> ControllerResult<ChoiceEntry>`
  trait method, with a default impl for stateless AI controllers.
- `NetworkRemoteController::choose_at`: `self.buffer.get(ac).cloned()
  .map(Ready).unwrap_or(NeedsInput)`.

### 2.3 Green check

Same as 1.3, plus: the existing network E2E suite (ai_harness +
fancy_tui) must still produce identical gamelogs commit-over-commit.

### 2.4 Rollback

Revert; the `VecDeque<OpponentChoice>` is restored. Step 1 stays
landed because the two are independent.

## 3. Per-controller buffer for local human clicks

Mirror of step 2 for `LocalHumanController`.

### 3.1 Deletes

- `SharedNetworkState::take_choice_accepted_for_seq` (the MVar-style
  destructive seq-matcher; obsoleted by `action_count` keying).
- `SharedNetworkState::choice_pending` (already dead after step 1, but
  the field declaration goes too).
- Per-seq matching helpers in `local_controller.rs`.

### 3.2 Adds

- `LocalHumanController::buffer: ActionLog<ChoiceEntry>`.
- UI-event appender: the wasm_bindgen export that today calls
  `submit_choice(...)` now calls `controller.buffer.push(ac, entry)`
  where `ac` is the engine's current `requesting_action_count`.
- Same `choose_at` shape as step 2.

### 3.3 Green check

Browser E2E: clicking through a game produces an identical sequence of
`[GAMELOG]` lines as before the refactor.

### 3.4 Rollback

Revert; previous MVar/seq path restored.

## 4. Native parity sweep

Apply the same shape to native client (`SharedNetworkState`) so the
`#[cfg(feature = "network")]` paths and WASM paths share an identical
controller / state-sync structure.

### 4.1 Deletes

- `SharedNetworkState::pending_reveals`, `pending_library_reorders`,
  `library_reorder_condvar`, plus all the `drain_*` and
  `wait_for_library_reorders` methods (`client.rs:342, 347, 351,
  495–611`).

### 4.2 Adds

- Native `NetworkClient` gets the same `state_sync: ActionLog<...>`
  field; native `NetworkRemoteController` gets the same `buffer:
  ActionLog<...>` field. Sync primitive is `Arc<Mutex<_>>` instead of
  `Rc<RefCell<_>>`; otherwise identical.

### 4.3 Green check

`make validate` end-to-end, including all native network integration
tests.

### 4.4 Rollback

Revert; native FIFOs restored. WASM (steps 1–3) stays landed.

## 5. Wire-protocol upgrade (optional, follow-up)

Promote `action_count` from "derived on receipt" to "server-authoritative":

- Add `action_count: u64` to `ServerMessage::CardRevealed` and
  `ServerMessage::LibraryReordered` (`mtg-engine/src/network/protocol.rs:406, 425`).
- Server populates from its own `undo_log.len()`.
- Client validates on receipt: if `received_ac != expected_ac`,
  immediate fatal desync (no recovery).
- Add the optional `state_hash: u64` field on each `StateSyncEntry`
  variant for in-data desync detection.

### 5.1 Green check

A staged client / server skew test: an old client against a new server
(and vice-versa) is handled cleanly by the existing `--version`
handshake — out-of-spec versions refuse the lobby.

### 5.2 Rollback

Revert; clients fall back to derive-on-receipt. Step 4 stays landed.

## 6. Final cleanup

Once all of the above lands and bakes:

- Remove the now-unused `SharedNetworkState` queueing helpers if any
  legacy callers remain.
- Delete `mtg-engine/src/network/reveal_processor.rs` if the
  state-sync path subsumes it (audit needed; it may also be reused by
  the server side).
- Audit and remove any `wait_for_*` / `choice_pending` / "drain if
  ready" comments that survived the rewrite.

## Test surface (applies to every step)

Every step must produce a `validate_logs/validate_<sha>.log` with
"=== All validation steps completed ===" before the next step starts.
The full suite includes:

- `cargo test -p mtg-engine --features network` — primitive unit tests
  + network integration tests.
- `cargo test --workspace --all-features` — full workspace.
- `cargo clippy --all-targets --all-features --features network -- -D warnings`.
- `cargo fmt --all -- --check` (nightly).
- `web/test_*.js` browser E2E (Playwright).
- `agentplay/*.sh` scripted reproducers — especially the new
  `robots42` reproducer added in step 1.

## Phase 2 net-LOC estimate

The Phase 2 changes are substantially net-negative on LOC. Rough
accounting:

- **Adds**: `StateSyncEntry` enum + `ChoiceEntry` struct (~80 lines
  combined); per-owner field declarations + `choose_at` /
  `apply_state_sync_at` adapters (~200 lines across 4–5 files);
  the `robots42` regression test (~120 lines).
- **Deletes**: the 14 legacy paths enumerated in
  `NETWORK_ACTION_LOG.md` § 5, including three sizable `drain_*`
  helpers, `wait_for_library_reorders`, the `choice_pending` race
  flag, the `library_reorder_condvar` plumbing, three `VecDeque`
  fields on each of `SharedNetworkState` and `WasmNetworkClient`, and
  the bounds-check / partial-drain defensive code in
  `WasmRemoteController` (~700–900 lines combined, plus the
  call-site cleanups that radiate from `client.rs:run_game`).

Best-guess **net Phase 2 delta: −400 to −700 LOC**, plus the
disappearance of an entire class of timing-dependent comments and
flags. The primary win, however, is not LOC but **invariant
enforcement**: a single Vec<T> per owner replaces three queues + four
race flags + two helpers with timeout-blocking semantics.
