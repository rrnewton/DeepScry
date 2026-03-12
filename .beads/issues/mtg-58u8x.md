---
title: Implement Mode$ DamageDone trigger parsing (affects 1000+ cards)
status: closed
priority: 3
issue_type: task
created_at: 2026-03-12T01:06:10.435970050+00:00
updated_at: 2026-03-12T01:28:20.328712735+00:00
---

# Description

## Summary

✅ COMPLETED: The card loader now parses `Mode$ DamageDone` triggers.

## Implementation (2026-03-12_#1913(2f6c587))

Added parsing support for Mode$ DamageDone triggers in `card.rs`:
- ValidSource$: Card.Self (self-only) vs Creature.YouCtrl (any creature)
- ValidTarget$: Player/Opponent (player damage) vs Creature (creature damage)
- CombatDamage$ True: combat-only triggers
- OptionalDecider$ You: optional 'you may' triggers

Maps to existing TriggerEvent::DealsCombatDamage.

## Cards Affected

1092 cards with Mode$ DamageDone triggers now parse correctly including:
- Hypnotic Specter: discard on damage to opponent
- Markov Blademaster: +1/+1 counter on combat damage
- Mask of Memory: draw/discard on combat damage
- Marisi, Breaker of the Coil: goad on combat damage

## Tests

- test_hypnotic_specter_from_cardsfolder - verifies trigger parsing
- test_parse_damage_done_trigger - non-combat, self-only
- test_parse_combat_damage_done_trigger - combat-only
- test_parse_optional_damage_done_trigger - optional triggers
