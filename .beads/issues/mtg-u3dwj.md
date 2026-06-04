---
title: 'Network desync: rogerbrand seed=3 HEURISTIC native local-vs-network DETERMINISTIC gamelog divergence (Turn 8 M2; All Hallow''s Eve/Wheel-of-Fortune sequencing)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-04T20:15:24.375081483+00:00
updated_at: 2026-06-04T23:04:50.651426760+00:00
---

# Description

## Summary

NATIVE local-vs-network gamelog DESYNC, DETERMINISTIC, on the rogerbrand mirror
match at seed=3 with the HEURISTIC controller. This is the native (mtg server +
two `mtg connect` clients) expression of the rogerbrand / All Hallow's Eve
family. It is DISTINCT from the already-tracked siblings:
  - mtg-586 (CLOSED): a NONDETERMINISTIC, random-controller HARNESS SEATING RACE
    (whole-game-from-line-1 divergence under load). Fixed; not this.
  - mtg-589 / mtg-609 (open): the WASM-SHADOW expression (P2 state-hash mismatch,
    All Hallow's Eve mass-resurrection applied ahead of server, Turn 13 upkeep).
This issue is the NATIVE, DETERMINISTIC, HEURISTIC-controller path, which to our
knowledge had no pinned native repro before.

## Deterministic repro (integration @8c0d6ac6, 2026-06-04)

Reproduces 3/3 times in isolation under `systemd-run --user --scope`:

    bug_finding/fuzz_determinism_netequiv.sh --invariant equivalence \
      --decks decks/old_school/01_rogue_rogerbrand.dck --pair-mode self \
      --start-seed 3 --seeds 1 --controllers heuristic --keep-logs

Result: local-vs-network [GAMELOG ...] diff = 377 lines (server authoritative).

LOCAL determinism itself HOLDS: two local `mtg tui ... --seed 3 --p1 heuristic
--p2 heuristic` runs are byte-identical (297/297 gamelog lines). So the engine
sim is reproducible; the divergence is purely LOCAL-vs-NETWORK, i.e. the
heuristic controller reaches a DIFFERENT decision when running on the client's
shadow state vs the server's full state. Per docs/NETWORK_ARCHITECTURE.md that
is an information-independence violation (controllers must decide identically on
full vs shadow state) -> a desync, which is always fatal.

random and zero controllers on this same deck/seed PASS in isolation (the
random path is the mtg-586-class load-flaky one, kept out of any gate).

## Divergence locus

The two games agree through Turn 7, then diverge at Turn 8 M2 on Gabriel's
discard/play sequence: LOCAL discards Scrubland + Badlands; NETWORK discards
Animate Dead x2, then plays Badlands and casts Sedge Troll. Downstream the
network game casts Wheel of Fortune (mass draw/discard) and Swords to
Plowshares, while All Hallow's Eve is drawn — i.e. the same mass-effect / forced
discard sequencing cluster as the WASM-shadow mtg-609. The native heuristic path
diverges EARLIER (Turn 8 vs Turn 13) and DETERMINISTICALLY.

## Where it surfaces

This was found by the new comprehensive desync canary
(bug_finding/desync_canary.sh / `make validate-desync-canary`), where rogerbrand is
carried as an explicit KNOWN-RED (XFAIL) leg: run + captured + reported every
time, but NOT gating, because the default validate gate deliberately excludes
the not-deterministically-green rogerbrand equivalence sweep. The canary's GREEN
gate (avatar-cycling, monored-burn, counterspells-stack) is all-green at
@8c0d6ac6.

## Next steps

Root-cause the heuristic shadow-vs-full decision divergence in the All Hallow's
Eve / Wheel of Fortune / forced-discard sequencing on the native client path
(likely shares a root with mtg-609). Eliminate the desync (do NOT paper over).
Once fixed, promote rogerbrand-heuristic into the canary GREEN corpus and update
the baseline note in bug_finding/desync_canary.sh.

Related: mtg-586, mtg-589, mtg-609, mtg-420, mtg-263, mtg-559, mtg-387.

## Full-canary baseline (integration @8c0d6ac6, 2026-06-04, isolated run)

The desync is HEURISTIC-CONTROLLER-WIDE on this deck, not unique to seed 3.
Running the rogerbrand mirror across seeds 1-4 x {heuristic,random,zero}:
  - heuristic seed=1 -> diverged (252-line diff)
  - heuristic seed=2 -> diverged (115-line diff)
  - heuristic seed=3 -> diverged (377-line diff)
  - heuristic seed=4 -> diverged (283-line diff)
  - ALL random + ALL zero combos -> PASS
So 4/4 heuristic seeds diverge deterministically; 8/8 random+zero pass. This is
a heuristic-decision shadow-vs-full divergence, consistent across seeds. The
canary's GREEN gate (avatar 18/18, monored 18/18 incl seed13, counterspells
12/12 incl seed5) was all-green in the same run.
## ROOT CAUSE (2026-06-04, fix-mtg-u3dwj @b886d9b3, evidence-backed)

NOT a controller info-leakage bug. The heuristic controller is CORRECT; it
decides on a DEFICIENT shadow view. This is a CLIENT-SIDE network shadow
reveal-ordering race in the in-resolution choice path.

