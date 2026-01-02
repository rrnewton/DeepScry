---
title: Thriving Grove doesn't enter tapped or prompt for color choice
status: closed
priority: 2
issue_type: bug
created_at: 2026-01-02T20:35:35.777706122+00:00
updated_at: 2026-01-02T22:26:51.799438042+00:00
closed_at: 2026-01-02T22:26:51.799437982+00:00
---

# Description

## Bug Summary

Thriving Grove and likely other Thriving lands don't properly implement their ETB abilities:
1. Should enter tapped - but it doesn't
2. Should prompt to choose a color when entering - but it doesn't

## Reproducer

```bash
timeout 30 target/release/mtg tui decks/booster_draft/avatar/gabriel_avatar_draft.dck \
  --p1=heuristic --p2=heuristic --seed=42 --log-tail=200 2>&1 | \
  grep -B2 -A2 "plays Thriving Grove"
```

## Observed Behavior

```
<Choice> Player2 chose 2 - play Thriving Grove
Player2 plays Thriving Grove (80)
<Choice> Player2 chose 0 - cast Ostrich-Horse
Player2 casts Ostrich-Horse (60) (putting on stack)
```

Player2 immediately casts Ostrich-Horse using Thriving Grove's mana on the same turn it entered. This should not be possible because:
1. Thriving Grove should enter tapped
2. No color choice prompt appeared

## Expected Behavior

1. Thriving Grove should enter tapped (can't tap for mana the turn it enters)
2. When it enters, player should be prompted to choose a color other than green
3. Later turns, it can tap for {G} or one mana of the chosen color

## Card Definition (from forge-java)

```
K:ETBReplacement:Other:ChooseColor
SVar:ChooseColor:DB$ ChooseColor | Defined$ You | Exclude$ green | AILogic$ MostProminentInComputerDeck
R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplacementResult$ Updated | ReplaceWith$ ETBTapped
A:AB$ Mana | Cost$ T | Produced$ Combo G Chosen
```

## Affected Cards

- Thriving Grove
- Likely all Thriving lands (Thriving Bluff, Thriving Heath, Thriving Isle, Thriving Moor)

## Root Cause (Suspected)

The ETBReplacement and ChooseColor abilities may not be implemented in the Rust engine.
