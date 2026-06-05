# Deep-AC prize blockers — diagnosis checkpoint (slot03-blockers)

**Stamp:** 2026-06-05_#2971(bcec4890) (branch `fix-deep-ac`, after the Balance/controller/doc increment)
**Author:** slot03-blockers
**Context:** slot04 desync-review (`ai_docs/DEEPAC_DESYNC_REVIEW_20260605.md`) found robots
seeds 7/11/19 still fatal, blocking the action_count re-inclusion prize (`eb8f938e`).

## What is FIXED (committed @bcec4890, validate green, merged to integration @a80e49a5)

- **mtg-d4j9v (seeds 7 & 11 — Balance):** was a LOG-ordering bug, not a state
  divergence. `execute_balance_effect` (Hand zone) emitted
  `"{p} discards {c} to Balance"` only when `cards.try_get(card_id)` was Some. On a
  network shadow the discarded opponent card's public reveal can arrive one
  ChoiceRequest AFTER the forced Balance resolution, so the line was dropped on the
  forward pass and present on the rewind replay → a +1 line-count offset that tripped
  the rewind/replay verifier (`wasm/replay_verifier.rs`). Fixed: emit unconditionally
  via `gamelog_reveal_stable`, verifier key `"{p} discards card#{id} to Balance"` (the
  3rd discard-log site of the mtg-677 reveal-timing class). **Seed 11 → PASS; seed 7 →
  advances past Balance to the deeper divergence below.**
- **mtg-f0w57:** controller reconstruction on rewind re-materialization
  (`reconstruct_controller_states`), twin of tapped. Unit-tested; robots can't exercise.
- **mtg-j4krs #1:** corrected the stale `SubmitChoice.spell_ability` doc.

Broad strict sweep (action_count RE-INCLUDED), mtime-fresh: **PASS 2,5,6,9,11,18,20,42 (8/10).**

## STILL BLOCKING — seed 7: Demonic Tutor search-to-hand (in-stack reveal lag)

**Symptom:** FATAL P1 state-hash mismatch at `choice_seq=230 action_count=1341`
(`server=23d9fef66393a9a5 client=750d1ee142e8531a`). DIFFERENCES box: **"Hand sizes
DIFFER"** — server WebRandom(P1) hand=6, client=5.

**Evidence (debug/blockers/seed7_shipped.log, ANSI-stripped):**
- Wheel of Fortune (101) resolves: WebRandom discards Copy Artifact + Su-Chi (2),
  draws 7 (ids 82,91,90,79,105,96,86) — **IDENTICAL on server and client streams**, so
  the Wheel redraw is NOT the divergence; post-Wheel hand = 7 on both.
- WebRandom casts Demonic Tutor (105) → hand 6 (tutor on stack).
- Demonic Tutor (105) resolves → "searches WebRandom's library … puts it into Hand".
- At that resolution sync point the engine logs:
  `NetworkLocalController: action_count mismatch! client=1339 server=1341 (diff=2)`
  and `RemoteController: action count mismatch! expected=1333 got=1335` /
  `expected=1338 got=1340` — i.e. the **client's undo log is 2 actions SHORT** of the
  server's, and its hand is 1 card SHORT.

**Root cause (hypothesis, strongly supported):** the shadow's forward GameLoop reaches
the Demonic-Tutor resolution sync/choice point BEFORE the search-result has been applied
to the shadow — the tutored card's library→hand move (and its undo-log action) is carried
by a reveal/SearchCandidates message that arrives slightly later, so the shadow is short
the moved card (hand 5 vs 6) and short the move action(s) (ac 1339 vs 1341). This is the
**deep-ac in-stack-resolution reveal-application lag** class (mtg-o99ow / mtg-559), the
SAME family as the historical seed-2 turn-17 reserved-card reveal-materialization timing.
It is NOT the tapped/controller re-materialization class and NOT a log-ordering bug.

**First divergence:** the put-into-hand of the Demonic-Tutor-searched card is reflected
on the server but not yet on the shadow at choice_seq=230 / ac≈1341 (client ac 1339).

**Fix direction:** the principled reveal-actionlog unification (**mtg-ho2r8**) — drive the
search-to-hand move through the action_count-keyed consensus log so the shadow applies it
in lockstep before the resolution sync point, rather than per-field/per-effect patching.

### TIGHTENED PIN (slot03-blockers, instrumented strict repro @state_hash strict toggle, 2026-06-05)

Captured the shadow's full undo dump (`debug/netarch-undo-dumps/..seed7_wasm_undo.log`)
and the server mismatch box at the fatal. Refines the hypothesis above:

