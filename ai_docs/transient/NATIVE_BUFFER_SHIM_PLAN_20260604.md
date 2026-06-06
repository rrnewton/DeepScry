# Native buffer shim (TASK 1 / "Piece 2") — implementation plan

**Branch:** netarch-reveal-actionlog-unify. **Author:** slot02-netarch.
**Status:** PLANNED (not started). TASK 0 settled first (see below + beads mtg-752).

## Why this is now GATING (TASK 0 result, 2026-06-04)

`network_vs_local_equivalence_e2e.sh 3 zero zero` is **branch-introduced
nondeterminism**, NOT pre-existing mtg-725. Proof: integration is 5/5
deterministic green (always 24 turns); this branch is ~50% desync (turn 5-6/11
fatal sync-mismatch). The server is fully deterministic (identical shuffle
every run); the desync is entirely in the **native client shadow** on the
cycling/library-search reveal path (shadow ~50% misses the fetched card →
hand off-by-one → fatal hash mismatch).

Root cause pinned by single-variable diagnostic: this branch's new
shuffle→`LibraryReordered` emission (`game/state.rs` ~818-837, added for
mtg-744 Timetwister stale-library) fires an EXTRA async message that races,
over the websocket, against the found-card reveal; the native EAGER
(synthetic-ac, order-sensitive, greedy-drain) apply path drops the found-card
reveal when they arrive out of order. Disabling JUST that emit → equiv-zero
8/8 green (diagnostic only; reverted — removing it permanently reintroduces
mtg-744).

**The principled fix IS this shim:** make native consume the single
ascending-`action_count` buffer carried in `ChoiceRequest` (one ordered
message instead of N racing eager messages), exactly as WASM already does.
This removes the race by construction and resolves equiv-zero.

## Reference: the WASM buffer-driver (already works, modulo the WASM-only B2)

`mtg-engine/src/wasm/network/client.rs`:
- `buffer_is_authoritative: bool` (field ~314, init false ~420); set true on the
  first `ChoiceRequest` (~913). Once true, the eager `CardRevealed` (~856),
  `LibraryReordered` (~1029), `SearchCandidates` (~1059), `OpponentChoice`
  (~945) arms early-`return` (opening-hand precedes the first ChoiceRequest, so
  it still flows through eager).
- `apply_choice_buffer(buffer)` (~1341-1393): routes each `(ac, BufferedFact)`:
  - `Reveal{owner,card,reason}` → `push_state_sync(ac, StateSyncEntry::RevealCard{...})`
  - `LibraryReorder{player,new_order}` → ac==0 ⇒ `initial_library_orders`; else
    `push_state_sync(ac, StateSyncEntry::LibraryReorder{...})`
  - `SearchCandidates{searcher,cards}` → `push_state_sync(ac, StateSyncEntry::SearchCandidates{...})`
  - `Choice{choice_seq,...}` → `record_opponent_choice(ChoiceEntry{choice_seq, action_count: ac, ...})` (keyed by choice_seq, NOT ac)
- `push_state_sync` (~1440-1466): `state_sync.insert_sorted(ac, entry)` (NOT
  push) — tolerant of out-of-order arrival; drop same-ac equal delta;
  FATAL on differing delta at same ac OR new delta behind the cursor.
- `apply_state_sync_at(shadow, local_player, target_action)` (~1570-1677): the
  L4 block-on-miss. `bound = target_action.min(frontier).min(max_received_choice_ac)`
  (~1602). `max_received_choice_ac` bumped by `note_received_choice_ac` on every
  ChoiceRequest/OpponentChoice (~1473). Two-pass in window: LibraryReorder
  first, then RevealCard/SearchCandidates → `process_card_reveal_wasm`. Cursor
  `last_applied_state_sync_ac` advanced per entry.

## Native current path (the gap)

`mtg-engine/src/network/client.rs` (2476 lines):
- `NetworkMessage::from_server_message` (~121-220): `ServerMessage::ChoiceRequest`
  arm (~133) destructures with `..` — **`buffer` is silently dropped**.
  `NetworkMessage::ChoiceRequest` (~68-78) has no `buffer` field.
- `StateSyncBuffer` (~283-297): `log: ActionLog<StateSyncEntry>`, `next_ac`
  (SYNTHETIC monotonic allocator), `last_applied_ac`.
- `push_state_sync`/`push_reveal`/`push_library_reorder` (~537-582): bump
  `next_ac` and `log.push(next_ac, ...)` — synthetic key, NOT server ac.
- `apply_state_sync_up_to_frontier(game, card_db, local_player)` (~583-643):
  applies `last_applied_ac < ac <= frontier()`, two-pass (reorder→reveal),
  `process_card_reveal`. **No `max_received_choice_ac` watermark; no target bound.**
- `run_ws_reader_shared` (~2024-2198): eager arms push reveals/reorders/choices
  unconditionally. `SearchCandidates` is EXPANDED into N synthetic-ac reveals
  (~2050-2055).
