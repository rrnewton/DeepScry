# Deep-AC in-stack/reveal desync — seed-5 root-cause pin (2026-06-05)

Branch: `fix-deep-ac` (slot03-deepac), off integration @b242cbfb (has mtg-725 +
mtg-677 + the 3 other tonight fixes). Class: mtg-559 / mtg-752 / mtg-677 tail.
This is the FINAL action_count-prize blocker.

## Repro (deterministic, mtime-fresh, isolated)

```
python3 scripts/kill_zombie_processes.py
cd web && systemd-run --user --scope --quiet node test_network_gui_e2e.js \
  --deck decks/old_school/03_robots_jesseisbak.dck --seed 5 --network-debug --undo-dump
```

FATAL: `P2 state hash mismatch! server=fe79d428add11897 client=80de73e9c1540d20
at choice_seq=160 action_count=965` (stable across runs with fresh server+wasm).

Native AI = P1 (index 0, `connect --controller random`); browser = P2 (index 1,
the WASM shadow that fails). Turn 14, Upkeep, active=1 (P2's own turn).

## EXACT diverging field (byte-pinned)

Server vs WASM-client view at ac=965 are byte-identical in EVERY hashed field
(life, hand/lib sizes, both graveyards, stack, all other battlefield cards,
controllers) EXCEPT one tuple:

| card | server | wasm client |
|------|--------|-------------|
| 14 (Mox Emerald, owner/controller = P0 index 0) | `(14, true, 0)` TAPPED | `(14, false, 0)` UNTAPPED |

Card 14 is the OPPONENT's (P0's) Mox Emerald, cast and **tapped for mana** during
P0's turn 13. Turn 14 is P2's turn, so P1/P0's permanents must STAY tapped through
P2's untap step. Server is correct (tapped); the WASM shadow wrongly shows it
untapped.

Server "SERVER STATE" box captured via a harness diagnostic (the WASM client
sends `debug_info=None`, so the server's `DIFFERENCES:` section is normally
suppressed — see `web/test_network_gui_e2e.js` deep-ac dump tweak; full server
stderr now persisted to `*_server_stderr_full.log`).

## Action log: server and client are IDENTICAL

Both undo logs (server `*_server_undo.log`, wasm `*_wasm_undo.log`) contain, in
the seq-160 block:

```
[ 922] Choice(P0 ... CastSpell { card_id: 14 })     # P0 casts Mox Emerald
[ 923] RevealCard(14 = "Mox Emerald" to ALL ...)
[ 929] MoveCard(14 Stack -> Battlefield owner=P0)
[ 934] Tap(14)                                       # P0 taps Mox for mana
...
[ 960] Turn(14 P0 -> P1)
[ 962] Untap(118)  [ 963] Untap(123)                 # P2's permanents only
[ 964] Step(Untap -> Upkeep)
```

`Tap(14)@934` is present; there is **NO `Untap(14)`** in either log. So the
action logs agree card 14 is tapped — yet the WASM **live** `card.tapped` field
is false. The live state contradicts the shadow's own action log.

## Empirical eliminations (probes, since reverted)

Temporary `DEEPAC_*` probes (in `untap_step`, `reveal_processor`, `Card::untap`)
+ harness dump of WASM browser-console probe lines (`*_deepac_wasm.log`):

1. **Untap step is innocent.** At the turn-14 untap step the probe shows card 14
   is ALREADY untapped and is NOT in `normal_to_untap=[118,123]` (its
   controller 0 ≠ active 1, correctly excluded). The tap is lost BEFORE the
   untap step.
2. **No untap code path runs on card 14.** `DEEPAC_CARD14_UNTAP count = 0`:
   `Card::untap()` is never called on card 14. `untap_permanent` would log a
   `TapCard{false}` entry (none exists). So the tap is not "cleared" — it is a
   non-undo-logged divergence.
3. **No instance replacement.** `EntityStore::insert` is write-once (panics on an
   occupied slot); `process_card_reveal` only `insert`s when `!card_already_known`.
   Card 14 is not re-instantiated.
