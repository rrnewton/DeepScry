---
title: Balance creature equalize logged multiple times
status: open
priority: 4
issue_type: bug
created_at: 2026-01-20T10:19:52.636092762+00:00
updated_at: 2026-01-20T10:19:52.636092762+00:00
---

# Description

## Bug Description

When Balance resolves, the 'Creature equalize to N' message is logged 3 times instead of once.

## Evidence

From game log:
```
[GAMELOG Turn11 M1] Balance: Creature equalize to 0
[GAMELOG Turn11 M1] Random2 sacrifices Hypnotic Specter to Balance
[GAMELOG Turn11 M1] Random2 sacrifices Hypnotic Specter to Balance
[GAMELOG Turn11 M1] Random2 sacrifices Sengir Vampire to Balance
[GAMELOG Turn11 M1] Balance: Hand sizes equalize to 0
[GAMELOG Turn11 M1] Balance: Creature equalize to 0
[GAMELOG Turn11 M1] Balance: Creature equalize to 0
```

## Impact

Minor - cosmetic logging issue. No gameplay impact.

## Fix

Ensure the equalize message is only logged once per category (Land, Hand, Creature).