- **Exactly ONE zone diverges: P1's HAND size (server 6 vs client 5).** Both sides'
  P1 library (36, sorted-identical), battlefield (16, identical incl. the just-played
  Plains 82 at `(82,false,1)`), P0 hand (7, identical ids), graveyards — ALL MATCH. The
  server's DIFFERENCES box lists only "Hand sizes DIFFER."
- **The shadow DID apply the Demonic Tutor fetch.** Its undo log shows
  `[1330] Choice(P1 #19 = LibrarySearch(Some(97)))`,
  `[1331] RevealCard(97="Fireball" to P1)`,
  `[1332] MoveCard(97 Library→Hand owner=P1)`,
  `[1333] ShuffleLibrary(P1 36)`, `[1334] MoveCard(105 Stack→Graveyard)` — i.e. the
  search-to-hand move IS present on the shadow. So the earlier "move not applied"
  framing is WRONG.
- **Around the boundary the shadow also processes P1's next main-phase land play:**
  `[1335] Choice(PlayLand 82)`, `[1336] RevealCard(82)`,
  `[1337] MoveCard(82 Hand→Battlefield)`, `[1339] LandsPlayed(P1=1)` (dump seq=242,
  local_ac=1340). So the in-stack tutor resolution flows DIRECTLY into the local
  player's next land/main-phase action with no intervening barrier.

**What is DEFINITIVE vs what still needs one more capture:**
- DEFINITIVE: at the fatal, only P1 hand SIZE differs (6 server / 5 client); everything
  else (P1 lib 36 sorted-identical, bf 16 identical, P0 hand, graveyards) matches. The
  controller logs `action_count mismatch client=1339 server=1341 (diff=2)` — the client
  is **2 actions BEHIND** the server's validation ac and **1 card short** in P1's hand.
- NOT yet pinned (blocked on a missing capture): exactly WHICH 2 server actions the
  client lacks, and which card is the hand difference. The shadow's own undo log shows
  it applied BOTH the tutor fetch (97→hand) and the land play (82 out), so the
  reconciliation requires the **server-side** undo log at the same ac — and
  `--network-debug` does NOT capture it (`..seed7_server_undo.log` is header-only,
  "no SERVER full-undo dumps captured"). The client-behind-by-2 + hand-short-by-1
  direction is consistent with the in-stack reveal/move LAG (the shadow has not yet
  applied a server-authoritative `RevealCard`+`MoveCard(→P1 hand)` pair at the
  validation ac), but proving it needs the server dump.

**NEXT DIAGNOSTIC STEP (do this first):** add a server-side full-undo dump to
`--network-debug` (mirror the WASM `WASM_FULL_UNDO_DUMP` at the coordinator/handler so
`..seed7_server_undo.log` is populated), re-run seed 7 strict, and diff the server vs
shadow undo logs at ac 1339–1341 to name the exact 2 missing actions + the missing card.
THEN decide the fix.

**Root class & assessment:** the shadow's choice-point/ac bookkeeping is not held in
lockstep with the server's per-choice validation across an in-stack resolution that flows
directly into the local player's next main-phase action — the in-stack **lockstep** half
of mtg-ho2r8 (NOT the missing-opponent-delta half the design doc §1-2 covers). It spans
`network/server.rs` choice_seq/ac stamping + `wasm/network/client.rs` apply/advance
cursors + the fancy_tui sync loop = netarch rearchitecture, NOT a one-session surgical
patch. Handed off at this pin; do not band-aid.

## STILL BLOCKING — seed 19: Fireball option-set divergence (mtg-8ow9h)

**Symptom:** `DESYNC DETECTED: NetworkController 1 received invalid choice index 2
(only 2 options available). Client sent indices [2]` at WebRandom's Fireball (118) cast,
**turn 24, X=0, ~choice_seq 322** (debug/blockers/seed19_shipped.log). The client's
info-independent local controller enumerated ≥3 options where the server has 2 → it sent
index 2 (the 3rd), which the server rejects. Persists in strict mode (action_count
included), so the upstream shadow divergence is in a field that does NOT enter the view
hash but DOES affect option generation (e.g. mana availability / a tapped-or-zone state
that happened to hash-match) — the mtg-0e1wo controller option-set family.

**Next instrumentation (recommended):** finish **mtg-j4krs #2** (populate
`SubmitChoice.spell_ability` in the WASM client, mirroring `local_controller.rs:461-484`)
so the server's always-on cross-check crashes EARLIER by CardId at the priority choice,
pinning exactly which ability the client over-generated. Then bisect the first
shadow-state divergence upstream of turn 24.

## Box note
All above is from existing captured logs (read/analysis only). A fresh instrumented repro
(per-action WebRandom hand-count + the reveal-application order around the tutor) needs a
network-e2e run — gated on validate-box availability (no netns isolation; mtg-vnirl pending).
