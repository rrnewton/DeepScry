---
title: 'Bug: DealsCombatDamage triggers only fire on damage-to-player, not damage-to-creature'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-31T04:00:02.691428461+00:00
updated_at: 2026-05-31T04:00:02.691428461+00:00
---

# Description

Engine gap (pre-existing, surfaced during compat-wave18 Spirit Link work mtg-r9po1).

DealsCombatDamage triggers (Mode$ DamageDone and Mode$ DamageDealtOnce) only
fire from the combat firing site for creatures that dealt combat damage to a
PLAYER. See mtg-engine/src/game/actions/combat.rs: the firing loop iterates
`creatures_that_dealt_player_damage` (player-damage only); the
`damage_dealt_by_creature` BTreeMap (which DOES include damage to blockers /
attackers, and is what real Lifelink uses) is NOT used to fire triggers.

Consequence:
- Spirit Link (mtg-r9po1) gains life only when the enchanted creature deals
  combat damage to a PLAYER, not when it deals combat damage to a creature it is
  blocking / blocked by. Real Spirit Link / Lifelink gains life on ALL combat
  damage (CR 119.3 / 702.15). So trigger-based pseudo-lifelink is strictly
  weaker than the Lifelink keyword in this engine.
- Affects ALL DamageDone/DamageDealtOnce triggers, e.g. Markov Blademaster-style
  "+1/+1 counter when deals combat damage" only counts player damage, etc.
  (Hypnotic Specter is correct since its trigger is player-targeted by design.)

The `[any-damage]` / `[damages-creature]` description markers set by the parser
(loader/card.rs parse_triggers, DamageDone branch) currently have NO consumer at
trigger-execution time, so they can't be relied on to gate player-vs-creature
firing. A proper fix must:
1. Fire DealsCombatDamage (with the per-creature damage amount) for creatures in
   `damage_dealt_by_creature` that dealt damage to a CREATURE as well, and
2. Gate each trigger by its intended target (player vs creature) using a
   STRUCTURED flag (replace the dead `[any-damage]`/`[damages-creature]` markers
   with real Trigger fields), so e.g. a player-only "discard" trigger does NOT
   fire on creature damage.

Until then, Spirit Link is WORKING for the documented primary case (damage to a
player) — see mtg-r9po1.
