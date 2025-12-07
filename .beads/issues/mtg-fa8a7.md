---
title: 'Browser TUI: Player gets stuck in pass-only loop'
status: open
priority: 2
issue_type: task
created_at: 2025-12-07T22:28:06.138714844+00:00
updated_at: 2025-12-07T22:28:06.138714844+00:00
---

# Description

## Issue

When playing in the browser TUI (fancy.html) with the Human controller, the player
eventually gets stuck in a state where the only available option is "Pass (do nothing)".

## Symptoms

- User reports: "the very first land I play does NOT appear" (fixed in 478a4aa)
- After some gameplay, user can "only play pass repeatedly"

## Possible Causes

1. **Legitimate MTG game state**: The opponent (P2) might be taking their turn and P1
   just needs to pass priority through all phases. This would feel like being "stuck"
   but is correct behavior.

2. **Rewind/replay corruption on turn boundaries**: The rewind pattern looks for
   ChangeTurn actions. On turn transitions there might be edge cases:
   - Turn 1: We now add a synthetic ChangeTurn marker (fixed in 478a4aa)
   - Turn 2+: Real ChangeTurn is logged in advance_to_next_turn()
   - Possible issue: If the turn transition happens mid-replay, the stopping point
     might be wrong

3. **Empty available actions**: The choose_spell_ability_to_play receives an empty
   available slice if:
   - No lands in hand or already played one this turn
   - No castable spells (not enough mana, wrong phase)
   - No activatable abilities
   In this case only "Pass (do nothing)" is shown, which is correct.

## Current State

The Playwright test (test_human_input.js) passes and shows cards appearing on
battlefield:
- Actions that added cards to battlefield: 2
- Final your battlefield count: 4

## Investigation Needed

1. Determine WHEN exactly the pass-only loop starts:
   - After how many turns?
   - At what phase?
   - During P1's turn or when waiting for P2?

2. Check if P2's Zero controller is properly passing or getting stuck

3. Add console logging to track:
   - Turn number and active player at each choice point
   - Size of available actions array
   - Whether we are in rewind/replay mode

## Related

- Fixed: Turn 1 land disappearance bug (478a4aa) - added synthetic ChangeTurn marker
- Related: mtg-25 (Interactive TUI controller)
