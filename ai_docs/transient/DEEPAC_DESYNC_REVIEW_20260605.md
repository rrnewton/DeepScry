# Adversarial desync review — slot03 deep-AC rewind fixes + action_count prize

**Reviewer:** slot04-desyncreview (independent)
**Date / stamp:** 2026-06-05_#2971(0f74f4d6)
**Branch reviewed:** `fix-deep-ac` fix tip `dd9d44ee` (review branch `review-deepac` @ `0f74f4d6`)
**Fix diff under review:** `git diff 6bebf491..dd9d44ee`
**Build:** mtime-fresh server + wasm rebuilt before every run; seed-5 (the byte-pinned case) confirmed PASS, proving the fix is live in the binary tested.

---

## VERDICT: **REJECT** (do not re-include `action_count` / merge the prize yet)

**PLAIN-LANGUAGE bottom line:** slot03's two fixes are *correct and safe* — they
repair the two specific desyncs they targeted (robots seeds 2 and 5) and they
break nothing. But they are **not the whole job**, and the highest-stakes change
(turning the action-counter back on as a cross-machine equality check) must
**not** be merged yet, for two independent reasons:

1. **Empirical (the decisive one):** of the broad set of historically-failing
   robots games, **3 of 10 (seeds 7, 11, 19) still crash with a fatal
   "the two machines disagree" error** — and they crash *whether or not* the
   action-counter is included. So the disagreement is in the *real game state*,
   not in the counter. You cannot ship a green test gate while 3 games red.

2. **Analytical (completeness):** when the client rebuilds an opponent's
   permanent during a rewind, it rebuilds it from the card's blank template —
   losing **every** per-instance fact about that card (how much damage it has,
   its counters, whether it has summoning sickness, what auras are attached, who
   controls it, …). slot03's fix restores **only one** of those facts (tapped or
   not). The single field that is *also* checked by the equality hash and is
   *still* unrestored is **controller** — the exact twin of the tapped bug, in
   the other hashed field. The robots deck happens to contain no
   control-changing cards, so a 100%-green robots sweep **cannot** exercise it.
   This is the "green masks a coverage gap" trap that already produced one false
   positive on this very feature.

**The encouraging nuance:** re-including `action_count` is *itself* sound. On all
7 converging seeds it produced **zero** new false positives — where the state
agrees, the counter agrees too. The blocker is the remaining rewind/replay
state bugs, not the counter change. Once those are fixed, the prize should land.

---

## Empirical results — robots deck, `--network-debug`, per-choice strict view-hash

Harness: `cd web && systemd-run --user --scope --quiet node test_network_gui_e2e.js
--deck decks/old_school/03_robots_jesseisbak.dck --seed <N> --network-debug`
(zombies killed between runs; ports random; mtime-fresh rebuild for each config).

| seed | action_count EXCLUDED (current) | action_count RE-INCLUDED (prize) | failure point |
|-----:|:-------------------------------:|:--------------------------------:|---------------|
|  2   | PASS                            | PASS                             | — (40 turns) |
|  5   | PASS                            | PASS                             | — (byte-pinned case, now fixed) |
|  6   | PASS                            | PASS                             | — |
|  9   | PASS                            | PASS                             | — |
| 18   | PASS                            | PASS                             | — |
| 20   | PASS                            | PASS                             | — |
| 42   | PASS                            | PASS                             | — (clean control) |
|  7   | **FAIL** (REWIND/REPLAY)        | **FAIL** (REWIND/REPLAY)         | Balance resolution, buffer idx 68 (prefix 14/15) |
| 11   | **FAIL** (REWIND/REPLAY)        | **FAIL** (REWIND/REPLAY)         | Balance resolution, buffer idx 382 (prefix 25/31) |
| 19   | **FAIL** (DESYNC)               | **FAIL** (DESYNC)                | invalid choice index 2 at Fireball cast (turn 24) |

Failures are **deterministic** (seeds 7 and 19 re-run identically, same byte-pinned
divergence offsets). Raw logs: `debug/deepac-review/seed*_{excluded,included}.log`;
undo dumps under `debug/netarch-undo-dumps/`.

### What the failures actually are

