---
title: Combat damage logged twice with different formats
status: open
priority: 4
issue_type: bug
created_at: 2026-01-20T10:19:33.658210267+00:00
updated_at: 2026-01-20T10:19:33.658210267+00:00
---

# Description

## Bug Description

Combat damage events are logged twice - once as 'X deals N damage' and once as 'Player takes N damage'.

## Evidence

From game log:
```
[GAMELOG Turn6 CD] Royal Assassin (120) deals 1 damage to Random1 (life: 19)
[GAMELOG Turn6 CD] Random1 takes 1 damage (life: 19)

[GAMELOG Turn10 CD] Sengir Vampire (114) deals 4 damage to Random1 (life: 15)
[GAMELOG Turn10 CD] Random1 takes 4 damage (life: 15)
```

## Impact

- Minor - cosmetic logging issue only
- Makes log analysis more verbose
- No gameplay impact

## Fix

Consolidate the damage logging to use a single format, or remove redundant message.
