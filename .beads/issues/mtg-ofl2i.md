---
title: 'Native-vs-WASM engine divergence: WASM create_game_from_database hand-rolls setup (different CardID assignment + shuffle order) breaking same-seed determinism'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T06:32:43.753296370+00:00
updated_at: 2026-05-30T06:33:12.217630203+00:00
---

# Description

## Summary

The mtg-forge-rs engine compiles to BOTH native and WASM and MUST produce the
SAME game for the SAME seed in either target (controllers are information- and
target-independent; see docs/NETWORK_ARCHITECTURE.md). A new native-vs-WASM
equivalence fuzz sweep (scripts/native_wasm_equiv_sweep.py) shows **100%
divergence**: every random-vs-random (seed, deck) combo plays a DIFFERENT game
in WASM than native, despite identical opening hands and identical library
order.

## Reproducer

    ./scripts/native_wasm_equiv_sweep.sh --seeds 3 \
        --decks 'decks/old_school/*.dck,decks/old_school2/*.dck' --max-turns 20

18/18 combos diverged. Divergent gamelogs + per-combo reproducers are written
under the gitignored debug/native_wasm_equiv/. Example (seed=1 ur_burn): both
targets agree on the opening line-up, then at Turn2 native casts Lightning Bolt
in Beginning-of-Combat targeting its own controller while WASM casts it in
Main1 targeting the opponent. Seed=2 ur_burn: Chain Lightning targets P1
(native) vs P2 (wasm).

## Root cause

Two DIVERGENT game-setup implementations (a DRY violation):

* **native** — `mtg-engine/src/loader/game_init.rs` (~line 211):
    1. build per-player Vec<Arc<CardDefinition>> in deck order,
    2. SHUFFLE the card-def lists FIRST (`p1_card_defs.shuffle(&mut rng)`),
    3. THEN assign positional CardIDs 0..N to the *shuffled* order
       (CardID == library position).

* **wasm** — `mtg-engine/src/wasm/fancy_tui.rs::create_game_from_database`
  (~line 3890):
    1. instantiate cards via `next_entity_id()` in DECK ORDER (unshuffled),
       assigning CardIDs as they are added to the library,
    2. THEN `game.shuffle_library(p1_id)` / `shuffle_library(p2_id)`.

Fisher-Yates depends only on length, so both targets shuffle to the SAME
permutation (hence identical opening CARDS and identical post-opening draw
order — both draw Sol Ring on Turn3). But the **CardID -> card mapping is
different** (native ID follows shuffled position; WASM ID follows deck order:
e.g. native Sol Ring=52 vs WASM Sol Ring=16). The RandomController enumerates /
orders its candidate choices (and consumes RNG) in a way that depends on
CardID, so the SAME RNG stream picks a DIFFERENT card / target in each target.

WASM also hand-rolls the opening-hand draw as an INTERLEAVED p1/p2/p1/p2 loop,
whereas the shared `mtg-engine/src/game/hand_setup.rs::setup_opening_hands`
draws all 7 for player 0 then all 7 for player 1. Draws do not consume RNG so
this does not change the cards, but it is the same drift symptom — WASM does
not reuse the shared setup helper.

## Fix direction (NOT done here — harness + bug only)

Unify the two paths so WASM game creation reuses the SAME shuffle-then-assign-
positional-CardID sequence (and `setup_opening_hands`) that native uses, OR
make the RandomController's choice ordering CardID-independent. Either makes the
two compile targets play identical games. Requires an MTG-rules-review per
CLAUDE.md before merge. The CardID-positional scheme in game_init.rs is also
what the network late-binding path relies on, so the WASM single-player path
should converge on it rather than the reverse.

## Relationship to existing issues

Distinct from mtg-sfihb (WASM network-E2E ActionLog push desync in multi-Shivan
combat) and mtg-yulth (heuristic local-vs-network Demonic Tutor search): this is
the RANDOM controller, SINGLE-player (non-network) WASM-vs-native axis, rooted
in game-setup CardID assignment, not network sync or heuristic info-leak.

## Detection harness

scripts/native_wasm_equiv_sweep.py + .sh (seed-range x deck-sample sweep,
exit 1 on any divergence). A bounded leg is wired into `make validate`
(validate-agentplay-step) gated by MTG_EQUIV_REQUIRE_WASM. Once this bug is
fixed the bounded leg flips from "documents the known divergence" to
"regression guard".
