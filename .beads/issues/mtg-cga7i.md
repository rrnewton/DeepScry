---
title: Implement Airbend mechanic
status: open
priority: 2
issue_type: task
created_at: 2026-01-02T12:51:29.238830422+00:00
updated_at: 2026-01-02T13:06:17.899146583+00:00
---

# Description

## Description

Implement the Airbend mechanic for Avatar set support.

## Mechanic Definition

Format: `DB$ Airbend | ValidTgts$ Creature` (or other permanent types)

Effect: Exile target. While exiled, its owner may cast it for {2} rather than its mana cost.

Reference: MTG Comprehensive Rules 701.65b

## Cards Requiring Airbend

- Aang, the Last Airbender: ETB trigger airbends nonland permanent
- Monk Gyatso: Triggered on targeting other creatures  
- Glider Staff: ETB airbend creature
- Airbender Ascension: ETB airbend creature

## Implementation Status (2026-01-02_#1439)

### 1. PersistentEffect Infrastructure - COMPLETED

Unlike Java Forge which stores persistent effects as virtual cards in the command zone, 
we use dedicated typed storage. This is cleaner and avoids conflating game zones with 
implementation details.

Implemented:
- [x] PersistentEffectStore in core/persistent_effect.rs
- [x] PersistentEffectKind::MayPlayFromExile variant
- [x] CleanupCondition enum for automatic effect cleanup
- [x] Storage in GameState.persistent_effects
- [x] Methods: add, remove, find_may_play_from_exile, find_effects_to_cleanup_*

### 2. Airbend Effect - COMPLETED

- [x] Parse DB$ Airbend via ApiType::Airbend in ability_parser.rs
- [x] Effect::Airbend variant in effects.rs
- [x] Effect converter in effect_converter.rs
- [x] Targeting logic in targeting.rs (creatures by default, nonland if targets_any)
- [x] Execute in actions/mod.rs: exile target + create MayPlayFromExile effect
- [x] Logging in game_loop/logging.rs

### 3. MayPlay Alternative Cost from Exile - TODO

Still needed:
- [ ] When determining legal actions, check persistent_effects for MayPlayFromExile
- [ ] Allow player to select exiled cards with MayPlay permission
- [ ] When casting, use alternative cost instead of mana cost
- [ ] Skip targeting/mode selection for the cast (already resolved)

### 4. Cleanup Triggers - TODO

Still needed:
- [ ] Hook into zone change events to cleanup effects
- [ ] Hook into spell resolution to cleanup effects when card is cast
- [ ] Call find_effects_to_cleanup_on_zone_change and remove_many appropriately

## Java Reference

See: forge-java/forge-game/src/main/java/forge/game/ability/effects/AirbendEffect.java

Key differences from our implementation:
- Java uses command zone for effect storage (we use dedicated PersistentEffect storage)
- Java uses "remembered" cards pattern (we use explicit typed tracking)

## Priority

High - Avatar decks are NOT playable without this mechanic.
