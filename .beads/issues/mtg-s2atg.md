---
title: Animate Dead may target creatures on battlefield
status: open
priority: 3
issue_type: bug
created_at: 2026-01-20T10:19:43.992772213+00:00
updated_at: 2026-01-20T10:19:43.992772213+00:00
---

# Description

## Bug Description

Animate Dead appears to target a creature on the battlefield rather than in the graveyard.

## Evidence

From game log:
```
[GAMELOG Turn13 M2] Sengir Vampire (40) enters the battlefield as a 4/4 creature
[GAMELOG Turn13 M2] Random1 casts Animate Dead (46) (putting on stack)
[GAMELOG Turn13 M2]   → targeting Sengir Vampire (40)
...
[GAMELOG Turn13 M2] Animate Dead enchants Sengir Vampire
```

Sengir Vampire had just entered the battlefield, but Animate Dead targeted it. By MTG rules, Animate Dead can only target creature cards in graveyards.

## MTG Rules

CR 303.4f: Animate Dead can only target a creature card in a graveyard when it's cast.

## Needs Investigation

This could be:
1. A targeting validation bug (allowing illegal targets)
2. A logging bug (showing wrong target)
3. Correct behavior if there was another Sengir Vampire in graveyard with same ID

## Impact

If this is a real gameplay bug, it allows illegal plays that could significantly affect game outcomes.
