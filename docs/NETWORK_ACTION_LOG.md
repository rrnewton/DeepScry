# Network Action Log (Generic Append-Only `action_count`-Indexed Log)

**Status:** Phase 1 design + primitive (this branch: `netarch-action-log-phase1`).
**Phase 2 (rewrite consumers) is gated on user review of this document.**

This document specifies the **append-only, `action_count`-indexed,
non-destructive, replayable** log primitive — `ActionLog<T>` — and the
**two-store ownership split** that puts the engine, every controller,
and the network client into proper isolation. It is a refinement of
the invariants in [`NETWORK_ARCHITECTURE.md`](NETWORK_ARCHITECTURE.md) —
read that first; this doc only covers what changes on top.

## 1. Motivating problems with the current model

Today, three independent FIFO queues sit on the client side and are
*destructively consumed* by the engine as it walks forward:

- `SharedNetworkState::pending_reveals` — drained by
  `drain_all_reveals_if_ready` from a sync callback
  (`mtg-engine/src/network/client.rs:534`).
- `SharedNetworkState::pending_library_reorders` — drained by
  `drain_all_library_reorders` after a `wait_for_library_reorders`
  *timeout-blocking* helper (`client.rs:569`).
- `WasmNetworkClient::opponent_choices` — drained by `pop_opponent_choice`
  (`mtg-engine/src/wasm/network/client.rs:827`) and the analogous
  `take_remote_choice` MVar on native.

This shape has four chronic problems:

1. **Destructive read = no rewind.** Once the engine consumes an entry,
   it is gone. Re-entering the engine to re-derive a state (snapshot
   resume, MCTS, post-mortem replay, late-binding fixups, fuzz
   minimisation) is impossible without re-fetching from the server.
2. **Sync hacks.** Because reveals and reorders arrive on separate
   queues with no shared ordering key, the code resorts to
   `wait_for_library_reorders(timeout)` + `choice_pending` flags +
   "drain if ready" predicates to paper over arrival races. Every one
   of these is a direct violation of the "no sleeps, no retries, no
   selects, no timeout-waits" rules in
   `NETWORK_ARCHITECTURE.md` § *What This Architecture PROHIBITS*.
3. **Implicit ordering between channels.** The engine assumes
   "reorders are applied before reveals at each sync point, and reveals
   before the choice." That ordering is not encoded in data; it is
   enforced by call-site sequencing in two languages (native +
   WASM) and was the root cause of mtg-559 / `robots42`.
4. **Engine coupling to controller types.** The current code path
   special-cases "is this a local human waiting for a click?" vs "is
   this a network remote waiting for a server message?" because the
   queues holding those two kinds of data live in different objects
   reached by different APIs. The engine should not know.

## 2. The model

### 2.1 One generic primitive — `ActionLog<T>`

A single small Rust type:

```rust
pub struct ActionLog<T> {
    entries: Vec<(u64, T)>,   // (action_count, payload), strictly ascending
}

impl<T> ActionLog<T> {
    pub fn push(&mut self, action_count: u64, entry: T);   // panics if not strictly > frontier
    pub fn get(&self, action_count: u64) -> Option<&T>;     // binary-search by ac
    pub fn frontier(&self) -> Option<u64>;                  // highest pushed ac
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)>;
}
```

Lives at `mtg-engine/src/network/action_log.rs`. Tests cover monotonicity
panic, sparse / dense lookup, frontier semantics, repeated non-destructive
reads, replay equivalence, and generic-over-payload.