- **Seeds 7 & 11 — same bug class (the mtg-677 rewind/replay class, NOT
  tapped/reorder).** Both diverge **inside `Balance`'s resolution** — a multi-step
  mass effect (equalize lands → sacrifice → equalize hands → discard → equalize
  creatures → sacrifice). On rewind+replay the client reproduces Balance's
  *sub-steps in a different order* than its own forward pass:
  - seed 7: expected `"Balance: Creature equalize to 0"`, replay produced
    `"NativeAI discards Braingeyser to Balance"`.
  - seed 11: expected `"card#18 is discarded"`, replay produced
    `"NativeAI discards Strip Mine to Balance"`.
  This is a replay-determinism bug in a multi-choice resolution, unrelated to the
  re-materialization field reset.
- **Seed 19 — state divergence surfacing as an illegal choice.** The client sends
  choice index 2 when the server offers only 2 options (indices 0–1), at a
  `Fireball` cast on turn 24. The shadow's state diverged enough to change a
  controller decision — the broader symptom the strict checks exist to catch.

### Reading of the prize result

The per-choice strict check (`SRV_P1_RECV ... client_hash == server_hash`) held on
every converging seed with the counter **included**, through to game end. The
end-of-game `action_count` gap I saw on seed 5 (server 1283 vs client 1288) is a
post-game-cleanup artifact that does not gate the per-choice equality. So
**`action_count` is genuinely a consensus value where the state converges** — the
netarch consensus-undo-log claim holds for the converging seeds. The prize is
blocked solely by the non-converging seeds.

---

## Completeness analysis (team-lead's #1 deliverable)

### The view hash only checks 3 per-card battlefield fields

`compute_view_hash` (`state_hash.rs` ~L456-470) folds, per battlefield card:
`card_id` (membership), `is_tapped`, `controller` — and nothing else per-card.
Zone-level fields are life, hand SIZE, library SIZE, graveyard size+ids, stack
size+ids. So **damage, counters, power/toughness bonuses, summoning sickness,
attachments, chosen_color, etc. are NOT in the hash** and cannot *directly* trip a
view-hash desync.

### But re-materialization resets ALL of them

A re-materialized opponent permanent is built by
`process_card_reveal → CardDefinition::instantiate → Card::new(id, name, owner)`
(`reveal_processor.rs` L246-256; `loader/card.rs` L645). `instantiate` copies only
*static definition* fields; **every per-instance dynamic field on `Card` defaults**:
`tapped=false`, `controller=owner`, `damage=0`, `counters=[]`,
`power_bonus/toughness_bonus=0`, `turn_entered_battlefield=None` (summoning
sickness), `attached_to/control_from_aura/control_grant=None`,
`chosen_color/chosen_player=None`, `regeneration_shields=0`, `x_paid=0`, …

slot03's fix (`reconstruct_tapped_states`) restores **only `tapped`**. The
honest flag in slot03's summary is accurate — and incomplete in three ways:

1. **`controller` is the uncovered twin (HASHED).** `controller` is in the view
   hash, `ChangeController` IS undo-logged (`undo.rs` L1247, with old/new), yet
   nothing reconstructs it for a re-materialized permanent — it silently reverts
   to `owner`. This is the *exact* analog of the tapped bug in the *other* hashed
   field. **The robots deck has no control-change cards (no Control Magic / Old
   Man of the Sea / Steal Artifact), so the green robots sweep cannot exercise
   it.** Any deck with a control-changer would expose a latent fatal on rewind,
   with or without `action_count`. → **CONCERN / file follow-up.**

2. **`reconstruct_tapped_states` is itself incomplete — 3 tap-state writes do
   NOT log `TapCard`,** so reconstruct (which replays `TapCard` entries) misses
   them and defaults the card untapped:
   - `state.rs:1487-1493` — **global ETB-tapped replacement** (Kismet, Loxodon
     Gatekeeper, Frozen Aether, Orb of Dreams): writes `card.tapped = true` with
     no `TapCard` log. (The *self*-replacement path just above, L1446-1458, was
     explicitly fixed to log it under mtg-ba6uq #1; the global path was missed —
     this is also a latent plain-undo bug.)
   - `state.rs:3563-3567` — "returns to the battlefield tapped": `card.tapped =
     true`, no log.
   - `actions/mod.rs:9256-9263` — `Cost::Untap` (untap-as-a-cost): `card.untap()`,
     no log.
   None are in the robots deck → again unexercised by the sweep. The verified-OK
   logged sites are `tap_permanent`/`untap_permanent` (`state.rs:490/720`), the
   mana-ability tap (`actions/mod.rs:8718`), and the ETB self-replacement.

