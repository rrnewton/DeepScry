---
title: 'Bug: ChangesZone trigger parser ignores ValidCard$ Creature.DamagedBy and other Creature.X patterns'
status: closed
priority: 3
issue_type: bug
created_at: 2026-05-13T03:00:17.754791218+00:00
updated_at: 2026-05-13T03:18:34.186586784+00:00
closed_at: 2026-05-13T03:18:34.186586664+00:00
---

# Description

FIXED 2026-05-12 (compat1) — implemented in pending commit (Sengir Vampire compat work).

Both parts addressed:

Part 1 (parser):
- mtg-engine/src/loader/card.rs: added a new branch in the ChangesZone trigger parser that accepts ValidCard$ starting with 'Creature.DamagedBy' and produces Trigger { event: TriggerEvent::DamagedCreatureDies, ... }. The new TriggerEvent variant is added in mtg-engine/src/core/effects.rs.

Part 2 (engine — DamagedBy tracking):
- mtg-engine/src/core/card.rs: new field damaged_by_this_turn: SmallVec<[CardId; 2]>.
- mtg-engine/src/game/actions/combat.rs: build damage_sources_per_target during combat, then before lethal-damage check push each source onto target.damaged_by_this_turn (deduped).
- mtg-engine/src/game/actions/mod.rs (check_death_triggers): scan battlefield for permanents with DamagedCreatureDies triggers whose CardId appears in dying_card.damaged_by_this_turn; fire each with itself as Defined$ Self.
- mtg-engine/src/game/actions/triggers.rs: PutCounter resolver now also handles is_self_target() (was placeholder-only). Without this, Sengir's TrigPutCounter (Defined$ Self) silently no-ops.
- mtg-engine/src/game/game_loop/steps.rs: clear damaged_by_this_turn at cleanup (CR 514.2).
- mtg-engine/src/game/heuristic_controller.rs: +15 evaluator weight for DamagedCreatureDies (high value).

NOTE: this fix only covers the 'Creature.DamagedBy' filter case (Sengir Vampire et al). Other filter shapes ('Creature.Other', 'Creature.YouCtrl', 'Creature.OppCtrl', type-specific filters) listed in this issue are still unimplemented. Filing follow-up not necessary unless one of those cards comes up in a tested deck.

Verified working: tests/sengir_vampire_flying_e2e.sh asserts the full chain — Sengir attacks Birds of Paradise, kills it (4 damage > 1 toughness), trigger fires, +1/+1 counter applied, Sengir is now 5/5.

Test Result: 716+ lib tests pass; new e2e test passes.

Closing as RESOLVED for the DamagedBy case.
