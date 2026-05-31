---
title: 'Bug: DealsCombatDamage triggers only fire on damage-to-player, not damage-to-creature'
status: closed
priority: 3
issue_type: bug
created_at: 2026-05-31T04:00:02.691428461+00:00
updated_at: 2026-05-31T08:12:42.305399547+00:00
---

# Description

FIXED 2026-05-31 (fix-mtg-m43mc-creature-dmg-trigger).

DealsCombatDamage triggers now fire for ALL combat damage a creature deals this
step -- to players AND to creatures/planeswalkers (CR 510.2: combat damage is one
simultaneous event). Previously the firing site in
mtg-engine/src/game/actions/combat.rs iterated only creatures_that_dealt_player_damage,
so a blocked/blocking creature never fired its trigger.

FIX (single shared path; native == WASM == network):
1. Firing site (resolve_combat_damage) now iterates the SAME deterministic
   damage_dealt_by_creature BTreeMap that the Lifelink keyword consumes -- in
   CardId order -- firing DealsCombatDamage once per creature that dealt combat
   damage. Per-creature it computes a CombatDamageBreakdown { total, to_player,
   to_creature } from damage_dealt_by_creature + a summed player-damage map.
2. New structured Trigger.combat_damage_target: CombatDamageTarget {Player,
   Creature, Any} (core/effects.rs), parsed from ValidTarget$ in loader/card.rs
   (replaces the DEAD [any-damage]/[damages-creature] description markers that
   had no consumer). #[serde(default)] = Any; part of the static card definition
   so it round-trips via serde (no new mutable/un-serialized game state).
3. check_triggers split into check_triggers / check_triggers_with_damage(fixed) /
   check_combat_damage_triggers(breakdown), all delegating to one
   check_triggers_inner. The recipient-class gate + per-trigger amount selection
   live in one place (CombatDamageBreakdown::amount_for): a Player-only trigger
   (Hypnotic Specter, Mark of Sakiko) is SKIPPED when only a creature was hit and
   observes the player-damage slice; an Any trigger (Spirit Link) fires on any
   combat damage and observes the total; a Creature trigger observes creature
   damage. Firing order unchanged (deterministic BTreeMap CardId order; no
   HashMap/HashSet introduced).

Determinism: trigger detection keys off the one recorded combat-damage event via
the same shared code path on native/WASM/network; no recipient- or platform-
specific branch; no unordered collections; no new un-serialized side state.

Tests:
- Unit (parser): test_parse_damage_done_trigger / test_parse_combat_damage_done_trigger
  assert combat_damage_target == Player for ValidTarget Opponent/Player.
- Unit (engine, real cards): combat.rs
  test_spirit_link_fires_on_combat_damage_to_creature (P0 gains 3 when enchanted
  3/3 is blocked by 4/4, defender takes 0) and
  test_player_only_trigger_does_not_fire_on_creature_damage (Hypnotic Specter does
  NOT discard when only a blocker is hit).
- E2E: puzzle_e2e test_spirit_link_lifelink_on_combat_damage_to_creature +
  test_puzzles/spirit_link_blocked_creature_damage.pzl (gain 3, log assertion).

EFFECT_SUPPORT.md updated. MTG Rules Review: PASS (CR 510.2 / 119.3 / 603.2).
See mtg-r9po1 for the residual non-COMBAT (pinger) damage gap.
