---
title: Modal spell mode selection should validate target availability
status: closed
priority: 3
issue_type: task
labels:
- modal-spell
created_at: 2026-01-03T20:29:02.631192217+00:00
updated_at: 2026-01-04T16:24:57.045364809+00:00
---

# Description

## Summary

When casting a modal spell like Heartless Act, the AI controller (and potentially the fixed controller) automatically chooses mode 0 without checking if there are valid targets for that mode.

## Resolution

Fixed by implementing mode validation in priority.rs that filters modes based on valid targets before presenting them to the controller:

1. Added `has_counters()` method to Card struct
2. Enhanced `TargetRestriction` to support `!HasCounters` modifier (for 'creature with no counters')
3. Added `get_valid_modes_for_spell()` function to targeting.rs
4. Updated mode selection in priority.rs to filter modes by target availability

Now when casting Heartless Act against a creature with counters, mode 1 ('Destroy target creature with no counters') is automatically filtered out, leaving only mode 2 ('Remove up to three counters') which has valid targets.

## E2E Test

Added `test_modal_spell_mode_validation_heartless_act` to puzzle_e2e.rs that verifies:
- Heartless Act is cast successfully
- Mode 2 (RemoveCounter) is chosen automatically
- Grizzly Bears (with 5 +1/+1 counters) is NOT destroyed

## Verified Behavior

```
Player 1 casts Heartless Act (3) (putting on stack)
Player 1 chooses mode: Remove up to three counters from target creature.
Heartless Act (3) resolves
```

## Files Changed

- `mtg-engine/src/core/card.rs` - Added has_counters() method
- `mtg-engine/src/core/effects.rs` - Enhanced TargetRestriction with requires_no_counters
- `mtg-engine/src/game/actions/targeting.rs` - Added get_valid_modes_for_spell()
- `mtg-engine/src/game/game_loop/priority.rs` - Updated mode selection to filter modes
- `mtg-engine/tests/puzzle_e2e.rs` - Added e2e test
