---
title: 'Fancy TUI: Display actual combat state in Combat tab'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-04T14:52:36.398376960+00:00
updated_at: 2025-11-04T14:52:42.779968631+00:00
---

# Description

Display actual combat state in the Combat tab instead of always showing "(No combat)".

## Implementation (completed in commit 84b9aa6)

1. Added GameStateView::combat() method to expose CombatState
2. Added get_player_name_by_id() helper for player name lookups
3. Updated draw_combat_view() to display attackers and blockers
4. Color-coded with yellow for attackers, cyan for blockers
5. Shows blocked/unblocked status (green/red)
6. Arrow indicators: → attackers, ← blockers

## Visual Design

Shows attackers and blockers with clear status indicators:
- Lists all attacking creatures with defending player
- Shows (blocked by N) or (unblocked) for each attacker
- Lists all blocking creatures with which attacker(s) they block
- Falls back to "(No combat)" when combat is not active

## Benefits

- See combat state at a glance without checking logs
- Clear visual indication of attack/block relationships
- Helps with combat decision-making
- Matches Java Forge combat panel functionality

Part of: mtg-121
