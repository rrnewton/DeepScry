# Networking Architecture Cleanup Audit — 2026-06-05

**Branch:** `review-netarch-cleanup` (slot01), off `integration` @b242cbfb.
**Status:** READ-ONLY audit. No `.rs` mutated. Cleanup EDITS are sequenced for a
later pass, AFTER the deep-AC desync fix (slot03-deepac2) lands, because several
items touch the same desync-critical files that agent is editing
(`reveal_processor.rs`, the consensus reveal buffer, the controllers, client replay).

**Lens:** `worktrees/slot01/CLAUDE.md` DRY + "PREFER STRONG TYPES / tighter types"
rules. Define ONE thing, make it as general as needed; introduce a narrower/
duplicate type only when it is a justified tighter fit.

**Architecture direction (for grounding):** the netarch effort moved to a single
ascending-`action_count` consensus buffer as the source of truth. The server now
collects every fact a recipient needs into ONE ascending-`ac` `buffer:
Vec<(u64, BufferedFact)>` carried by the next `ChoiceRequest`
(`mtg-engine/src/network/protocol.rs:802-812`). The recipient splits that buffer
by variant into the two logs it already owns: reveal-class variants →
`StateSyncEntry` (keyed by game `ac`); `BufferedFact::Choice` → `ChoiceEntry`
(keyed by `choice_seq`). The *eager* server→client message zoo
(`CardRevealed` / `LibraryReordered` / `SearchCandidates` / `OpponentChoice`) is
the pre-buffer design and is being deleted variant-by-variant. **`OpponentChoice`
is already fully dead on the send side; the other three are partially still
dual-emitted.** That asymmetry is the bulk of the cruft below.

---

## Q1. RemoteController vs WasmRemoteController duplication

**PLAIN-LANGUAGE:** There are two "opponent's-eye-view" controllers — one for the
native desktop build (`RemoteController`) and one for the browser/WASM build
(`WasmRemoteController`). They do the same job (replay the opponent's decisions
that the server sent us) but are written twice. The native one still carries a
stale comment claiming it uses an "MVar" (a blocking mailbox) to receive the
opponent's choices. That is no longer true: the opponent's choices now come from
an **append-only, replayable log read by a cursor** — exactly the
"choices-come-from-the-log, not from extra comms" model the user wants. The MVar
that remains is for a *different* purpose (delivering *our own* turn's
choice-request), not for reading the opponent's decisions.

### Field/method inventory

**`RemoteController`** (`mtg-engine/src/network/remote_controller.rs:45-55`)
- `player_id: PlayerId`
- `shared_state: Option<Arc<SharedNetworkState>>` — the network buffer handle
- `last_library_search_result: Option<CardId>`
- `pending_choice: Option<CachedOpponentChoice>` — cached choice from
  `prepare_for_priority_choice()`

**`WasmRemoteController`** (`mtg-engine/src/wasm/network/remote_controller.rs:38-55`)
- `player_id: PlayerId`
- `network_client: SharedNetworkClient` — the WASM buffer handle
- `last_spell_ability: Option<SpellAbility>`
- `last_library_search_result: Option<CardId>`
- `game_ended: bool`
- `choice_checkpoint: Option<u64>` — saved opponent-choice cursor for the
  multi-step combat-damage checkpoint/restore (mtg-sfihb)

Both implement the full `PlayerController` choice surface
(`choose_spell_ability_to_play`, `choose_targets`, `choose_mana_sources_to_pay`,
`choose_attackers/blockers`, damage-assignment, `choose_from_library`, sacrifice,
modes, …). The per-method bodies are near-identical index-decode logic
(see e.g. native `choose_attackers` lines 348-377 vs wasm 226-251; native
`choose_blockers` 379-414 vs wasm 253-281; the lethal/remaining-damage CardId-vs-
index handling native 437-564 vs wasm 315-444). The wasm file even carries a
"Code Sharing Note" admitting the duplication
(`mtg-engine/src/wasm/network/remote_controller.rs:12-16`).

### Why the extra WASM fields (duplicated vs genuinely divergent)