Exact divergence: Turn 8 M2, Gabriel activates Bazaar of Baghdad ("Draw two
cards, then discard three cards."). Both sides draw the same cards: Scrubland
(106) + Badlands (105). The forced discard-3 then diverges:
  - LOCAL (full state): discards the 2 lands (106,105) + Disenchant — lands-first.
  - NETWORK (shadow decides): keeps both lands, discards Disenchant + 2x Animate
    Dead. Server faithfully applies the client's chosen indices, so server+client
    stay hash-consistent (no hash desync — the game runs to completion); the
    divergence is ONLY vs the local full-state game, which is exactly why only the
    local-vs-network EQUIVALENCE invariant catches it, never the state-hash check.

Mechanism (proven by instrumented run, since reverted):
1. heuristic_controller.rs:6182 `choose_cards_to_discard` ranks lands key=0
   (discard first), creatures by eval, spells key=100 (keep). Line ~6189 builds
   candidates via `hand.iter().filter_map(|id| view.get_card(id))` — it SILENTLY
   DROPS any CardId whose `get_card` returns None.
2. On Gabriel's shadow, at the discard choice the just-drawn cards 105/106 are
   `view.get_card() == None` — NOT instantiated. Instrumentation dump:
       [DBG_DISCARD] id=106 <NOT IN VIEW / get_card=None>
       [DBG_DISCARD] id=105 <NOT IN VIEW / get_card=None>
   so they are dropped from the candidate set; the heuristic then discards the
   3 lowest of the remaining 7 (all spells) instead of the lands.
3. Why None: on the SERVER the draw reveals ARE generated and bundled
   (network/controller.rs `collect_reveals_since_last_choice` collected
   `[(105,387),(106,384)]` for the discard ChoiceRequest) and sent as
   `CardRevealed` messages BEFORE the ChoiceRequest. But on the NATIVE client the
   shadow library holds RESERVED-but-uninstantiated ids; the Card instance is
   only materialized when `network/reveal_processor.rs::process_card_reveal`
   (Draw arm, ~line 116-118 `game.cards.insert`) runs. The shadow GameLoop
   advances through Bazaar's draws and INTO the discard choice BEFORE that apply
   runs. Decisive ordering proof from the client log:
       line 335  [DBG_DISCARD] id=106 <NOT IN VIEW>     <- discard decided here
       line 343  [DBG_INST] instantiate card=106 Scrubland   <- reveal applied 8 lines LATER
   The `sync_callback` (client.rs ~1885, `apply_state_sync_up_to_frontier`)
   applies reveals "up to the current frontier", but the shadow loop reaches the
   in-resolution discard before the frontier/apply covers the draw reveals.
   `NetworkLocalController::choose_cards_to_discard` (local_controller.rs:736)
   only `verify_action_count_sync` (verifies, does NOT block+apply) before
   querying the controller.

This is the SAME draw-then-discard-in-one-resolution window already noted (for a
different symptom — the re-add-to-hand desync) at reveal_processor.rs:151-156.
Bazaar is the trigger because the drawn cards are consumed by a choice in the
SAME resolution, before control returns to the message loop. Latent at the Turn
6/10 Bazaars only because the drawn cards happened not to be selected for discard
there. Heuristic-controller-wide on this deck (4/4 seeds diverge) because the
heuristic's land-first discard reliably WANTS to discard the freshly-drawn lands,
exposing the gap; random/zero pass because they don't preferentially target the
just-drawn cards.

## PROPOSED FIX (precise; NOT yet implemented — deep desync surgery, review-gated)

Principled fix: before the shadow GameLoop serves ANY in-resolution local choice,
block until the state-sync frontier reaches the choice's `server_action_count`
and apply all pending reveals, so just-drawn own cards are instantiated before
the controller queries them. `wait_for_state_sync_frontier(count)` (client.rs:657)
+ `apply_state_sync_up_to_frontier` already exist; the gap is that the local
choice path uses `verify_action_count_sync` (verify-only) instead of a
blocking sync-and-apply. Best done once in the GameLoop pre-choice hook /
`call_pre_choice_hook` (network_choice.rs) so every ChoiceKind benefits (DRY),
not per-method.

RISKS / why review-gated: (a) this is the deterministic-sequential lockstep core
on desync sacred ground; a blocking wait must be bounded to avoid deadlock if a
reveal is never sent. (b) It directly overlaps the ACTIVE netarch consensus-undo-log
rearchitecture (mtg-… slot02), which is restructuring exactly this state-sync /
frontier path — the fix may conflict with or be subsumed by that work. Recommend
team-lead decide whether to fix here or fold into the netarch rework.

Secondary hardening (independent, low-risk): heuristic_controller.rs:6189
`filter_map` should NOT silently drop CardIds the engine put in the discard
candidate `hand`; an unresolvable own-card in a discard list is itself an
invariant violation worth a hard error in shadow/debug builds rather than a
silent skip that masks exactly this class of bug.

Repro (deterministic 3/3): bug_finding/fuzz_determinism_netequiv.sh --invariant
equivalence --decks decks/old_school/01_rogue_rogerbrand.dck --pair-mode self
--start-seed 3 --seeds 1 --controllers heuristic --keep-logs
