---
title: 'Combat: Attack action not available during Declare Attackers phase'
status: open
priority: 2
issue_type: bug
created_at: 2026-01-02T20:05:52.082486277+00:00
updated_at: 2026-01-02T20:05:52.082486277+00:00
---

# Description

## Bug Summary

During the Declare Attackers phase, the game correctly shows 'Available creatures' that can attack, but the 'Player1 available actions' list does not include any 'attack' options. This makes it impossible to declare attackers in agentplay mode.

## Reproducer

```bash
./agentplay/start_game.sh decks/booster_draft/avatar/ryan_avatar_draft.dck
./agentplay/continue_game.sh 1    # play Mountain
./agentplay/continue_game.sh 2    # play Swamp
./agentplay/continue_game.sh 4    # cast Zhao, the Moon Slayer
./agentplay/continue_game.sh 0    # pass x5 to get to combat
./agentplay/continue_game.sh 0
./agentplay/continue_game.sh 1    # play Mountain (Turn 5)
./agentplay/continue_game.sh 0    # pass
./agentplay/continue_game.sh 0    # should be at declare attackers
./agentplay/continue_game.sh "attack zhao"  # FAILS
```

## Observed Behavior

The output shows:
```
--- Declare Attackers (Player1) ---
Available creatures (1):
  [0] Zhao, the Moon Slayer 2/2

Player1 available actions:
  [0] pass
  [1] cast Fatal Fissure
  [2] cast Heartless Act
  [3] cast Lightning Strike
```

Notice that:
1. The game correctly shows Zhao as an available creature to attack with
2. But the available actions only show pass and cast spells - NO attack options

Trying "attack zhao" gives:
```
Error: InvalidAction("Controller error: Command 'attack zhao' did not match any available action. Available actions: [\"cast Fatal Fissure\", \"cast Heartless Act\", \"cast Lightning Strike\"]")
```

## Expected Behavior

The available actions during Declare Attackers should include:
```
  [0] pass
  [1] attack Zhao, the Moon Slayer
  [2] cast Fatal Fissure
  ...
```

Or similar format that allows declaring attackers via the fixed input controller.

## Root Cause (Suspected)

The code that generates available actions for the Declare Attackers phase may not be including attack options in the menu. The "Available creatures" display appears to be informational only, not actually providing selectable actions.

## Notes

This also affects the display ordering bug filed in mtg-p9svf where "Available actions" appear BEFORE "--- Draw Step ---" - these display issues may be related.