| field | divergent? | why |
|---|---|---|
| `last_spell_ability` | **duplicated, worse design** | native caches the whole choice in `pending_choice.spell_ability`; wasm splits one field out. Should fold into a shared payload. |
| `game_ended: bool` | **genuine** | wasm is poll-driven (`NeedInput` when the queue is empty); it must remember terminal state because there is no blocking thread. Native blocks on a condvar and learns exit via `None`. |
| `choice_checkpoint: Option<u64>` | **genuine** | wasm implements `mark/restore_choice_checkpoint` (mtg-sfihb) for the re-entrant combat-damage pass; native's blocking model doesn't re-enter the same way. |
| `pending_choice` (native only) | **genuine-ish** | native pre-fetches the choice in `prepare_for_priority_choice()` so reveals buffer before abilities compute; wasm computes lazily. |

So the **divergence is real at the concurrency-model layer** (native = blocking
condvar + prefetch; wasm = non-blocking poll + checkpoint), but the
**per-choice index-decoding bodies are pure duplication** and should be shared.

### Is the "MVar communication architecture for choices" note accurate? — NO, STALE

- `RemoteController`'s module/struct docs
  (`remote_controller.rs:5-13, 32-44, 47, 53, 78-129, 162-185, 247`) describe an
  "MVar Design / MVar mode / reads OpponentChoice from `remote_choice_mvar`,
  blocking if needed" and a "Legacy mode: Panics."
- **Empirically the opponent-choice path is NOT an MVar.** It is an
  `ActionLog<ChoiceEntry>` (`opponent_choices`, an append-only log + read cursor)
  inside `SharedNetworkState`:
  - struct `OpponentChoiceBuffer { log: ActionLog<ChoiceEntry>, cursor }`
    (`mtg-engine/src/network/client.rs:392-414, 482`)
  - `take_opponent_choice()` is a **non-destructive cursor read that blocks on a
    Condvar** and leaves the entry in the log for replay
    (`client.rs:1105-1126`); `reset` rewinds the cursor (`client.rs:1127`).
  - `RemoteController` reads exclusively through `state.take_opponent_choice()`
    (`remote_controller.rs:102, 200`).
- **This is exactly the user's desired model:** opponent choices live in an
  append-only, replayable log, read forward by a cursor; rewind = reset cursor.
  The log *is* the never-invalidated "index of where choice-actions live" the
  user described — the optimization, not extra comms.
- The ONLY surviving MVar is `local_choice_mvar: MVar<LocalChoiceInfo>`
  (`client.rs:473, 570, 1047, 1055, 1198`). That delivers **our own**
  `ChoiceRequest` from the WS reader to `NetworkLocalController` — a request
  signal, not the opponent's decision. It is legitimately a mailbox and is not
  the thing the docs are describing.

**Conclusion Q1:** The "MVar architecture for choices" note on `RemoteController`
is stale and contradicts the shipped code. The opponent-choice path already reads
from the append-only `ActionLog<ChoiceEntry>` cursor buffer. **Cleanup:**
(a) rewrite the `RemoteController` MVar docs to describe the cursor-buffer model;
(b) delete the dead "legacy mode" — see next paragraph; (c) extract the shared
index-decode bodies.

### Dead "legacy mode" on RemoteController (bonus Q1 finding)

`shared_state` is `Option<Arc<SharedNetworkState>>` with a `None` "legacy mode"
that panics (`remote_controller.rs:48, 59-66, 95-97, 232-237`). But the legacy
constructor `RemoteController::new` (line 59) is **never called** — the only
constructor used is `new_with_shared_state` (`client.rs:2258`). The `Option`, the
`None` arms, and the panic branch are dead. **Cleanup:** make `shared_state:
Arc<SharedNetworkState>` (non-optional, a tighter type), delete `new`, delete the
panic arms.

---

## Q2. `ChoiceAccepted` and `OpponentChoice` messages

**PLAIN-LANGUAGE:** `OpponentChoice` was the old "here is what your opponent just
did" push message. It has been fully replaced by the new buffer and the server
**no longer sends it at all** — but the message type and the client code that
listens for it are still sitting there, dead. `ChoiceAccepted` is the server's
"got your move" receipt; the client only actually *waits* on it for one case
(library-search), to receive the specific hidden card the server picked. The
receipt itself is otherwise unused.

### `OpponentChoice` (`ServerMessage::OpponentChoice`, protocol.rs:816-862)

- **Purpose (historical):** server → the *other* client, telling it what the
  active player chose, so its shadow could replay the move.