4. **Reveal re-materialization observed but inert.** Card 14 gets repeated
   `reason=Played owner=0 already_known zone=Battlefield` reveals during the
   buffer batch — but a re-reveal of a known card only does mask reconciliation,
   not re-instantiation.
5. **Shadow replays from scratch each choice** (777 untap-step probes, 44398
   reveal probes for a 14-turn game). At seq-159 (turn-13 end) the replay tapped
   card 14 correctly (`(14,true)`); at seq-160 (turn-14, longer replay) the same
   opponent actions leave it untapped.

## Mechanism (root)

`tap_permanent` only logs `Tap` after a successful `card.tap()` on a live
instance, so card 14 WAS tapped at some point in the seq-160 replay; by hash time
its live `tapped` is false with no logged/explicit untap. This is a **non-undo-
logged tap-state divergence** introduced by the reveal/replay-ordering model:

`NetworkStateSyncBuffer::apply_state_sync_*` (Pass 2, REVEALS) applies reveals
**EAGERLY ahead of the shadow's replay position**, on the documented assumption
that "reveals are identity injections (library-order independent), so applying
them early is safe" (`mtg-engine/src/network/client.rs` ~L914-919; WASM mirror in
`src/wasm/network/client.rs`). That assumption HOLDS for library order but FAILS
for an opponent permanent whose **tap-state is set by a replayed action** (Mox
tapped for mana the same turn): eager materialization + forward replay leaves the
live instance untapped while the `Tap` action is still recorded — action_count
parity preserved, hashed state wrong. This is precisely the mtg-o99ow
"reveal/materialization not aligned at the correct action_count" class, surfaced
now that mtg-725/mtg-677 let the robots seeds run this deep.

This is why action_count-in-hash (the eb8f938e prize) cannot yet be re-enabled:
the underlying shadow STATE genuinely diverges here (not just the count).

## Fix direction (next agent)

Principled (no band-aid, no verifier normalization that hides the divergence):
make opponent-permanent materialization + derived tap/other per-instance state
align with the action_count at which the replay executes the corresponding
action, so a replayed `Tap`/`Untap`/state mutation lands on the correct live
instance. Options to evaluate (the reveal-actionlog unification, mtg-752):
 - Do NOT apply a reveal eagerly when it materializes a permanent whose
   subsequent same-turn tap/state actions will be replayed; bound reveal apply by
   the shadow's replay position for the battlefield-permanent case (mirror the
   POSITIONAL reorder bound), keeping eager apply only for true identity-only
   injections (hand/library/graveyard reveals).
 - OR drive the permanent's tap (and analogous per-instance derived state)
   through the action_count-keyed log so it replays deterministically alongside
   the undo log, rather than being a forward-only mutation that the eager reveal
   front-runs.

### Decisive next probe to nail the last hop

Instrument EVERY write to card 14's `tapped` field (a `set_tapped` chokepoint or
a watch at hash-compute time asserting `live.tapped == replayed-log tap-state`).
The non-logging writer that flips 14 to false (Card::untap ruled out) is the
exact line to fix. Suspect: a zone-move / ETB / clone / snapshot-restore path
that resets `tapped` after the logged tap, ordering-dependent on the eager reveal.

## Acceptance (the prize)

After the fix: robots seeds 2 AND 5 `--network-debug` converge end-to-end; then
re-apply eb8f938e (re-include `action_count` in `compute_view_hash`, delete the
DELIBERATELY-EXCLUDED note, flip the pinning test), add seeds 2/5 to
`web/test_network_multideck.js`, and run the FULL un-excluded canary + `make
validate` (native + WASM, mtime-fresh) GREEN with NO exclusions. team-lead runs
the adversarial empirical seed-2/5 desync review before merge.

## Harness diagnostics kept on this branch (gitignored debug output)

`web/test_network_gui_e2e.js`: the server mismatch box is now captured even
without a client `DIFFERENCES:` section, full server stderr is persisted to
`debug/netarch-undo-dumps/*_server_stderr_full.log`, and WASM `DEEPAC_` probe
lines to `*_deepac_wasm.log`. The engine `DEEPAC_*` probes themselves were
reverted (engine tree clean).
