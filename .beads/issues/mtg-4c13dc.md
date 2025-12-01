---
title: 'Bug: Peter Porker (Spider-Ham) incorrectly disappears when Food token is sacrificed'
status: closed
priority: 2
issue_type: bug
created_at: 2025-11-30T01:37:46.253851447+00:00
updated_at: 2025-12-01T11:29:50.961523820+00:00
---

# Description

## Status: RESOLVED (Not a Bug / Combat Behavior is Correct)

**Investigation result (2025-12-01_#1057(6d87c69)):**

After thorough investigation with SBA debug logging, the reported behavior is **correct MTG rules**:

### What Happens

1. Player 1's Peter Porker attacks
2. Player 2 blocks with their Peter Porker
3. Both are 2/2 creatures dealing 2 damage to each other
4. Both die from lethal damage = **correct MTG behavior**

### Evidence

From game log:
```
Combat: Spider-Ham, Peter Porker (28) (2 damage) ↔ Spider-Ham, Peter Porker (6) (2 damage)
[DEBUG zone] Moving card Spider-Ham, Peter Porker (id=6) from Battlefield to Graveyard
[DEBUG zone] Moving card Spider-Ham, Peter Porker (id=28) from Battlefield to Graveyard
```

SBA check confirmed correct stats before death:
```
[DEBUG sba] SBA check: Spider-Ham, Peter Porker (id=6) damage=0 toughness=2 has_lethal=false
```

The creature is healthy before combat, takes lethal damage during combat, then dies correctly.

### Original Confusion

The original bug report may have confused:
- Combat damage killing creatures (correct)
- With incorrect trigger behavior (separate issue, now fixed)

### Related Fix

During investigation, discovered and fixed a **different bug**: ETB triggers firing incorrectly for all matching cards instead of just the card that entered. See commit for fix.

---
Closed 2025-12-01_#1057(6d87c69) - behavior is correct
