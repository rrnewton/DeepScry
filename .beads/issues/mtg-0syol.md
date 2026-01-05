---
title: 'Stateful cards: per-card persistent state system'
status: open
priority: 2
issue_type: task
created_at: 2026-01-05T20:18:34.759068046+00:00
updated_at: 2026-01-05T20:18:34.759068046+00:00
---

# Description

## Summary

Design and implement a system for per-card persistent state that:
1. Survives zone changes (card identity, not zone presence)
2. Integrates with undo infrastructure
3. Supports multiple MTG mechanics requiring card-level state

## Affected Cards (ryan_avatar_draft)

- Zuko, Conflicted: ChoiceRestriction$ ThisGame (track which modal choices used)

## MTG Mechanics Requiring This System

### 1. Modal History (ChoiceRestriction$ ThisGame)
- Track which modes have been used across the game
- Must persist when card changes zones (exile/return)
- Example: Zuko's "choose one that hasn't been chosen"

### 2. "Once Each Turn" Abilities
- Track if a specific ability has been activated this turn
- Reset at beginning of turn or cleanup
- Common pattern: "Activate only once each turn"

### 3. "First Time Each Turn" Triggers
- Track if specific event has occurred this turn for this card
- Example: "The first time a creature enters the battlefield each turn..."

### 4. Sagas (Lore Counters + Chapter Tracking)
- Counter-based but with special trigger timing
- "When this Saga enters and after your draw step, add a lore counter"
- Chapter abilities trigger at specific lore counter values

### 5. Level Up Creatures
- Level counters determine P/T and abilities
- Different tiers of effects at different levels

## Proposed Design

```rust
pub struct CardPersistentState {
    /// Modal choices used (for ChoiceRestriction$ ThisGame)
    pub used_modal_choices: SmallVec<[String; 4]>,
    
    /// Abilities activated this turn (for "once each turn")
    pub abilities_activated_this_turn: SmallVec<[AbilityId; 2]>,
    
    /// Turn-scoped event tracking (reset at turn start)
    pub events_this_turn: SmallVec<[TriggerEvent; 4]>,
}
```

## Undo Integration

The undo log must capture:
1. Changes to persistent state fields
2. Turn-boundary state resets
3. Modal choice selections

## Dependencies

- Existing counter system (card.counters)
- Existing undo infrastructure (undo_log)

## Related Issues

- mtg-ijo2m: SpellCast triggers (different but related trigger infrastructure)