3. **Decision-determinism gap (beyond the hash).** A re-materialized opponent
   permanent with wrong `damage`/`counters`/`power_bonus`/summoning-sickness/
   attachments/`chosen_color` is read by the *info-independent local controller*
   when it makes its own decisions. Wrong shadow facts → a different decision →
   desync that surfaces as a bad choice index (precisely the seed-19 symptom
   class). These fields are invisible to the view hash but can still cause a
   fatal. The reveal carries no per-instance state, so none are reconstructed.

### Verified-correct parts of the fix

- **Truncation ordering is sound.** `rewind_to_turn_start` pops the undo log to
  turn-start R *before* `unwind_state_sync_to` runs (`fancy_tui.rs` L1769-1792),
  so `reconstruct_tapped_states` reads only `TapCard` entries ≤ R. Last-write-wins
  per card is correct; covers multiple tapped permanents and tapped-then-untapped.
- **Non-mana taps are covered.** All `TapCard`-logging sites log regardless of tap
  *reason*, so attack-taps (Mishra's Factory) and ability-taps (Strip Mine) are
  reconstructed correctly — and those cards ARE in the robots deck (exercised by
  the passing seeds).
- **The reorder-cursor-to-R fix is sound.** `LibraryReorder { player, new_order:
  Vec<CardId> }` carries a **full wholesale** order; apply does a blind
  `library.cards = new_order` (`client.rs` L1644-1646) with no membership filter.
  So (a) re-applying a stale pre-R reorder genuinely re-adds a departed card (the
  seed-5 phantom, size N+1), and (b) any reorder > R is a complete
  server-authoritative snapshot that does not depend on a skipped pre-R reorder.
  Resetting the reorder cursor to R while the reveal cursor stays 0 is correct.
  Library order is not hashed (only size), so order edge cases are moot.
- **`library_ids` diagnostic** is hash-neutral (debug-only) and correct.

---

## MTG-rules review

**N/A (with justification).** Both fixes touch only the **client shadow** used for
desync detection and info-independent decisions; neither changes
server-authoritative game behavior. `reconstruct_tapped_states` makes the shadow's
`tapped` match the server; the reorder-cursor change makes shadow library
membership match the server. No CR-governed effect, reveal ordering, information
hiding, or decision authority changes. No observable gameplay change. (Should the
fix later be extended to reconstruct `controller`/counters/etc., re-confirm — those
also only make the shadow match the server, so still expected N/A.)

---

## Recommendation

- **Do NOT** apply `eb8f938e` (re-include `action_count`) / merge the prize now.
  Blocker: robots seeds **7, 11, 19** fatally desync independent of `action_count`.
- slot03's **tapped + reorder fixes are good** and may merge on their own merit
  (they fix seeds 2/5, regress nothing) — but file the completeness gaps so they
  are not mistaken for a complete rewind-faithfulness fix:
  1. **Balance (and multi-choice resolution) rewind/replay reorders sub-steps**
     (seeds 7, 11) — mtg-677 class; the primary prize blocker.
  2. **Seed-19 state divergence → invalid choice at Fireball** (turn 24).
  3. **`controller` not reconstructed on re-materialization** (hashed; untested by
     robots deck).
  4. **3 unlogged tap-state sites** (global ETB-tapped, returns-tapped, Cost::Untap)
     defeat `reconstruct_tapped_states`; ideally route all tap-state mutations
     through a single `TapCard`-logging chokepoint (DRY) so reconstruct is complete
     by construction.
  5. **Other per-instance fields** (damage/counters/P-T/summoning-sickness/
     attachments/chosen_color) reset on re-materialization can diverge a controller
     decision; the principled fix is the reveal-actionlog unification (drive
     per-instance state through the action_count-keyed log) rather than per-field
     reconstruction band-aids.
- The action_count change is **pre-approved by evidence** once 1–2 are fixed:
  zero false positives across 7 converging seeds with the counter on.
