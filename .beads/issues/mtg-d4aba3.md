---
title: Creatures with summoning sickness can activate tap abilities
status: closed
priority: 2
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:28:33.462245029+00:00
updated_at: 2026-05-12T13:57:42.269113852+00:00
closed_at: 2026-05-12T13:57:42.269113772+00:00
---

# Description

## Context
- Date: 2026-04-03
- Decks: decks/fuzz_white_aggro.dck vs decks/fuzz_black_control.dck
- Mode: heuristic-vs-heuristic, Seeds: 6, 7

## Steps to Reproduce
1. Run: ./target/release/mtg tui --seed 7 --p1 heuristic --p2 heuristic -v 2 decks/fuzz_white_aggro.dck decks/fuzz_black_control.dck
2. Turn 6: Royal Assassin enters the battlefield
3. Same turn 6: Royal Assassin activates {T} ability to destroy Tundra Wolves

## Expected Behavior
Creatures cannot activate abilities with {T} in the cost the same turn they enter (CR 302.6), unless they have haste.

## Actual Behavior
Royal Assassin activates its tap ability on the same turn it entered. Reproduced in seeds 6 and 7.

Log excerpt (seed 7):
  Royal Assassin (111) enters the battlefield as a 1/1 creature
  [passes through to Declare Attackers]
  Royal Assassin activates ability: {T}: Destroy target tapped creature.

## Rules Notes
- CR 302.6: A creature's activated ability with the tap symbol in its activation cost can't be activated unless the creature has been under its controller's control continuously since their most recent turn began.
- The engine does not track when creatures entered or enforce summoning sickness for activated abilities.

## Evidence
- Command: ./target/release/mtg tui --seed 7 --p1 heuristic --p2 heuristic -v 2 decks/fuzz_white_aggro.dck decks/fuzz_black_control.dck
- Log: debug/fuzz_results/heur_seed7_fuzz_white_aggro_vs_fuzz_black_control.log
