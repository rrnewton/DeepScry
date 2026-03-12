---
title: Implement Mode$ DamageDone trigger parsing (affects 1000+ cards)
status: open
priority: 3
issue_type: task
created_at: 2026-03-12T01:06:10.435970050+00:00
updated_at: 2026-03-12T01:06:10.435970050+00:00
---

# Description

## Summary

The card loader currently doesn't parse `Mode$ DamageDone` triggers, which are used by over 1000 cards including classic cards like Hypnotic Specter.

## Cards affected

Example cards:
- Hypnotic Specter: `T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Opponent | Execute$ TrigDiscard`
- Sengir Vampire: Uses `Mode$ ChangesZone` with `ValidCard$ Creature.DamagedBy` (related but different)
- Many specter cards, saboteur abilities, etc.

## Implementation needed

1. Add `DamageDone` trigger mode parsing in `card.rs` parse_triggers()
2. Map to `TriggerEvent::DealsCombatDamage` (or add a more general `DealsDamage` event)
3. Handle `ValidSource$`, `ValidTarget$` parameters
4. Support `CombatDamage$` filter for combat-only triggers

## Reference

Java Forge trigger handling: `forge-game/src/main/java/forge/game/trigger/TriggerDamageDone.java`

## Test cases

- `test_hypnotic_specter_from_cardsfolder` - currently skips trigger assertion
- `test_sengir_vampire_from_cardsfolder` - requires DamagedBy tracking
