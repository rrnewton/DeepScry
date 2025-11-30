---
title: 'Bug: Peter Porker (Spider-Ham) incorrectly disappears when Food token is sacrificed'
status: open
priority: 2
issue_type: bug
created_at: 2025-11-30T01:37:46.253851447+00:00
updated_at: 2025-11-30T01:37:46.253851447+00:00
---

# Description

## Bug Report

**Card:** Spider-Ham, Peter Porker (1G, 2/2 Legendary Creature - Spider Boar Hero)
**Deck:** decks/ryan_spiderman_draft.dck

### Expected Behavior

Peter Porker should:
1. Remain on the battlefield after dealing combat damage
2. Remain on the battlefield when you sacrifice the Food token it created
3. Only leave the battlefield via normal means (destruction, exile, bounce, sacrifice, etc.)

The Food token it creates is a separate permanent - sacrificing it should not affect Peter Porker.

### Actual Behavior

Peter Porker disappears (leaves the battlefield) in two incorrect scenarios:

1. **After dealing combat damage:** Peter Porker attacked, dealt 2 damage, then disappeared without any message explaining why
2. **When sacrificing the Food token:** Sacrificing the Food token that Peter Porker created also made Peter Porker disappear

### Root Cause Hypothesis

Possible issues:
1. **Object identity confusion:** The game may be incorrectly treating Peter Porker and the Food token as the same object
2. **Token creation bug:** The Food token creation may be replacing Peter Porker instead of creating a separate permanent
3. **Sacrifice target bug:** Sacrificing the Food may be incorrectly targeting/affecting Peter Porker

### Card Definition Reference

```
Name:Spider-Ham, Peter Porker
ManaCost:1 G
Types:Legendary Creature Spider Boar Hero
PT:2/2
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigToken | TriggerDescription$ When NICKNAME enters, create a Food token.
SVar:TrigToken:DB$ Token | TokenAmount$ 1 | TokenScript$ c_a_food_sac | TokenOwner$ You
S:Mode$ Continuous | Affected$ Spider.Other+YouCtrl,...[other animals] | AddPower$ 1 | AddToughness$ 1
```

### Reproduction

1. Play `./target/release/mtg tui --p1=fancy decks/ryan_spiderman_draft.dck decks/julian_spiderman_draft.dck`
2. Play Peter Porker (1G)
3. Attack with Peter Porker - observe it disappears after dealing damage
4. OR: Sacrifice the Food token - observe Peter Porker also disappears

### Impact

**Severity:** High
- Makes the card completely unplayable
- Affects token creation system (mtg-34)
- May indicate broader object identity issues

### Related Issues

- mtg-34: Token creation (general feature)
- Potentially related to object tracking/identity system

### Next Steps

1. Debug token creation to verify Food token is created as separate permanent
2. Check object identity in EntityStore after token creation
3. Trace what happens during combat damage resolution
4. Trace what happens during Food token sacrifice
5. Add test case for "permanent creates token, permanent persists"
