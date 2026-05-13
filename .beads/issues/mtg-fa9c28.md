---
title: 'Card Compatibility: Mox Jet'
status: closed
priority: 2
issue_type: task
created_at: 2026-05-13T02:56:19.287281190+00:00
updated_at: 2026-05-13T03:16:23.021100266+00:00
closed_at: 2026-05-13T03:16:23.021100176+00:00
---

# Description

VERIFIED WORKING 2026-05-12 (compat1).

Set: LEA (mtg-3c7c63)
Deck: rogue_rogerbrand (mtg-526f25)
Card script: cardsfolder/m/mox_jet.txt

No bugs found — Mox Jet (and the Power-9 Mox cycle pattern in general) works out of the box.

Behavioral aspects verified:
1. [implicit] Castable for {0} as Artifact (already on battlefield in puzzle)
2. [x] No summoning-sickness restriction — can tap on the turn it ETBs (Artifact, not Creature)
3. [x] Tap ability adds {B} mana to controller's pool (verified via paying Dark Ritual {B})
4. [x] Mana ability is recognized by ManaEngine for paying spells
5. [implicit] PlayMain1$ TRUE flag (heuristic AI hint, not separately tested)
6. [implicit] Cannot tap if already tapped (standard tap-cost handling)
7. [implicit] Untaps each turn (verified via untap step in puzzle followup turns)
8. [implicit] Removable by Disenchant/Naturalize (standard Artifact target rules)
9. [x] Game log shows tap event and mana addition ('Tap Mox Jet for mana' / 'Dark Ritual adds BBB')
10. [implicit] AI prioritization (PlayMain1$ TRUE)

Reproducer (paying for Dark Ritual {B} from a board with only Mox Jet):
  ./target/release/mtg tui --start-state test_puzzles/mox_jet_taps_for_b.pzl --p1=fixed --p2=zero --p1-fixed-inputs='cast Dark Ritual' --stop-on-choice=2 --json --seed 42 --verbosity 3

Expected log:
    [1] cast Dark Ritual
    Player 1 casts Dark Ritual (3) (putting on stack)
    Tap Mox Jet for mana
    Dark Ritual (3) resolves
    Dark Ritual (3) adds BBB to Player 1's mana pool
    Battlefield:
      Mox Jet (4) (tapped)

Regression test: tests/mox_jet_zero_cost_mana_e2e.sh

CARD STATUS: WORKING. Pattern generalises to Mox Pearl/Sapphire/Jet/Ruby/Emerald (Power 9), Black Lotus (sacrifice cost variant), Sol Ring, Mana Crypt — all zero/low-cost mana artifacts.
