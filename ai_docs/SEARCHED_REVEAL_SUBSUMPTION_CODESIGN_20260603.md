# Searched/Reorder Reveal Subsumption — Co-Design Note (2026-06-03)

**Context:** netarch reveal-action-log unification (`mtg-o99ow`, branch
`netarch-reveal-actionlog-unify`, slot01). This note is the **co-design
checkpoint** the orchestrator requested before touching `searched_card_for`
semantics / starting 4a-client. It shows how server-authoritative
*reorder*/*search* reveals stamped by **game action_count** subsume class-A
residual #1 (`mtg-yexvc`), and the exact decisions to lock before 4a.

**HOLD POINT:** do NOT touch the dummy `Searched`-reveal stamp (server.rs
~2992) nor key the client ActionLog by game ac (4a-client) until the
"Decisions to lock" below are agreed. step-3 (draw-reveal stamping, merged)
did NOT touch any of this.

## 1. The single alignment contract
Every server-authoritative delta the shadow cannot compute itself
(revealed card identity, RNG-shuffled library order, opponent fetched-card
id) is **stamped at the EXACT game action_count where the shadow's
deterministic replay reaches the point that consumes it.** Then the shadow,
keyed by game ac (4a-client) and blocking on miss (4b/4c), applies the
delta in strict ac order. "DONE = action logs always aligned, identical
modulo reveal-name visibility" falls out of this one rule.

The canonical ac per delta kind:
- **draw reveal** → the draw's own `RevealCard`/`MoveCard` action ac. ✓ DONE (step 3 @0772675a, for the `collect_reveals_since_last_choice` draws).
- **shuffle reorder** → the `ShuffleLibrary` action's own ac. ✗ NOT EMITTED TODAY (residual #1 — see §2).
- **scry/surveil reorder** → the `ReorderLibrary` action's own ac. Partially present: scry/surveil push `pending_library_reorders` (state.rs:2066/2152) but the wire `LibraryReordered` carries NO ac field yet.
- **opponent `Searched` fetch (dummy, hidden id)** → the **LibrarySearch resolution ac** (where the shadow records the `LibrarySearch(Some(id))` ChoicePoint). KEEP here; do NOT move to an earlier `RevealCard` position (see §3 RISK).

## 2. Residual #1 mechanism (CONFIRMED) and how the model subsumes it
`mtg-yexvc` decisive evidence: seed-2 turn-16, after the Timetwister
server-RNG shuffle, the shadow's P1 hand is missing card 105 — "the shadow
can't reproduce P1's post-shuffle library order." Root cause CONFIRMED in
code: `GameState::shuffle_library` (state.rs:745) logs `GameAction::ShuffleLibrary`
(for undo) but does **NOT** push `pending_library_reorders`, so NO
`ServerMessage::LibraryReordered` is ever emitted for a shuffle. (Only
scry/surveil enqueue reorders — state.rs:2066/2152.) The shadow's library
order after a shuffle is therefore stale/divergent; subsequent draws pop the
wrong CardIds → wrong/absent identities → card 105 missing.

Subsumption (this is step 6 of the sequence, but it is the SAME mechanism as
4a, not a separate effort):
1. `shuffle_library` becomes a `LibraryReordered` emission point, stamped at
   the `ShuffleLibrary` action's own ac.
2. `ServerMessage::LibraryReordered` GAINS `action_count: u64` (protocol.rs:658
   — it lacks it today). Same additive shape as `CardRevealed.action_count`.
3. Post-shuffle draw reveals are already keyed at their own draw ac (step 3).
4. Shadow keyed by game ac + block-on-miss applies, in strict ac order:
   `ShuffleLibrary`-ac reorder → then each draw-ac reveal → reproduces P1's
   post-shuffle hand exactly → card 105 present → seed-2 turn-16 desync GONE.
This is why the unification SUBSUMES the class-A residual: residual #1 is a
"missing server-authoritative delta at the right ac," identical in kind to a
reveal.

## 3. The Searched-reveal seam + the RISK to `searched_card_for`
`searched_card_for(searcher, target_action)` (wasm/network/client.rs:1254)
is the mtg-mb668 fix: for an OPPONENT tutor, the server sends a dummy
`Searched` reveal (empty name, authoritative CardId) stamped at the search
**choice** ac; the shadow picks the dummy `Searched` reveal owned by
`searcher` with the GREATEST `effective_ac <= target_action`, where
`target_action` is the ac at which the shadow resolves that search. Distinct
searches carry distinct (strictly larger) acs, so each resolution selects its
own reveal.

RISK: 4a-server's "stamp ALL reveals at their own `RevealCard` forward_idx"
must **NOT** apply to the dummy `Searched` reveal. Its alignment ac is the
**search-resolution ac**, not an earlier `RevealCard` log position. If we
re-stamped it earlier, `searched_card_for`'s "greatest eff_ac <= target"
selection would pick the wrong reveal (or none) → reintroduce the mtg-mb668
desync. So: **the `Searched` dummy stays stamped at the LibrarySearch
choice/resolution ac.** When 4a-client keys the ActionLog directly by game ac
(making `effective_ac_of` the identity), `searched_card_for` reads the key
directly — semantics PRESERVED iff this stamp invariant holds.

(Note: our OWN named search candidates + own fetched result are matched out by
`searched_card_for` via the `!name.is_empty()` filter, so their stamping is
free to change — but there is no reason to move them either; they too belong
at the search ac.)

## 4. Strict-monotonicity precondition for killing the synthetic key
4a-client keys `ActionLog<StateSyncEntry>` directly by game ac and deletes the
synthetic counter + `state_sync_effective_ac` map. `ActionLog::push` requires
**strictly increasing** acs, so two deltas at the SAME game ac would panic.
Audit needed before 4a-client: can two server-authoritative deltas share one
ac? Candidates: a scry that BOTH reorders AND reveals at one action; a
shuffle + an immediate reveal at the same ac. If any genuine collision exists,
options (co-design): (a) give each its own micro-action ac (preferred — they
ARE distinct undo-log actions); (b) a composite key `(ac, seq)`; (c) a
`SmallVec` payload per ac. Leaning (a): reorder and reveal are already
SEPARATE undo-log actions with distinct positions, so distinct acs should fall
out naturally — must verify.

## 5. Decisions to LOCK with orchestrator before 4a
1. Canonical ac per delta kind = §1 table (confirm).
2. `Searched` dummy stays at the search-resolution ac (confirm the invariant).
3. `LibraryReordered` gains `action_count: u64`; `shuffle_library` emits it at
   the `ShuffleLibrary` ac (confirm; this is residual-#1 fix folded into 4a).
4. Collision audit outcome (§4) → choose (a)/(b)/(c).
5. Sequencing vs mtg-mb668: since searched_card_for + reorder alignment ARE the
   class-A residual, do we land 4a (which makes them ac-aligned) as the fix for
   mtg-yexvc seed-2/seed-5 directly, with robots42 un-excluded-green as the
   acceptance gate? (Orchestrator owns this since slot03 archived.)

## 6. mtg-677 prerequisite (unchanged)
The draw-step BLOCK/rewind keying (4b/4c wiring the draw site to yield
NeedsInput / native `wait_for_state_sync_frontier`) still depends on draw-step
rewind-completeness. Per the 26c5a460 reassessment the guard family is gone +
both net paths rewind-replay, so the draw step is largely satisfied; the
in-stack-resolution residual is what 4a+ closes. Keep 4b's new block point
gated on confirming the draw-step rewind holds under the new keying.
