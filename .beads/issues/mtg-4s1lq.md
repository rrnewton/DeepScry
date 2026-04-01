---
title: Commander format support
status: open
priority: 1
issue_type: feature
created_at: 2026-04-01T02:43:55.841215877+00:00
updated_at: 2026-04-01T02:43:55.841215877+00:00
---

# Description

## Commander Format Support

Tracking issue for implementing Commander (EDH) format support in the MTG engine.

## Commander Rules (MTG CR 903)
- 100-card singleton decks (including commander)
- Designated legendary creature (or planeswalker with "can be your commander") as commander
- Commander starts in the command zone
- Starting life: 40
- Commander can be cast from the command zone
- Commander tax: costs {2} more for each previous cast from command zone
- When commander would go to graveyard or exile, owner may put it in command zone instead
- Commander damage: 21+ combat damage from a single commander = lose
- Color identity: deck cards must match commander's color identity

## Implementation Plan

### Phase 1: Core Infrastructure
- [x] Create deck file with [Commander] section support
- [ ] Add command zone to PlayerZones
- [ ] Add commander tracking to Player (commander_id, commander_tax, commander_damage)
- [ ] Add commander format flag to GameState
- [ ] Support 40 starting life
- [ ] Parse [Commander] section in deck loader

### Phase 2: Commander Mechanics
- [ ] Cast commander from command zone
- [ ] Commander tax calculation and payment
- [ ] Zone-change replacement effect (graveyard/exile -> command zone choice)
- [ ] Commander damage tracking and loss condition

### Phase 3: TUI and Display
- [ ] Command zone display in TUI (lower-left of battlefield)
- [ ] Commander tax indicator in casting cost display

### Phase 4: Testing
- [ ] Random controller e2e tests
- [ ] Fixed controller targeted play tests
- [ ] Heuristic AI commander awareness
- [ ] Card-by-card mechanics validation for Chandra deck

## Test Deck
- decks/commander/chandra_tokens.dck - Boros token-swarm with Chandra, Torch of Defiance

## Related Issues
- mtg-3: MTG feature completeness
- mtg-4: Gameplay features (TUI)