Invariants (in the user's words):

> The log monotonically accumulates information at known
> `action_count`s. Once appended, an entry is never lost or mutated.
> Any reader consumes the log by index as many times as it likes — on
> rewind or replay — and only ever blocks at the frontier (the highest
> `action_count` we have appended so far).

Formally:

- **Append-only.** Only the designated appender writes. No code ever
  removes or rewrites entries.
- **Strictly monotonically increasing `action_count`.** Each game
  action has a unique `action_count` (per `NETWORK_ARCHITECTURE.md`
  § *The Action Log*), and at most one log entry per `action_count` per
  log instance.
- **Non-destructive reads.** `get(k)` returns the same `&T` on every
  call.
- **Frontier-bounded.** `frontier()` is the highest `action_count`
  appended so far. A read of `action_count > frontier` is the **only**
  legitimate "I need more data" signal.

We deliberately do NOT keep a parallel `HashMap<u64, usize>` index next
to the Vec. The Vec is already sorted; binary search is O(log N) at the
≲10⁴-entries-per-game cardinality, and a side map is exactly the kind of
"second data structure shadowing the first" that motivates the whole
refactor.

### 2.2 Frontier-driven progress (no select, no sleep, no timeout)

```
engine asks for action_count K
        │
        ▼
   K ≤ frontier? ─── yes ──► return entry, engine runs deterministically
        │
        no
        ▼
return NeedsInput; Rust stack unwinds back to the caller
        │
        ▼
   (control returns to JS event loop / native runtime)
        │
        ▼
appender pushes entries for K, K+1, ... ; frontier extends
        │
        ▼
caller re-enters engine; engine re-reads the same indices, gets the same
entries, makes identical decisions → resumes past K
```

This *is* a coroutine. The trampoline is the JS event loop (WASM) or
the host driver (native). There is no `select!`, no `sleep`, no
`wait_for_*` with a timeout — the only synchronisation primitive is
"frontier moved, try again," which is naturally edge-triggered by
appender pushes.

### 2.3 Rewind / replay is free

Engine state is reset via the existing `undo_log` (which retains all
the way back to game start — see `ai_docs/reference/snapshot_architecture.md`).
Re-drive the engine; every read at `action_count K` re-reads the same
entry, deterministically. No network re-fetch, no destructive consumption.
This is the same primitive MCTS will use server-side later to evaluate
hypothetical lines.

### 2.4 Output suppression on replay (the mirror discipline)

The action log is *input*. The mirror invariant for *output* is:

> During a replay pass — the engine running over already-known inputs
> to re-derive state — it must reapply state mutations but MUST NOT
> re-emit external side-effects.

PR #11 already established this for `[GAMELOG]` stdout lines (replay
suppresses logging; see `replaying` flag in
`snapshot_architecture.md`). Phase 2 generalises this to **all**
external emissions:

