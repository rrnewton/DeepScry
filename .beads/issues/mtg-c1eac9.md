---
title: 'Triskelion: ETB counter keyword not parsed, enters without +1/+1 counters'
status: closed
priority: 3
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:28:49.265104819+00:00
updated_at: 2026-05-12T13:57:36.800003787+00:00
closed_at: 2026-05-12T13:57:36.800003707+00:00
---

# Description

## Context
- Date: 2026-04-03
- Decks: decks/old_school/01_rogue_rogerbrand.dck vs decks/old_school2/the_deck_classic.dck
- Mode: heuristic-vs-heuristic, Seed: 3

## Steps to Reproduce
1. Run: ./target/release/mtg tui --seed 3 --p1 heuristic --p2 heuristic -v 2 decks/old_school/01_rogue_rogerbrand.dck decks/old_school2/the_deck_classic.dck
2. Cast Triskelion
3. Observe it enters as a 1/1 instead of 4/4

## Expected Behavior
Triskelion enters the battlefield with three +1/+1 counters (should be 4/4).

## Actual Behavior
Engine logs: 'Triskelion (55) enters the battlefield as a 1/1 creature'
The K:ETB:Counter<P1P1/3> keyword is not recognized (Warning: Unknown parameterized keyword 'ETB').

## Rules Notes
- Triskelion has 'Triskelion enters the battlefield with three +1/+1 counters on it.'
- The cardsfolder format uses K:ETB:Counter<P1P1/3> for this
- The engine's keyword parser does not handle the ETB:Counter parameterized keyword

## Evidence
- Command: ./target/release/mtg tui --seed 3 --p1 heuristic --p2 heuristic -v 2 decks/old_school/01_rogue_rogerbrand.dck decks/old_school2/the_deck_classic.dck
- Log: debug/fuzz_results/heur_seed3_01_rogue_rogerbrand_vs_the_deck_classic.log
