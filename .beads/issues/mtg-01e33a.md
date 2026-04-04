---
title: 'Hymn to Tourach: Caster discards instead of target opponent'
status: open
priority: 2
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:31:45.059648564+00:00
updated_at: 2026-04-03T21:31:45.059648564+00:00
---

# Description

## Context
- Date: 2026-04-03
- Decks: decks/fuzz_black_control.dck vs decks/fuzz_blue_control.dck
- Mode: random-vs-random, Seed: 10

## Steps to Reproduce
1. Run: ./target/release/mtg tui --seed 10 --p1 random --p2 random -v 2 decks/fuzz_black_control.dck decks/fuzz_blue_control.dck
2. Random1 casts Hymn to Tourach
3. Observe that Random1 (the caster) discards 2 cards instead of the opponent

## Expected Behavior
Hymn to Tourach says 'Target opponent discards two cards at random.'
The opponent (Random2) should discard 2 cards at random.

## Actual Behavior
The caster (Random1) discards 2 cards. From log:
  Random1 casts Hymn to Tourach (55) (putting on stack)
  Hymn to Tourach (55) resolves
  Swamp is discarded
  Random1 discards Swamp
  Royal Assassin is discarded
  Random1 discards Royal Assassin
  Hymn to Tourach (55) causes Random1 to discard 2 card(s)

## Rules Notes
- Hymn to Tourach targets an opponent
- The targeted player should be the one discarding, not the caster
- This appears to be a discard effect resolution bug where the effect is applied to the caster instead of the target

## Evidence
- Command: ./target/release/mtg tui --seed 10 --p1 random --p2 random -v 2 decks/fuzz_black_control.dck decks/fuzz_blue_control.dck
- Log: debug/fuzz_results/seed10_fuzz_black_control_vs_fuzz_blue_control.log