- `run_game` `sync_callback` (~1896-1912): `move |game, _target_action| {
  apply_state_sync_up_to_frontier(...) }` — **ignores `target_action`, drains
  greedily to frontier.** Installed via `GameLoop::with_sync_callback` (~1942);
  `GameLoop::sync_to_action` (game_loop/mod.rs:769) calls it with
  `game.action_count()`.
- `RemoteController` consumes opponent choices via `take_opponent_choice`
  (condvar). No client-side rewind in native (blocking-thread model).

Shared: `ActionLog<T>` (network/action_log.rs: `push` ~100 strict-monotonic;
`insert_sorted` ~140 out-of-order-tolerant, panics on exact-dup; `get`/`frontier`/
`iter`). `process_card_reveal` native wrapper (client.rs ~2330) →
`reveal_processor::process_card_reveal`. `StateSyncEntry::SearchCandidates`
EXISTS in state_sync.rs (WASM uses it; native currently does not).

## Implementation steps (incremental, test equiv-zero after each milestone)

1. **Carry the buffer to the reader.** Add `buffer: Vec<(u64, BufferedFact)>`
   to `NetworkMessage::ChoiceRequest`; stop dropping it in
   `from_server_message` (replace `..`-drop of `buffer` with capture).
2. **`buffer_is_authoritative` on `SharedNetworkState`** (AtomicBool). Setter
   invoked on first ChoiceRequest in `run_ws_reader_shared`.
3. **Native `apply_choice_buffer`** in the reader (or a SharedNetworkState
   method): route facts EXACTLY as WASM — Reveal/SearchCandidates/LibraryReorder
   → state_sync keyed by **server ac** via a new `push_state_sync_at(ac, entry)`
   that uses `log.insert_sorted` (out-of-order tolerant); ac==0 LibraryReorder →
   the native initial-orders path (see `wait_for_game_start` ~1688). Choice →
   `push_opponent_choice(ChoiceEntry{choice_seq, action_count: ac, ...})`.
   ADD native `StateSyncEntry::SearchCandidates` handling in
   `apply_state_sync_up_to_frontier` (mirror WASM; do NOT re-expand into N
   synthetic reveals — that defeats true-ac keying).
4. **Gate the eager arms**: when `buffer_is_authoritative`, the
   CardRevealed/LibraryReordered/SearchCandidates/OpponentChoice reader arms
   become no-ops (opening-hand/pre-first-choice still flows). This makes the
   buffer the SOLE mid-game source — satisfies the brief's false-positive guard
   (buffer provably drives native, not dead weight). Cleanest: also short-circuit
   in `from_server_message` is NOT possible (no shared-state access there) — gate
   inside the reader loop where `shared_state` is in scope.
5. **L4 watermark + target-bounded apply** (the load-bearing replay-driver bit):
   - Add `max_received_choice_ac` to `SharedNetworkState` (AtomicU64), bumped on
     every ChoiceRequest/OpponentChoice (server ac).
   - `apply_state_sync_up_to_frontier` → bound by
     `target.min(frontier).min(max_received_choice_ac)`; add a `target` param.
   - `sync_callback` must PASS `target_action` through (stop ignoring it):
     `move |game, target| apply_state_sync_at(game, ..., target)`. This keys the
     shadow advance to the shadow's own `action_count()` (which == server ac in
     the buffer-driven replay), applying only reveals at ac ≤ the shadow's
     current position — the WASM model.
   - Keep `wait_for_state_sync_frontier` semantics but block on the **server ac**
     watermark, not synthetic frontier count.
6. **mtime-verify + test** after each milestone:
   - `cargo build --release --features network`; confirm binary mtime > source.
   - equiv-zero seed3 ×8 isolated (`systemd-run --user --scope`): must be 8/8
     green (the regression gate).
   - native sweep: equiv-random ×3, equiv-heuristic, cycle315 ×3, robots42 ×4.
7. **FALSE-POSITIVE GUARD (mandatory):** prove the buffer drives native — e.g.
   temporarily make the eager state-sync arms `debug_assert!(false)` when
   buffer-authoritative (or log a counter) to confirm zero eager reveals are
   applied mid-game. "Native green" counts ONLY if the buffer is the sole source.

## TASK 2 (after TASK 1 green): delete eager opponent-choice send
Server `server.rs` ~3005-3084 (OpponentMadeChoice/OpponentChoice + eager
CardRevealed bundle). Collapse `record_opponent_choice` dedup to a bare push.
KEEP a minimal explicit initial-sync (opening-hand reveals + initial library
orders) OR fold into the first ChoiceRequest buffer — do NOT resurrect
dual-emit of mid-game choices.

## Gotchas / invariants
- Shadow `action_count()` MUST align with server ac in the buffer-driven replay
  (it does in WASM; native runs the same GameLoop). Verify early with a logged
  assert at the first ChoiceRequest (shadow ac vs ChoiceRequest.action_count).
- `SearchCandidates` = ONE entry at the search-resolution ac (do not re-expand).
- Searched-dummy reveal stays at the search-resolution ac (mtg-728).
- Distinct ac per delta; same-ac equal delta = drop, differing = FATAL.
- Desync ALWAYS fatal; never paper over. equiv-zero is a GATE, not an exclusion.
- HARD STOP + surface if the buffer-driven native path won't converge.
