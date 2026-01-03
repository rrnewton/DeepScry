---
title: Modal spell mode selection should validate target availability
status: open
priority: 3
issue_type: task
labels:
- modal-spell
created_at: 2026-01-03T20:29:02.631192217+00:00
updated_at: 2026-01-03T20:29:02.631192217+00:00
---

# Description

## Summary

When casting a modal spell like Heartless Act, the AI controller (and potentially the fixed controller) automatically chooses mode 0 without checking if there are valid targets for that mode.

## Reproduction

1. Load puzzle: `puzzles/heartless_act_remove_counter_e2e.pzl`
2. Cast Heartless Act
3. AI chooses mode 1 ("Destroy target creature with no counters")
4. Grizzly Bears has 5 +1/+1 counters, so it's NOT a valid target for mode 1
5. Spell resolves anyway and tries to destroy "Unknown (0)"

## Expected Behavior

1. Mode 1 should be skipped because there are no valid targets (creatures without counters)
2. Mode 2 ("Remove up to three counters") should be chosen instead since it HAS valid targets
3. Or the player should be prompted only with modes that have valid targets

## Relevant Files

- `mtg-engine/src/game/game_loop/priority.rs` - Mode selection happens here
- `mtg-engine/src/controllers/` - Controller implementations for choose_modes()
- `mtg-engine/src/core/effects.rs` - Effect::ModalChoice definition

## Test Puzzle

File: `puzzles/heartless_act_remove_counter_e2e.pzl`

```pzl
[state]
p1battlefield=Grizzly Bears|Counters:P1P1=5
```

The Grizzly Bears has 5 +1/+1 counters, making mode 1 illegal but mode 2 legal.

## Notes

The puzzle loader correctly applies counters (verified with load_puzzle example).
The issue is in mode selection logic, not counter loading.

This is similar to how targeting validation works for regular spells - modal spells need
their modes filtered by target legality before presenting choices to the player.
