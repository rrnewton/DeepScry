---
title: 'Bug: T:Mode$ DamageDealtOnce + ValidSource$ Card.AttachedBy + TriggerCount$DamageAmount (triggered pseudo-lifelink) unsupported'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-30T23:35:49.101624331+00:00
updated_at: 2026-05-31T03:48:21.345119433+00:00
---

# Description

RESOLVED (WORKING) 2026-05-30, compat-wave18-triggers.

Spirit Link's triggered pseudo-lifelink now fires. Implemented as GENERAL
damage-dealt-trigger machinery (lifts other "deals damage -> do X" attached
permanents, not just Spirit Link):

1. Trigger MODE 'DamageDealtOnce' now parsed (loader/card.rs parse_triggers):
   maps to TriggerEvent::DealsCombatDamage (same firing site as DamageDone),
   resolves the Execute$ SVar chain via extract_effects_from_svar.

2. ValidSource$ Card.AttachedBy now resolved: new Trigger.requires_attached_source
   flag (core/effects.rs). check_triggers filters: the trigger fires only when
   the event source == the trigger card's attached_to (the enchanted creature).
   So the Aura's trigger fires for its host, not for every creature.

3. TriggerCount$DamageAmount plumbed: DynamicAmount::parse now recognizes
   SVar:X:TriggerCount$DamageAmount -> DynamicAmount::DamageDealt;
   extract_effects_from_svar routes a dynamic LifeAmount$ through
   gain_life_dynamic_from_params (previously silently dropped). New
   TriggerContext.damage_amount carries the damage; check_triggers_with_damage
   threads it from the combat firing site (combat.rs). resolve_effect_placeholder
   converts GainLifeDynamic{DamageDealt} -> GainLife{controller, damage} at fire
   time. Also added a gamelog line to the fixed Effect::GainLife arm so the gain
   is visible (closes a log gap; LoseLife already logged).

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

Tests:
- Parser-shape unit: test_card_compat_spirit_link (mtg-engine/src/game/actions/tests/effects.rs)
  asserts DealsCombatDamage trigger present, !trigger_self_only,
  requires_attached_source, and GainLifeDynamic{DamageDealt} effect.
- E2E: test_spirit_link_aura_targeting (mtg-engine/tests/puzzle_e2e.rs) now
  asserts P0 life increases and the "Player 1 gains N life" log line.

EFFECT_SUPPORT.md: DamageDealtOnce ... TriggerCount$DamageAmount row -> WORKING.

MTG Rules Review verdict: PASS (see commit message). CR 119.3 (life gain from
damage), CR 603.2 (trigger events), CR 608.2 (resolution). No hidden info, no
controller-layer crossing, deterministic.
