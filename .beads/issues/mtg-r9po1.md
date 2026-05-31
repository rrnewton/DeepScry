---
title: 'Bug: T:Mode$ DamageDealtOnce + ValidSource$ Card.AttachedBy + TriggerCount$DamageAmount (triggered pseudo-lifelink) unsupported'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T23:35:49.101624331+00:00
updated_at: 2026-05-31T08:12:55.178751710+00:00
---

# Description

PARTIAL -> combat damage now FULLY WORKING; only non-COMBAT damage remains.
Updated 2026-05-31 (fix-mtg-m43mc-creature-dmg-trigger).

Spirit Link's triggered pseudo-lifelink now fires for ALL COMBAT damage the
enchanted creature deals -- to a PLAYER (was already working, mtg-r9po1 wave-18)
AND now to a CREATURE (blocker/attacker), via the general combat-damage trigger
fix in mtg-m43mc (CLOSED). The DealsCombatDamage firing site now iterates every
creature that dealt combat damage this step (deterministic damage_dealt_by_creature
BTreeMap order) and Spirit Link's any-recipient trigger observes the total combat
damage, matching the Lifelink keyword (CR 510.2 / 119.3).

Evidence (combat damage to a creature):
- Unit: game::actions::tests::combat::test_spirit_link_fires_on_combat_damage_to_creature
  -- enchanted 3/3 blocked by a 4/4: defending player takes 0, P0 gains 3.
- E2E: puzzle_e2e::test_spirit_link_lifelink_on_combat_damage_to_creature
  (test_puzzles/spirit_link_blocked_creature_damage.pzl) -- enchanted Hill Giant
  (3/3) blocked by Wall of Stone (0/8): P1 life unchanged (20), P0 gains 3 (10->13),
  with a 'gains 3 life' gamelog line.
- Existing player-damage E2E test_spirit_link_aura_targeting still green.

REMAINING GAP (still PARTIAL): non-COMBAT damage (e.g. an enchanted PINGER's
'{T}: deal 1 damage' activated ability, or any non-combat damage source the host
deals) does NOT yet fire the DealsCombatDamage trigger -- the trigger is only
fired from the combat-damage step. Real Spirit Link gains life on that damage too
(CR 119.3). Wiring a non-combat damage-dealt trigger hook (deal_damage path)
is the remaining work to mark Spirit Link fully WORKING. Keep this issue OPEN
for that residual; combat lifelink is complete.

MTG Rules Review: PASS (CR 510.2 / 119.3 / 603.2).