- **Senders:** **ZERO.** `grep -rn "ServerMessage::OpponentChoice" src/` finds no
  construction/send anywhere. The mid-game eager send was deliberately deleted
  (`server.rs:3063-3079`: "The eager `OpponentChoice` + bundled `CardRevealed`
  sends were DELETED here … fully superseded by the buffer"). Opponent decisions
  now travel only as `BufferedFact::Choice` in the next `ChoiceRequest.buffer`.
- **Receivers (still present, now dead in production):**
  - native: `client.rs:199-208` decode → `NetworkMessage::OpponentChoice`;
    `client.rs:2553-2580` WS-reader arm pushing into `opponent_choices`.
  - wasm: `wasm/network/client.rs:912` handler; a synthetic
    `OpponentChoice` is fed in a wasm test at `wasm/network/client.rs:2313`.
  - native `push_opponent_choice` keep-first dedup (`client.rs:1062-1099`) exists
    *solely* to drop the Phase-1 dual-emit duplicate — and the comment says the
    eager arm is "processed only while the buffer is not yet authoritative."
    Since nothing sends the eager copy, that whole dedup-against-eager rationale
    is moot.
- **Removable cruft? YES** (strong evidence: 0 senders). Removing it touches the
  client replay path slot03-deepac2 is editing → sequence AFTER deep-AC.

### `ChoiceAccepted` (`ServerMessage::ChoiceAccepted`, protocol.rs:864-887)

- **Purpose:** server → originating client, acking a `SubmitChoice`. Carries
  `library_search_result` (the hidden CardId the server's tutor moved to hand).
- **Senders:** server still sends it (`server.rs:2632, 2824, 3111`).
- **Receivers / who waits:** only `NetworkLocalController::choose_from_library`
  **blocks** on it (`local_controller.rs:843-863`, via
  `wait_for_choice_accepted(choice_seq)`), to obtain the server-authoritative
  `library_search_result`. Every other choice method is fire-and-forget
  (`send_choice` then return; e.g. `local_controller.rs:394, 475-483, 556, …`).
  The wasm client just logs it and moves on (`wasm/network/client.rs:958-959`).
- **Is the UI-wait still needed?** **Partly.** It is NOT a generic
  "UI waits for server" handshake — it is a **data-fetch** for one choice type:
  the searcher needs the hidden CardId the server chose. Under the buffer model
  the *searcher's own* found card already arrives at its true resolution `ac` via
  `collect_reveals_since_last_choice` / `SearchCandidates` (see the long
  rationale at `server.rs:3088-3109`), and the CardId still rides in
  `ChoiceAccepted.library_search_result`. So:
  - The **block** is a real synchronization the search path currently depends on.
  - But `ChoiceAccepted`'s sole *payload* purpose is delivering that one CardId.
    Once the buffer is the sole source and the found-card CardId is threaded
    through the buffer/state-sync log at its true `ac`, `ChoiceAccepted` becomes a
    pure latency-adding ack that could be removed (or demoted to a debug-only
    receipt). **Not safe to delete yet** — it is load-bearing for library search
    today. **Recommendation:** keep for now; file a follow-up to fold the
    library-search CardId into the buffer/state-sync path and then delete the
    block + message. Gated on the deep-AC reveal work.

---

## Q3. `BufferedFact`, the `LibraryReordered` legacy, and the duplicated `Choice` payload

**PLAIN-LANGUAGE:** `BufferedFact` is the one wire enum for "a fact the server
sends you to catch your shadow up to a decision point." Having it as its own enum
is justified — it is the union of *exactly* the catch-up fact kinds, tagged for
serialization. The problem is twofold: (1) the *old* standalone messages it
replaced (`LibraryReordered` especially) still exist and are still half-emitted,
so the codebase describes two architectures at once; and (2) the choice payload
(the indices + the 3 disambiguators) is re-declared in **five** places. We should
declare that payload ONCE and reuse it inside every enum/struct that bundles it.

### Is the separate `BufferedFact` enum justified? — YES

`BufferedFact` (`protocol.rs:416-448`) is the tagged union of the catch-up fact
kinds (`Reveal`, `LibraryReorder`, `SearchCandidates`, `Choice`) carried in one
ascending-`ac` vector. The enum/union discriminant genuinely varies per fact, so
the enum itself is the right shape. What is NOT justified is re-bundling the
`Choice` payload inline (see DRY refactor below) and keeping the superseded
standalone messages alongside it.

### The `LibraryReordered` legacy to remove (full list)

The point-in-time target is: `BufferedFact::LibraryReorder` (protocol.rs:430) is
the canonical form; the eager standalone `ServerMessage::LibraryReordered` is the
pre-buffer dual-emit. Unlike `OpponentChoice`, `LibraryReordered` is **still
actively dual-emitted** (`server.rs:3132` "The eager LibraryReordered send below
still happens"), so this is a Phase-2 removal, not an already-dead one.

Every reference that must be removed/retargeted for a single coherent architecture:

- **Wire variant + senders (the eager message):**
  - `ServerMessage::LibraryReordered { player, new_order, action_count }`
    declaration — `protocol.rs:730-745`.
  - Initial game-setup sends — `server.rs:2079-2131` (4 sends + log line). NOTE:
    these are the game-start library sync; they predate any `ChoiceRequest`, so
    they need a replacement path (initial-state delivery) before deletion, not a
    blind cut.
  - Mid-game dual-emit forward — `server.rs:3119-3140`
    (`GameToHandler::LibraryReordered` arm + the eager `conn.send`).
  - Coordinator → handler plumbing: `GameToHandler::LibraryReordered`
    (`server.rs:455, 2498-2513, 2704-2719`).
- **Client decode/handlers (eager path):**
  - native `client.rs` `LibraryReordered` decode arm (paired with the buffer
    apply at `client.rs:766-775`).
  - wasm `wasm/network/client.rs:1007-1010, 1336-1345` (buffer apply) and the
    eager handler.
- **Comments still describing the eager `LibraryReordered` protocol as current**
  (retarget to the buffer model): `state_sync.rs:47`; `controller.rs:982,
  1007`; `action_log.rs:12, 120`; `game/state.rs:311-334, 750, 819-832,
  4188, 4230, 4240`; `undo.rs:311, 2565`; `game/game_loop`/`combat.rs:553`
  surrounding notes; the "message zoo" comment `server.rs:3434`.

(Keep the *buffer* variant `BufferedFact::LibraryReorder` and
`StateSyncEntry::LibraryReorder` — those are canonical.)

### DRY refactor: ONE shared `Choice` payload struct

The same choice payload — `choice_indices` + the three disambiguators
(`spell_ability`, `library_search_result`, `target_card_ids`) — is re-declared
inline in **five** structs/variants:

| site | fields bundled |
|---|---|
| `protocol.rs:439-447` `BufferedFact::Choice` | `choice_seq, choice_type, choice_indices, description, spell_ability, library_search_result, target_card_ids` |
| `choice_entry.rs:43-83` `ChoiceEntry` | `choice_seq, action_count, choice_indices, description, spell_ability, library_search_result, target_card_ids` |
| `protocol.rs:816-862` `ServerMessage::OpponentChoice` | superset (+`player, action_count, timestamp_ms, state_hash_after, debug_info`) |
| `remote_controller.rs:24-30` `CachedOpponentChoice` | `action_count, indices, spell_ability, library_search_result, target_card_ids` |
| `protocol.rs:319-356` `ClientMessage::SubmitChoice` / `controller.rs:106-121` `ChoiceResponse` | `choice_indices, spell_ability, target_card_ids` (the client→server subset) |

**Proposal:** define one strong-typed payload struct, e.g.

```rust
/// The opponent/active-player decision content the shadow needs to replay a
/// choice — the indices plus the structured disambiguators that survive when
/// index-based lookup would point at the wrong (hidden) card. Envelope fields
/// (choice_seq, action_count, timestamps, hashes) stay on the OUTER type.
pub struct ChoicePayload {
    pub choice_indices: Vec<usize>,
    pub spell_ability: Option<SpellAbility>,
    pub library_search_result: Option<CardId>,
    pub target_card_ids: Option<Vec<CardId>>,
}
```

Reuse it inside `BufferedFact::Choice` (the enum/union part — `choice_seq`,
`choice_type`, `description` — stays on the variant; the bundled payload becomes
`ChoicePayload`), inside `ChoiceEntry`, inside `OpponentChoice` (until that
message is deleted per Q2), and replace `CachedOpponentChoice` entirely (it is a
field-renamed clone of the `ChoiceEntry` payload — `indices` vs
`choice_indices`). Add common handling helpers (e.g. one `apply_indices_to_slice`
+ one CardId-vs-index resolver) so the controllers share one decode path.
`SubmitChoice`/`ChoiceResponse` carry the client→server subset — either reuse
`ChoicePayload` (with `None` for `library_search_result`) or define a
deliberately tighter `SubmittedChoice` subset; that is a justified tighter-fit
case, document it as such.

---

## Q4. `SubmitChoice.spell_ability` — already a cross-check, but stale doc + a coverage gap

**PLAIN-LANGUAGE:** Good news — the thing the user wants (treat the spell the
client names as a *cross-check* against the index, and crash on disagreement)
is **already implemented on the server**, always-on, and cheap. The catch is two
defects: (1) the doc comment on the message says the opposite of what the code
does ("uses this directly instead of looking up by index"), and (2) the
**browser/WASM client never fills the field in** (always `None`), so the
cross-check silently never runs on the production web path — only native-vs-native
games are actually protected.

### (a) Is `spell_ability` populated for ALL Priority choices?

- **Native client: YES.** `NetworkLocalController::choose_spell_ability_to_play`
  sends `choice.clone()` for every priority choice
  (`local_controller.rs:461-484`, the only `send_choice` call that passes a
  non-`None` spell_ability).
- **WASM/web client: NO — always `None`.**
  `wasm/network/client.rs:1915-1922`: `spell_ability: None, // WASM client doesn't
  track spell_ability yet`. Since the deployed site is the WASM client, the
  cross-check is a **no-op in production**.

### (b) Does the server cross-check the index against `spell_ability`, or ignore the index?

**It cross-checks — index is canonical, `spell_ability` validates.** The
`SubmitChoice` doc comment claims the reverse:

> `protocol.rs:343-347`: "When present, server uses this directly instead of
> looking up by index."

But the actual server code does index-canonical + validation:

```
mtg-engine/src/network/controller.rs:644-687
  let idx = result.indices.first()...;  // canonical: index-based lookup
  let ability = available[ability_idx].clone();
  // VALIDATION ONLY: If spell_ability is present, verify it matches.
  if let Some(ref expected) = result.spell_ability {
      if &ability != expected { return ChoiceResult::Error("FATAL DESYNC ..."); }
  }
```

So the comment at `protocol.rs:343-347` (and the echoing one at
`controller.rs:116-120`) is **stale/inaccurate**. The behavior the user asked for
("cross-check, not replace; in perfect lockstep index and SpellAbility must
agree; assert cheaply to catch divergence early") is exactly what
`controller.rs:660-685` already does, always-on (not debug-gated), at the cost of
one `SpellAbility` equality compare.

Note the separate `spell_ability` extracted for the *opponent's* `OpponentChoice`
relay (`server.rs:2592-2600, 2784`) is recomputed from the server's own
`abilities` list by index — it does NOT consume the client's submitted field. So
the submitted `spell_ability` is *purely* the cross-check input.

### Recommendation Q4 (exact changes)

1. **Fix the stale docs** (no behavior change): rewrite `protocol.rs:343-347`
   and `controller.rs:116-120` to: "Cross-check only. The index is authoritative;
   if `spell_ability` is present the server asserts it equals the index-selected
   ability and treats a mismatch as fatal desync (always on, one equality
   compare)." — does not touch slot03 files; safe to do in the first cleanup wave.
2. **Close the production coverage gap:** populate `spell_ability` in the WASM
   client's `SubmitChoice` (`wasm/network/client.rs:1915-1922`) so the cross-check
   actually protects the deployed web games. This requires the wasm
   local-controller to thread the chosen `SpellAbility` through (mirror native
   `local_controller.rs:461-484`). Keep the assert always-on (it is cheap).
3. Keep the assert always-on (already is). No debug-gating needed — a single
   `SpellAbility` `==` per priority choice is negligible.

---

## OTHER cruft / duplication

1. **Eager message zoo is being deleted asymmetrically (the biggest cleanup).**
   - `OpponentChoice`: 0 senders → fully dead receive path (native
     `client.rs:199-208, 2553-2580`, wasm `wasm/network/client.rs:912, 2313`) +
     the dedup-against-eager logic (`client.rs:1062-1099`). **Removable now**
     (post-deep-AC).
   - `CardRevealed`: still eagerly sent at game-setup (`server.rs:2162-2206`) and
     a mid-game generic flush (`server.rs:2934`). Buffer form is
     `BufferedFact::Reveal`. Partial migration.
   - `LibraryReordered`: still dual-emitted (`server.rs:3132`). See Q3.
   - `SearchCandidates`: still sent (`server.rs:2984`). Buffer form is
     `BufferedFact::SearchCandidates`.
   - The "Phase-1 dual-emit" framing (`protocol.rs:413-415`) implies a planned
     convergence to buffer-only; finishing it removes 4 wire variants, their
     senders, their decode arms, and the dedup shims.

2. **`CachedOpponentChoice` is a renamed clone of `ChoiceEntry`'s payload**
   (`remote_controller.rs:24-30`) — `indices` vs `choice_indices`, otherwise
   identical. Fold into the shared `ChoicePayload` (Q3).

3. **`last_spell_ability` split-out in WASM** (`wasm/network/remote_controller.rs:43`)
   duplicates what native keeps inside the cached choice; unify under shared
   payload.

4. **Two near-identical remote controllers** (Q1) — extract the per-choice
   index-decode + CardId-vs-index resolver into shared free functions /
   a shared helper, keeping only the genuinely-divergent concurrency glue
   (blocking-condvar+prefetch vs poll+checkpoint) per target.

5. **Stale "MVar" naming/docs throughout the native path** — `mvar.rs` usage
   diagram (`mvar.rs:17-26`) still shows `put(OpponentChoice)`, which no longer
   happens; `remote_controller.rs` headers; `controller.rs:2148` comment. Retarget
   to "append-only `ActionLog<ChoiceEntry>` cursor buffer." The `local_choice_mvar`
   keeps a legitimate MVar; scope the rename to the opponent-choice path.

6. **Stale comment "uses this directly instead of looking up by index"** — Q4
   docs fix (`protocol.rs:343-347`, `controller.rs:116-120`).

7. **`#[allow(dead_code)]` legacy helper** `NetworkLocalController::handle_choice`
   (`local_controller.rs:383-397`) marked dead — verify and delete.

8. **`RemoteController::new` legacy constructor + `Option<shared_state>` panic
   mode** — dead (Q1). Tighten to non-optional `Arc`.

---

## Beads issues filed (all `related:mtg-o99ow`)

- **mtg-1sp35** — A1/E1: remove dead eager `OpponentChoice` + dedup-against-eager shim.
- **mtg-0jct2** — B1: DRY the choice payload (shared `ChoicePayload`).
- **mtg-yvzet** — C1/C2/Q1: dedup the two remote controllers + retarget stale MVar docs.
- **mtg-j4krs** — D/Q4: `spell_ability` cross-check — fix stale doc + populate in WASM client.
- **mtg-3ubw4** — A2/A3/A4/Q3: finish eager→buffer migration + delete legacy mode/dead helper.
- **mtg-qc2ue** — E2/Q2: fold library-search CardId into buffer, then remove `ChoiceAccepted` block.

## Cleanup PLAN (grouped; ordering vs slot03-deepac2)

slot03-deepac2 is editing `reveal_processor.rs`, the consensus reveal buffer, the
controllers, and client replay to fix a deep desync. Items touching those files
are marked **[SEQUENCE AFTER deep-AC]**. Items in protocol docs / server-only /
isolated comments are **[SAFE FIRST WAVE]**.

### (A) DELETE dead cruft
- **A1 [SEQUENCE AFTER deep-AC]** Remove `ServerMessage::OpponentChoice`
  (`protocol.rs:816-862`), the `NetworkMessage::OpponentChoice` decode + WS-reader
  arm (`client.rs:199-208, 2553-2580`), wasm handler
  (`wasm/network/client.rs:912`) + its test feed (`:2313`), and the
  dedup-against-eager rationale in `push_opponent_choice`
  (`client.rs:1062-1099`). *Risk:* touches client replay (slot03). Evidence it is
  safe: 0 senders.
- **A2 [SAFE FIRST WAVE]** Delete `RemoteController::new` + the
  `Option<shared_state>`/panic legacy mode (`remote_controller.rs:48,59-66,95-97,
  232-237`); make `shared_state: Arc<SharedNetworkState>`. *Risk:* low; native
  controller construction only (`client.rs:2258`). Coordinate with slot03 if it
  is editing this file.
- **A3 [SAFE FIRST WAVE]** Delete `#[allow(dead_code)] handle_choice`
  (`local_controller.rs:383-397`) after confirming no callers.
- **A4 [SEQUENCE AFTER deep-AC, Phase-2]** Finish the eager→buffer migration for
  `CardRevealed` / `LibraryReordered` / `SearchCandidates` (Q3 list). *Risk:*
  high — directly in the reveal/replay path; do strictly after the desync fix and
  with its own validation. Game-setup `LibraryReordered`/`CardRevealed` need a
  replacement initial-sync path before deletion.

### (B) DRY duplicative types — shared `Choice` payload
- **B1 [SEQUENCE AFTER deep-AC]** Introduce `ChoicePayload` (Q3) and reuse it in
  `BufferedFact::Choice`, `ChoiceEntry`, `OpponentChoice` (until A1),
  `CachedOpponentChoice` (delete it). Add shared decode helpers. *Risk:* touches
  `ChoiceEntry` + controllers (slot03). Sequence after.
- **B2 [SAFE-ISH]** Decide `SubmitChoice`/`ChoiceResponse`: reuse `ChoicePayload`
  subset or a deliberately tighter `SubmittedChoice` — document the tighter-fit
  justification either way.

### (C) Dedup RemoteController / WasmRemoteController
- **C1 [SEQUENCE AFTER deep-AC]** Extract the shared per-choice index-decode +
  CardId-vs-index resolver into shared free functions; keep only the divergent
  concurrency glue per target. *Risk:* both controllers are slot03-touched.
- **C2 [SAFE FIRST WAVE]** Rewrite the stale MVar docs on the opponent-choice
  path to the cursor-buffer model (`remote_controller.rs` headers; `mvar.rs:17-26`
  diagram; `controller.rs:2148`). Docs-only; safe.

### (D) `spell_ability` cross-check (Q4)
- **D1 [SAFE FIRST WAVE]** Fix the stale "uses this directly instead of looking
  up by index" docs (`protocol.rs:343-347`, `controller.rs:116-120`) to describe
  the existing always-on index-canonical + cross-check-assert behavior. Docs-only.
- **D2 [SEQUENCE AFTER deep-AC]** Populate `spell_ability` in the WASM client's
  `SubmitChoice` (`wasm/network/client.rs:1915-1922` + thread it through the wasm
  local controller) so the cross-check actually runs on the production web path.
  *Risk:* touches wasm controller (slot03-adjacent). Keep assert always-on.

### (E) Remove-or-justify `ChoiceAccepted` / `OpponentChoice`
- **E1** `OpponentChoice` → DELETE (= A1).
- **E2 [DEFER, follow-up issue]** `ChoiceAccepted`: keep for now (load-bearing for
  library-search CardId delivery via the block at `local_controller.rs:843-863`).
  File a follow-up to fold the library-search found-CardId into the
  buffer/state-sync log at its true `ac`, then delete the block + message. Gated
  on the deep-AC reveal work.

---

## File:line evidence index (quick reference)

- Native remote controller: `mtg-engine/src/network/remote_controller.rs`
  (struct 45-55; MVar docs 5-13/32-44; legacy panic 232-237;
  `CachedOpponentChoice` 24-30).
- WASM remote controller: `mtg-engine/src/wasm/network/remote_controller.rs`
  (struct 38-55; code-sharing note 12-16; extra fields 43/50/54).
- Opponent-choice buffer: `mtg-engine/src/network/client.rs:392-414, 482,
  1062-1126`.
- `local_choice_mvar` (legit MVar): `client.rs:473, 1047, 1055`.
- `OpponentChoice` 0-senders: `server.rs:3063-3079`; receivers `client.rs:199-208,
  2553-2580`, `wasm/network/client.rs:912, 2313`.
- `ChoiceAccepted`: senders `server.rs:2632, 2824, 3111`; sole blocker
  `local_controller.rs:843-863`.
- `BufferedFact`: `protocol.rs:416-448`; buffer field on `ChoiceRequest`
  `protocol.rs:802-812`; assemble `server.rs:3434-3525`.
- `LibraryReordered` eager: `protocol.rs:730-745`; setup sends `server.rs:2079-2131`;
  dual-emit `server.rs:3119-3140`.
- `ChoicePayload` candidates: `protocol.rs:439-447` / `choice_entry.rs:43-83` /
  `protocol.rs:816-862` / `remote_controller.rs:24-30` / `protocol.rs:319-356` +
  `controller.rs:106-121`.
- `spell_ability` cross-check: server `controller.rs:644-687`; native populate
  `local_controller.rs:461-484`; wasm `None` `wasm/network/client.rs:1915-1922`;
  stale doc `protocol.rs:343-347`, `controller.rs:116-120`.
