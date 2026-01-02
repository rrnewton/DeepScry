---
title: 'Agentplay CLI: Turn sequence and display bugs'
status: open
priority: 2
issue_type: bug
created_at: 2026-01-02T19:36:58.159017626+00:00
updated_at: 2026-01-02T19:36:58.159017626+00:00
---

# Description

## Summary

Multiple bugs found during agentplay testing with ryan_avatar_draft.dck deck.

## Bugs Found

### 1. Hand contents shows pre-draw state
**Severity: Medium**
The hand contents displayed at the start of each turn is snapshotted BEFORE the draw step, so newly drawn cards don't appear in the hand contents list.

Example:
```
Hand: 7 | Library: 33
Hand contents:
  - Card1, Card2, ... (7 cards)
--- Draw Step ---
  Player2 draws Zhao, Ruthless Admiral
```
After draw, hand should have 8 cards but only 7 are listed.

### 2. Available actions shown BEFORE draw step
**Severity: High**
The turn display shows available actions BEFORE the draw step, which is incorrect. MTG turn sequence should be:
1. Untap Step
2. Upkeep Step  
3. Draw Step
4. Main Phase (choices here)

But the output shows:
```
Player1 available actions:
  [0] pass
  [1] cast Fatal Fissure
--- Draw Step ---
  Player1 draws Lightning Strike
```

### 3. Land play option missing on Turn 5
**Severity: High**
On Turn 5, Player1 has a Mountain in hand but "play Mountain" is NOT offered as an available action. This prevents the player from playing their land for the turn.

Hand contents shows Mountain, but available actions are:
- [0] pass
- [1] cast Fatal Fissure
- [2] cast Heartless Act

No land play option!

### 4. Lands show as tapped at turn start
**Severity: Low (display only)**
At the start of a turn, lands are displayed as "(tapped)" from the previous turn, even though they should have untapped during the Untap step. However, spells can still be cast, suggesting the lands ARE untapped internally.

## Reproducer

```bash
./agentplay/start_game.sh decks/booster_draft/avatar/ryan_avatar_draft.dck
./agentplay/continue_game.sh "play mountain"
./agentplay/continue_game.sh "play mountain" 
./agentplay/continue_game.sh "play swamp"
./agentplay/continue_game.sh "cast zhao"
./agentplay/continue_game.sh "play swamp"
./agentplay/continue_game.sh "cast fire sages"
## Now on Turn 5 - observe bugs
```

## Root Cause Hypothesis

The snapshot/display timing seems to capture game state at wrong points during the turn sequence. The available actions may be generated from a different game state than what's displayed.
