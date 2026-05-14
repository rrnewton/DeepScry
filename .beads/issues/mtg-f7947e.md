---
title: 'Forest is treated as creature combatant: ''Forest (79) deals 3 damage to Fire Sages'' is illegal under MTG 302.1'
status: closed
priority: 3
issue_type: task
created_at: 2026-05-14T15:08:02.104920126+00:00
updated_at: 2026-05-14T15:08:21.277080040+00:00
---

# Description

## Resolution: Not a bug

Investigated further — Forest (79) was animated to 3/3 on Turn 18 by Cracked Earth Technique:

```
[GAMELOG Turn18 M1] Cracked Earth Technique (69) causes Gabriel to gain 3 life - life: 23 => 26
[GAMELOG Turn18 DA] Gabriel declares Forest (79) (3/3) as attacker
```

The land becoming a creature persists for the rest of the game and was correctly used as both attacker and blocker on later turns. The Turn 24 'Forest (79) deals 3 damage' event is intentional gameplay from a still-animated land. Closing as not-a-bug.

Original (incorrect) report retained below for context:

[old text omitted]
