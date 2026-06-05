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
