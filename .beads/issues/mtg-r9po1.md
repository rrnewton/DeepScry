---
title: 'Bug: T:Mode$ DamageDealtOnce + ValidSource$ Card.AttachedBy + TriggerCount$DamageAmount (triggered pseudo-lifelink) unsupported'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T23:35:49.101624331+00:00
updated_at: 2026-05-31T04:00:27.376328970+00:00
---

# Description

PARTIAL (primary case WORKING) 2026-05-30, compat-wave18-triggers.

Spirit Link's triggered pseudo-lifelink now fires for the PRIMARY case (enchanted
creature deals combat damage to a PLAYER). Implemented as GENERAL damage-dealt-
trigger machinery (lifts any "deals damage -> do X" attached permanent):

1. Trigger MODE 'DamageDealtOnce' now parsed (loader/card.rs parse_triggers) ->
   TriggerEvent::DealsCombatDamage; Execute$ SVar chain resolved via
   extract_effects_from_svar.
2. ValidSource$ Card.AttachedBy -> new Trigger.requires_attached_source flag
   (core/effects.rs); check_triggers fires the Aura's trigger only when the event
   source == the trigger card's attached_to (the enchanted creature).
3. TriggerCount$DamageAmount plumbed: DynamicAmount::parse recognizes
   SVar:X:TriggerCount$DamageAmount -> DynamicAmount::DamageDealt;
   extract_effects_from_svar routes dynamic LifeAmount$ through
   gain_life_dynamic_from_params (was silently dropped). New
   TriggerContext.damage_amount + check_triggers_with_damage thread the combat
   damage; resolve_effect_placeholder turns GainLifeDynamic{DamageDealt} into a
   concrete GainLife at fire time. Added a gamelog line to the fixed
   Effect::GainLife arm (closes a log gap).

Game-log evidence (puzzle test_puzzles/spirit_link_aura.pzl):
  Savannah Lions (6) deals 2 damage to Player 2 (life: 18)
  Player 1 gains 2 life (life: 12)
  ... second attack ...
  Player 1 gains 2 life (life: 14)
P0 (Spirit Link controller) goes 10 -> 14 over two 2-damage attacks.

Reproducer:
```sh
cargo test --release --features network --test puzzle_e2e test_spirit_link_aura_targeting -- --nocapture
```
Expected: two "Player 1 gains 2 life" lines; P0 life 10 -> 14.

REMAINING GAP (why PARTIAL, not WORKING): lifegain does NOT fire when the
enchanted creature deals combat damage to a CREATURE (blocker/attacker), only to
a player. This is a general pre-existing engine limitation — the combat firing
site only fires DealsCombatDamage triggers for damage-to-player
(creatures_that_dealt_player_damage), not for the damage_dealt_by_creature map
that real Lifelink uses. Affects all DamageDone/DamageDealtOnce triggers.
Filed as: mtg-m43mc. Non-combat damage (enchanted pinger) is also not yet wired.

Tests:
- Parser-shape unit: test_card_compat_spirit_link (effects.rs) — asserts
  DealsCombatDamage trigger present, !trigger_self_only,
  requires_attached_source, GainLifeDynamic{DamageDealt} effect.
- E2E: test_spirit_link_aura_targeting (puzzle_e2e.rs) — asserts P0 life
  increases + "Player 1 gains N life" log.

EFFECT_SUPPORT.md: DamageDealtOnce ... TriggerCount$DamageAmount row -> WORKING
(primary case).

MTG Rules Review: PASS (commit message). CR 119.3, CR 603.2, CR 608.2.

CARD STATUS: PARTIAL — triggered lifelink fires on combat damage to a player;
creature-combat-damage + non-combat damage deferred to mtg-m43mc.
