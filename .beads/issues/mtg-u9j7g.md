---
title: Effect::DamageAll bypasses deal_damage -> non-combat deals-damage triggers don't fire on mass damage
status: open
priority: 4
issue_type: bug
created_at: 2026-05-31T12:24:48.441438395+00:00
updated_at: 2026-05-31T12:24:48.441438395+00:00
---

# Description

Effect::DamageAll (Earthquake, Pestilence, Hurricane, Pyroclasm-style mass damage) applies damage by directly mutating card.damage and calling player.lose_life() in execute_effect (mtg-engine/src/game/actions/mod.rs ~5005-5026), bypassing the shared deal_damage / deal_damage_to_creature functions.

Consequences:
1. The per-resolution non-combat damage accumulator (GameState::damage_dealt_by_source, added in mtg-r9po1) is NOT fed, so a 'whenever ~ deals damage' trigger (Spirit Link CR 119.3) would not fire if an enchanted creature were ever the SOURCE of a DamageAll. In practice DamageAll's source is the resolving spell (not the enchanted creature), so Spirit Link is unaffected today -- hence out of scope for mtg-r9po1.
2. DamageAll-to-players also bypasses source-prevention shields (Circle of Protection) and the regular damage-prevention shield, a pre-existing DRY/correctness inconsistency.

Fix direction (DRY): route DamageAll through deal_damage_to_creature / deal_damage (which already handle prevention shields and now the deals-damage accumulator), or factor a shared apply_damage helper that both the per-target and mass-damage paths call.