- `[GAMELOG]` stdout writes — already suppressed (PR #11).
- Outbound `ClientMessage` sends — must be suppressed (otherwise
  double-submit on rewind).
- File / network sinks for analytics — must be suppressed.
- Anything else with a side-effect outside the engine's heap — must be
  suppressed.

This is an explicit, doc-level **invariant**: replay is pure with
respect to the outside world.

Desync detection collapses out of the data path: each entry can
optionally embed the post-state hash the server reported at that
`action_count`. The engine, after applying the entry, asserts its own
hash matches. Mismatch = `FATAL ERROR: DESYNC DETECTED`, per
`NETWORK_ARCHITECTURE.md` § *Desync is ALWAYS a Fatal Error*. **There is
no recovery hook.** The bounds-check in `WasmRemoteController` becomes
obsolete.

## 3. Ownership chain — two stores, one primitive

There is no single global "input log." There are **two distinct
`ActionLog<T>` instances per game session**, owned by different parties
and disjoint by construction. The engine is agnostic to which one (if
any) is involved at a given `action_count`.

### 3.1 Per-controller choice buffer (PRIVATE to each `Controller`)

Each `Controller` impl embeds an `ActionLog<ChoiceEntry>` keyed by the
`action_count` at which it was asked. The buffer is a private
implementation detail — nothing outside the controller reads it.

```rust
pub trait Controller {
    /// Engine asks: "what is this controller's choice at this action_count?"
    /// The controller decides whether to return a cached entry from its
    /// buffer (replay), to consume a freshly-arrived input and buffer it
    /// (frontier), or to yield NeedsInput.
    fn choose_at(&mut self, view: PlayerView, action_count: u64)
        -> ControllerResult<ChoiceEntry>;
    // ... other choose_* methods may follow the same pattern, or be
    // unified under a single ChoicePoint enum.
}
```

Concrete impls:

- **`LocalHumanController`**: buffers UI clicks. When the JS frontend
  submits a click at the active `action_count`, the controller's
  appender pushes to its `ActionLog<ChoiceEntry>`. The next engine
  re-entry returns the cached entry for that `action_count`.
- **`NetworkRemoteController`**: buffers `OpponentChoice` and
  `ChoiceAccepted` messages from the WS. The WS reader is the appender.
  The engine reads by `action_count`; before-the-frontier yields
  `NeedsInput`.
- **AI controllers (`RandomController`, `HeuristicController`,
  `ZeroController`, ...)**: already deterministic from the
  `PlayerView`. The default trait impl can either (a) not buffer at
  all (stateless re-derivation on every replay is cheap), or
  (b) buffer for cache-determinism. Either is fine; the engine doesn't
  care.

In a WASM game where the local human plays vs a remote opponent, the
two controllers each own their own `ActionLog<ChoiceEntry>`. Those
buffers never touch each other. The engine calls
`active_controller.choose_at(view, ac)` and the right one answers,
with no special-casing.

### 3.2 Shadow state-sync log (`NetworkClient`-owned, shadow mode only)

`RevealCard` and `LibraryReorder` from the server are **not** anyone's
"choice" — they are server-pushed mutations to the **shadow `GameState`**
(revealing card identities in the WASM shadow, fixing library order on
a `Scry`/`Brainstorm`/`Surveil`). They have no controller home.

They live in a single `ActionLog<StateSyncEntry>` owned by
`NetworkClient`, present **only** when there is a server (shadow mode):

```rust
pub enum StateSyncEntry {
    RevealCard { owner: PlayerId, card: CardReveal, reason: RevealReason },
    LibraryReorder { player: PlayerId, new_order: Vec<CardId> },
    // future: any other server → shadow-state mutation that is not a
    // controller's choice
}

impl NetworkClient {
    pub state_sync: ActionLog<StateSyncEntry>,  // shadow mode only
}
```

The engine, immediately before requesting the active controller's
choice for `action_count = K`, asks the network client (if any) to
apply any state-sync entries at K:

```rust
// engine loop sketch (Phase 2 shape; not Phase 1 code):
fn step(&mut self, ac: u64) -> StepResult {
    if let Some(client) = self.network_client.as_ref() {
        // Apply state-sync mutations at this ac, or yield if the server
        // hasn't published them yet.
        match client.apply_state_sync_at(&mut self.shadow_state, ac) {
            Ready => {}
            NeedsInput => return StepResult::NeedsInput,
        }
    }
    self.active_controller.choose_at(self.view_for(ac), ac)
}
```

Native mode (the server is its own engine, no shadow): `network_client`
is `None`; the `state_sync` log doesn't exist; nothing changes for the
engine path. This keeps the boundary clean.

### 3.3 Heap > stack: why these owners must outlive engine calls

Whichever owner you pick, the `ActionLog<T>` inside it must outlive any
single engine call so that when the engine returns `NeedsInput` and
the Rust stack unwinds, the log is still there for the next entry.

WASM (single-threaded JS event loop):

```
JsModuleInstance
  └─ #[wasm_bindgen] handle (Rust-side)
       └─ Rc<RefCell<Controller>>  /  Rc<RefCell<WasmNetworkClient>>
            └─ pub buffer: ActionLog<...>
                 └─ Vec<(u64, T)>   ← survives every engine unwind
```

Native (multi-threaded: WS reader thread + engine thread):

```
Native harness (tokio runtime + spawn_blocking engine)
  └─ Arc<Mutex<Controller>>  /  Arc<Mutex<NetworkClient>>
       └─ pub buffer: ActionLog<...>
            └─ Vec<(u64, T)>
```

Same shape, only the sync primitive differs (`Rc<RefCell>` vs
`Arc<Mutex>`). The lock wraps the *owner*, not the log itself. Reads
do not block on the frontier; they read what's there and the engine
yields `NeedsInput` to its trampoline, which the runtime knows how to
wait on (a `Condvar` notified on every push, or simply the JS event
loop firing again on the next WS message).

## 4. Threading model — same diagram, per owner

```
        ┌──────────────────────────────────────────────┐
        │             ActionLog<T> (heap)              │
        │   Vec<(u64, T)>, strictly ascending by ac    │
        └──────────────────────────────────────────────┘
              ▲ push (sole appender)        │ get(ac) (any reader)
              │                             ▼
   ┌──────────────────────┐         ┌───────────────────────┐
   │  Designated appender │         │   Engine / driver     │
   │  (varies by owner)   │         │                       │
   └──────────────────────┘         └───────────────────────┘

  WASM:   one JS thread; both sides borrow Rc<RefCell<owner>> non-overlappingly
  Native: appender thread + engine thread; Arc<Mutex<owner>>; short critical sections
```

The diagram applies **per `ActionLog<T>` instance**. Each owner picks
its own appender:

- `LocalHumanController`'s appender = the UI event handler (JS click
  → wasm_bindgen export → push to the controller's buffer).
- `NetworkRemoteController`'s appender = the WS reader, on receipt of
  `OpponentChoice` / `ChoiceAccepted`.
- `NetworkClient.state_sync`'s appender = the WS reader, on receipt
  of `CardRevealed` / `LibraryReordered`.

The sync primitive (`RefCell` / `Mutex`) wraps the owner; the
`ActionLog` itself is just a field.

## 5. What this replaces (Phase 2: DELETE)

The two-store ownership split does not change the elegance proof.
Each legacy path below is still deleted in Phase 2 — they are
fundamentally incompatible with the append-only log because they are
*destructive* reads with *implicit cross-channel ordering*. The
"Phase 2 fate" column now also names the **owner** the replacement
read lives on.

| Legacy path | Location | Phase 2 fate |
|---|---|---|
| `SharedNetworkState::drain_reveals_up_to` | `mtg-engine/src/network/client.rs:495` | **DELETE** — replaced by `network_client.state_sync.get(ac)` (§3.2) |
| `SharedNetworkState::drain_all_reveals` | `mtg-engine/src/network/client.rs:517` | **DELETE** — `state_sync.get(ac)` (§3.2) |
| `SharedNetworkState::drain_all_reveals_if_ready` | `mtg-engine/src/network/client.rs:534` | **DELETE** (and with it the `choice_pending` race flag) |
| `SharedNetworkState::wait_for_library_reorders(count, timeout)` | `mtg-engine/src/network/client.rs:569` | **DELETE** — timeout-block is forbidden by `NETWORK_ARCHITECTURE.md` |
| `SharedNetworkState::drain_all_library_reorders` | `mtg-engine/src/network/client.rs:611` | **DELETE** — `state_sync.get(ac)` (§3.2) |
| `SharedNetworkState::pending_reveals` (VecDeque) | `client.rs:342` | **DELETE** — `state_sync` is the only home (§3.2) |
| `SharedNetworkState::pending_library_reorders` (VecDeque) | `client.rs:347` | **DELETE** — `state_sync` is the only home (§3.2) |
| `SharedNetworkState::library_reorder_condvar` | `client.rs:351` | **DELETE** — frontier-notify subsumes this |
| `SharedNetworkState::choice_pending` (AtomicBool) | `client.rs:376` | **DELETE** — race flag obsolete |
| `WasmNetworkClient::pop_opponent_choice` | `mtg-engine/src/wasm/network/client.rs:827` | **DELETE** — `NetworkRemoteController.buffer.get(ac)` (§3.1) |
| `WasmNetworkClient::drain_reveals` | `mtg-engine/src/wasm/network/client.rs:852` | **DELETE** — `state_sync.get(ac)` (§3.2) |
| `WasmNetworkClient::drain_library_reorders` | `mtg-engine/src/wasm/network/client.rs:859` | **DELETE** — `state_sync.get(ac)` (§3.2) |
| `WasmNetworkClient::pending_reveals` / `pending_library_reorders` / `opponent_choices` (VecDeques) | `wasm/network/client.rs:99,108,111` | **DELETE** — split across the two `ActionLog`s by owner |
| Bounds-check / partial-drain defensive code in `WasmRemoteController` | `wasm/network/remote_controller.rs` | **DELETE** — "K > frontier" is the *only* "wait" signal |

That is **14 distinct legacy paths** dying, against the addition of
one ~100-line generic primitive plus a small number of thin owner
adapter fields.

Call-site rewrites in `wasm/network/ai_harness.rs`, `wasm/fancy_tui.rs`,
`wasm/network/exports.rs`, `wasm/network/remote_controller.rs`,
`wasm/network/local_controller.rs`, and the sync-callback wiring in
`client.rs::run_game` are itemised in
[`NETWORK_ACTION_LOG_MIGRATION.md`](NETWORK_ACTION_LOG_MIGRATION.md).

## 6. Wire-protocol note (Phase 2 protocol extension)

Today `ServerMessage::CardRevealed` and `ServerMessage::LibraryReordered`
do **NOT** carry an `action_count` field
(`mtg-engine/src/network/protocol.rs:406, 425`). To populate the
state-sync log faithfully, Phase 2 will either:

(a) **Extend the wire protocol** to add `action_count: u64` to both
    variants (preferred — explicit, validates immediately on receipt), or
(b) **Derive `action_count` on the client** by tagging every reveal /
    reorder with `server_action_count` at the moment of receipt
    (the current `SharedNetworkState::server_action_count` already
    tracks this for `ChoiceRequest` / `OpponentChoice`). This is a
    safer no-protocol-change migration step and is what Phase 1's
    shadow-log accumulator uses.

The Phase 1 primitive accepts both — `action_count` is an argument to
`push`, the appender decides how to populate it. The migration plan
defaults to (b) for the first land, then upgrades to (a) in a follow-up
so the field becomes server-authoritative.

## 7. MCTS / server-symmetry note

The same primitive serves server-side MCTS. The server's MCTS will
need to evaluate hypothetical lines: drive the engine forward over a
sequence of *simulated* choices, then rewind and try a different
choice. Each rollout creates its own `ActionLog<SimulatedChoiceEntry>`
that the MCTS driver pushes to, and that the engine reads by
`action_count` exactly like a controller buffer. Sharing the primitive
between client and server means the engine's "ask for input at
`action_count = K`" code path is identical in both modes, removing a
whole class of "works on server but not on client" bugs.

## 8. Why per-owner > one-global

The earlier sketch of this design put a *single* `ActionLog` on
`NetworkClient` and asked the engine to fish each kind of entry out of
it. That model is rejected. Three problems:

1. **It re-couples the engine to controller types.** If the engine
   knows that an `OpponentChoice` entry "belongs to" the remote
   controller and a buffered local click "belongs to" the human
   controller, it has to switch on controller identity to pick the
   right entry. The current `WasmRemoteController` bounds-check exists
   precisely because of this coupling; we want it *gone*, not
   re-shaped.

2. **It mixes data owned by different parties in one place.** A local
   human's click is the human's private state — it should not be
   appended to a log that the network client also writes to. Symmetric
   for AI controller decisions in pure local play, which have no
   `NetworkClient` at all but are still per-controller decisions
   indexed by `action_count`.

3. **It would force the global log to exist even in pure-local play.**
   In a hot-seat or solo-AI game there is no server, no shadow state,
   and no need for a state-sync stream. But each controller still
   benefits from a buffered, replayable record of its decisions
   (cheap rewind, snapshot resume). Per-owner gives every controller
   that buffer "for free"; one-global would either force a
   `NetworkClient` stub into local play or short-circuit awkwardly.

The two-store split (§3.1 + §3.2) cleanly avoids all three:

- The engine sees `controller.choose_at(view, ac)` and an optional
  `network_client.apply_state_sync_at(state, ac)`. It does not know
  the controller's concrete type, and `network_client` is simply
  `None` in pure-local play.
- Each owner's buffer is its own implementation detail.
- The same `ActionLog<T>` primitive — one ~100-line file — backs all
  of them. The DRY win is *stronger*: one type, ≥3 owners, all
  invariants reused.

## 9. Summary of invariants (the doc-level checklist)

A reviewer can check Phase 2 code against this list:

1. **Two stores per session, max.** Per-controller choice buffer
   (private), and (in shadow mode) `NetworkClient.state_sync`. No
   third channel.
2. **Each appender is the sole writer to its log.** Engine never
   appends to anyone's log.
3. **Strictly monotonic `action_count` per log.** `ActionLog::push`
   panics if violated.
4. **Non-destructive reads.** No `pop`, no `drain`, no "if-ready"
   gating.
5. **Frontier-bounded.** "Need ac K > frontier" is the only "wait"
   signal; engine returns `NeedsInput` and unwinds.
6. **No sleeps, retries, selects, timeout-waits** anywhere in the
   client-side network path. (Inherited from
   `NETWORK_ARCHITECTURE.md`.)
7. **Output suppression on replay.** All external emissions
   suppressed during a replay pass; only state mutations reapplied.
8. **Desync is a fatal assertion**, never a recovery hook.
9. **Engine is controller-agnostic.** It calls `choose_at(view, ac)`;
   it does not switch on controller type or on the presence of a
   network client beyond the optional `apply_state_sync_at` hook.
10. **Same primitive native + WASM**; the only difference is
    `Rc<RefCell>` vs `Arc<Mutex>` wrapping the *owner*, not the log.

## 10. Related docs

- [`NETWORK_ARCHITECTURE.md`](NETWORK_ARCHITECTURE.md) — north-star
  invariants (this doc is a refinement, not a replacement).
- [`NETWORK_ACTION_LOG_MIGRATION.md`](NETWORK_ACTION_LOG_MIGRATION.md) —
  ordered Phase 2 rewrite plan, test surface, rollback.
- [`../ai_docs/reference/snapshot_architecture.md`](../ai_docs/reference/snapshot_architecture.md) —
  rewind/resume mechanism the engine already provides; the
  replay-suppression invariant generalises the `replaying` flag.
- Related beads: `mtg-176` (network tracking), `mtg-228`
  (single-channel architecture), `mtg-229` (shadow desync), `mtg-559`
  (`robots42` reorder/reveal race — the proving case for Phase 2).
