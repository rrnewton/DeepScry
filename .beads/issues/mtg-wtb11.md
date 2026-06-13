---
title: 'Puzzle assertion migration: deferred event/log-based assertions catalog (Phase 2 demand)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-13T23:16:43.171180284+00:00
updated_at: 2026-06-13T23:16:43.171180284+00:00
---

# Description

## Context

Phase 1 of the puzzle assertion DSL (mtg-0oopj) implemented final-state assertions ([assertions] section in .pzl files). This issue catalogs the Rust test assertions in puzzle_e2e.rs that CANNOT be migrated to the inline DSL yet because they depend on structured game log events (trigger fired, activation logged, etc.).

## What was migrated (Phase 1 migration, claude/puzzle-migration branch)

8 puzzle files received inline [assertions] sections covering final-state checks:
- serra_angel_should_attack.pzl — game won, opponent life lt 20
- flying_vs_ground.pzl — game won, opponent life lt 8
- vigilance_blocks_back.pzl — game won, NOT game lost
- lethal_through_blockers.pzl — game won, turn le 5
- must_attack_creature.pzl — game won, opponent life lt 20
- large_creature_attack.pzl — game won, opponent life lt 12
- crusade_buff_e2e.pzl — game won, opponent life lt 3
- forestwalk_blocks_forest_owner.pzl — game won, opponent life eq 0
- life_race_decision.pzl — game ended
- lifelink_race_evaluation.pzl — game ended

The corresponding Rust test assertions are kept as belt-and-suspenders (not removed).

## Deferred: EVENT/LOG-derived assertions (require structured GameEvent stream)

These Rust tests in puzzle_e2e.rs have assertions that check game log text (e.message.contains(...)) — these VIOLATE the NO HACKY STRING OPERATIONS rule if moved to the DSL via substring matching. The correct fix requires a structured GameEvent enum (Phase 2 of the DSL, tracked in PUZZLE_ASSERTION_DSL.md).

Count: 17 deferred assertions across 14 test functions.

### Catalog of deferred log-based assertions

1. **test_royal_assassin_with_log_capture** (royal_assassin_kills_attacker.pzl)
   - 
   - Needs: GameEvent::AbilityActivated { card_name }

2. **test_defender_shouldnt_attack** (defender_shouldnt_attack.pzl)
   - 
   - Needs: GameEvent::CreatureDeclaredAttacker { card_id } (absence assertion)

3. **test_mishras_workshop_taps_for_ccc** (mishras_workshop_artifact_cast.pzl)
   - 
   - 
   - Needs: GameEvent::ManaTapped, GameEvent::SpellResolved

4. **test_hurkyls_recall_returns_all_artifacts** (hurkyls_recall_bounce_artifacts.pzl)
   -  (count == 3)
   -  etc.
   - Needs: GameEvent::CardChangedZone { from: Battlefield, to: Hand }

5. **test_timetwister_shuffles_hand_graveyard_and_draws_seven** (timetwister_shuffle_draw7.pzl)
   -  (count == 7)
   -  (count == 7)
   - Needs: GameEvent::CardDrawn { player_id }

6. **test_chain_lightning_deals_three_to_player** (chain_lightning_three_damage.pzl)
   - 
   - 
   -  (absence)
   - Needs: GameEvent::SpellCast, GameEvent::DamageDealt, GameEvent::SpellCopied

7. **test_chain_lightning_copy_chain_when_opponent_has_red** (chain_lightning_copy_chain.pzl)
   - 
   - Needs: GameEvent::SpellCopied { spell_name }

8. **test_drain_life_caps_lifegain_at_player_life** (drain_life_cap.pzl)
   -  (absence assertion)
   - 
   - Needs: GameEvent::LifeGained { amount }, absence of internal debug events

9. **test_drain_life_caps_lifegain_at_creature_toughness** (drain_life_creature_cap.pzl)
   - 
   - 
   - Needs: GameEvent::CardDied, GameEvent::LifeGained

10. **test_mana_drain_deferred_mana** (mana_drain_deferred_mana.pzl)
    - 
    - 
    - Needs: GameEvent::SpellCountered, GameEvent::ManaAddedToPool

11. **test_spirit_link_aura_targeting** (spirit_link_aura.pzl)
    -  (partial)
    - Needs: GameEvent::LifeGained { player, amount }

12. **test_spirit_link_lifelink_on_combat_damage_to_creature** (spirit_link_blocked_creature_damage.pzl)
    - 
    - Needs: GameEvent::LifeGained { amount: 3 }

13. **test_spirit_link_lifelink_on_noncombat_damage** (spirit_link_noncombat_pinger.pzl)
    - 
    - Needs: GameEvent::LifeGained { amount: 1 }

14. **test_fireball_divides_x_among_two_targets** (fireball_divide_two_targets.pzl)
    - 
    - 
    -  
    -  (absence)
    - Needs: GameEvent::SpellCast, GameEvent::DamageDealt { amount, target }

15. **test_earthquake_dragon_graveyard_return** (earthquake_dragon_graveyard_return.pzl)
    - 
    - Needs: GameEvent::AbilityActivated

16. **test_offspring_thundertrap_trainer_creates_1_1_token** (offspring_thundertrap_trainer.pzl)
    -  (cast logged)
    - 
    - Needs: GameEvent::SpellCast, GameEvent::OffspringCostPaid

17. **test_city_in_a_bottle_arn_hoser** (city_in_a_bottle_arn_hoser.pzl)
    - 
    - Needs: GameEvent::PermanentSacrificed { card_name }

## What Phase 2 needs to provide

To migrate the 17 deferred assertions above, Phase 2 must add a structured GameEvent enum alongside the string log:



Once GameEvent is available, the DSL assertion grammar needs new predicates:
-  — at least one SpellCast event for this card
-  — total life gained by me/opponent
-  — a specific card died
-  — a card activated an ability
- etc.

## Priority

This is P3 (granular tracking issue). Phase 2 (structured game log) is the unblocking dependency.
It should be addressed after the game log rework (which also benefits MCTS/fuzz by allowing replay verification).

See: ai_docs/reference/PUZZLE_ASSERTION_DSL.md § Log-derived assertions — DEFERRED
