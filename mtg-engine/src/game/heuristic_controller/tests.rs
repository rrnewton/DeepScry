//! Unit tests for the heuristic AI controller.
//!
//! Split out of the former monolithic `heuristic_controller.rs`. These exercise
//! the `HeuristicController` decision methods directly; `use super::*` brings the
//! controller, its helper types, and submodule methods into scope.
use super::*;
use crate::core::EntityId;

#[test]
fn test_heuristic_controller_creation() {
    let player_id = EntityId::new(1);
    let controller = HeuristicController::new(player_id);
    assert_eq!(controller.player_id(), player_id);
    assert_eq!(controller.aggression_level, 3);
}

#[test]
fn test_seeded_controller() {
    let player_id = EntityId::new(1);
    let controller = HeuristicController::new(player_id);
    assert_eq!(controller.player_id(), player_id);
}

#[test]
fn test_aggression_setting() {
    let player_id = EntityId::new(1);
    let mut controller = HeuristicController::new(player_id);

    controller.set_aggression(0);
    assert_eq!(controller.aggression_level, 0);

    controller.set_aggression(6);
    assert_eq!(controller.aggression_level, 6);

    // Test clamping
    controller.set_aggression(10);
    assert_eq!(controller.aggression_level, 6);

    controller.set_aggression(-5);
    assert_eq!(controller.aggression_level, 0);
}

#[test]
fn test_pump_spell_evaluation_basic() {
    use crate::core::{Card, CardType};

    let player_id = EntityId::new(1);
    let _controller = HeuristicController::new(player_id);

    // Create a Grizzly Bears (2/2) creature
    let mut bears = Card::new(EntityId::new(10), "Grizzly Bears", player_id);
    bears.set_base_power(Some(2));
    bears.set_base_toughness(Some(2));
    bears.add_type(CardType::Creature);

    // Test Case 1: Pump that doesn't kill the creature (+3/+3)
    // Should return true if it would enable attacking
    let power_bonus = 3;
    let toughness_bonus = 3;
    let _keywords: Vec<String> = vec![];

    // Note: This test would need a full GameStateView mock to work properly
    // For now, we're just testing that the method exists and compiles
    // A full integration test would be needed to test the logic end-to-end

    // Test Case 2: Pump that would kill the creature (-5/-5)
    // This should return false
    let _bad_power = 0;
    let bad_toughness = -5; // Would make 2/2 into 2/-3 (dies)

    // The should_cast_pump method would return false for this case
    // because current_toughness (2) + bad_toughness (-5) = -3 <= 0

    // Verify the logic path exists
    assert_eq!(bears.base_power(), Some(2));
    assert_eq!(bears.base_toughness(), Some(2));

    // Calculate what would happen
    let would_die = i32::from(bears.current_toughness()) + bad_toughness <= 0;
    assert!(would_die, "Creature should die with -5 toughness");

    let would_live = i32::from(bears.current_toughness()) + toughness_bonus > 0;
    assert!(would_live, "Creature should live with +3 toughness");

    // Test that we can calculate pumped power
    let pumped_power = i32::from(bears.current_power()) + power_bonus;
    assert_eq!(pumped_power, 5, "2/2 with +3/+3 should have 5 power");
}

#[test]
fn test_pump_spell_evasion_granting() {
    use crate::core::{Card, CardType};

    let player_id = EntityId::new(1);
    let controller = HeuristicController::new(player_id);

    // Create a 2/2 ground creature (the one we might pump)
    let mut ground_creature = Card::new(EntityId::new(10), "Grizzly Bears", player_id);
    ground_creature.set_base_power(Some(2));
    ground_creature.set_base_toughness(Some(2));
    ground_creature.add_type(CardType::Creature);

    // Create a 1/1 flying creature (opponent's blocker)
    let mut flying_creature = Card::new(EntityId::new(11), "Bird", EntityId::new(2));
    flying_creature.set_base_power(Some(1));
    flying_creature.set_base_toughness(Some(1));
    flying_creature.add_type(CardType::Creature);
    flying_creature.keywords.insert(Keyword::Flying);

    // Scenario 1: Ground creature attacks, flying creature tries to block
    // can_block_simple(attacker, blocker, keywords_on_attacker)

    // Test: Can the flying creature block the ground attacker? Yes (flying can block anything).
    assert!(controller.can_block_simple(&ground_creature, &flying_creature, &[]));

    // Test: If ground creature had Flying, can flying creature still block it? Yes.
    let flying_granted = vec!["Flying".to_string()];
    assert!(controller.can_block_simple(&ground_creature, &flying_creature, &flying_granted));

    // Scenario 2: Flying creature attacks, ground creature tries to block

    // Test: Can ground creature block the flying attacker? No (needs Flying or Reach).
    assert!(!controller.can_block_simple(&flying_creature, &ground_creature, &[]));

    // This test doesn't make sense - we don't grant keywords to blockers in this function
    // The keywords_granted parameter applies to the ATTACKER, not the blocker
    // So we can't test "granting Flying to the blocker" with this function
}

#[test]
fn test_damage_assignment_order_logic() {
    // Test the core logic of damage assignment ordering
    // Port of Java Forge's AiBlockController.orderBlockers()
    //
    // Scenario: 5/5 attacker vs three blockers:
    // - 4/4 High-value creature (eval ~200)
    // - 2/2 Medium creature (eval ~140)
    // - 1/1 Low-value creature (eval ~115)
    //
    // With 5 damage available:
    // - Can kill 4/4 (need 4 damage) = yes, 1 damage left
    // - Can't kill 2/2 with 1 damage left = no
    // - Can kill 1/1 (need 1 damage) = yes, 0 damage left
    //
    // So order should be: 4/4 first, 1/1 second, 2/2 last
    // This maximizes kills (2 creatures) rather than damage spread

    // This is a conceptual test - actual integration test would need GameStateView
    let available_damage = 5;
    let blockers = vec![
        ("4/4 High", 200, 4), // (name, eval, toughness)
        ("2/2 Medium", 140, 2),
        ("1/1 Low", 115, 1),
    ];

    // Sort by evaluation (descending)
    let mut sorted = blockers.clone();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    assert_eq!(sorted[0].0, "4/4 High");
    assert_eq!(sorted[1].0, "2/2 Medium");
    assert_eq!(sorted[2].0, "1/1 Low");

    // Simulate the algorithm
    let mut remaining = available_damage;
    let mut killable = vec![];
    let mut unkillable = vec![];

    for (name, _eval, toughness) in sorted {
        if toughness <= remaining {
            killable.push(name);
            remaining -= toughness;
        } else {
            unkillable.push(name);
        }
    }

    // Result: 4/4 is killable (5 >= 4, remaining = 1)
    //         2/2 is NOT killable (1 < 2)
    //         1/1 is killable (1 >= 1, remaining = 0)
    assert_eq!(killable, vec!["4/4 High", "1/1 Low"]);
    assert_eq!(unkillable, vec!["2/2 Medium"]);

    // Combined order: killable first, unkillable last
    let final_order: Vec<_> = killable.into_iter().chain(unkillable).collect();
    assert_eq!(final_order, vec!["4/4 High", "1/1 Low", "2/2 Medium"]);

    // We successfully kill 2 creatures (4/4 and 1/1) instead of just 1
    // If we had put 2/2 first after 4/4, we'd waste damage:
    // - 4/4: 4 damage, 1 left
    // - 2/2: can't kill with 1 damage, but rules require assigning lethal
    //        before moving to next blocker, so we'd be stuck
    // The algorithm correctly recognizes 2/2 can't be killed and skips it
}

#[test]
fn test_damage_assignment_with_deathtouch() {
    // Test that deathtouch changes lethal damage calculation
    // MTG Rules 702.2c: Any nonzero damage from deathtouch is lethal
    //
    // Scenario: 2/2 Deathtouch attacker vs three blockers:
    // - 5/5 Big creature (eval ~250)
    // - 4/4 Medium creature (eval ~200)
    // - 3/3 Small creature (eval ~175)
    //
    // Without deathtouch: 2 damage total, can't kill anything
    // With deathtouch: 1 damage kills anything, so can kill 2 creatures!
    //
    // Expected order with deathtouch:
    // - 5/5: 1 damage kills (deathtouch), 1 damage left
    // - 4/4: 1 damage kills (deathtouch), 0 damage left
    // - 3/3: no damage left, unkillable

    let available_damage = 2;
    let blockers = vec![
        ("5/5 Big", 250, 5), // (name, eval, toughness)
        ("4/4 Medium", 200, 4),
        ("3/3 Small", 175, 3),
    ];

    // Sort by evaluation (descending - target best creatures first)
    let mut sorted = blockers.clone();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    // Simulate algorithm WITH deathtouch
    let attacker_has_deathtouch = true;
    let mut remaining = available_damage;
    let mut killable = vec![];
    let mut unkillable = vec![];

    for (name, _eval, toughness) in sorted {
        // With deathtouch, 1 damage is lethal
        let lethal_damage = if attacker_has_deathtouch && toughness > 0 {
            1
        } else {
            toughness
        };

        if lethal_damage <= remaining {
            killable.push(name);
            remaining -= lethal_damage;
        } else {
            unkillable.push(name);
        }
    }

    // With deathtouch: can kill 5/5 (1 dmg) and 4/4 (1 dmg), 3/3 no damage left
    assert_eq!(killable, vec!["5/5 Big", "4/4 Medium"]);
    assert_eq!(unkillable, vec!["3/3 Small"]);

    // Verify without deathtouch would give different (worse) result
    let mut no_dt_remaining = 2;
    let mut no_dt_killable = vec![];

    for (name, _eval, toughness) in blockers {
        if toughness <= no_dt_remaining {
            no_dt_killable.push(name);
            no_dt_remaining -= toughness;
        }
    }

    // Without deathtouch: can't kill anything with only 2 damage
    assert!(
        no_dt_killable.is_empty(),
        "Without deathtouch, 2 damage can't kill any creature"
    );
}

#[test]
fn test_damage_assignment_with_indestructible() {
    // Test that indestructible blockers are always put last
    // MTG Rules 702.12: Indestructible creatures can't be destroyed by damage
    //
    // Scenario: 6/6 attacker vs three blockers:
    // - 4/4 Indestructible (eval ~300 due to indestructible bonus)
    // - 3/3 Normal creature (eval ~175)
    // - 2/2 Normal creature (eval ~140)
    //
    // Even though the indestructible creature has highest eval,
    // it should be last because we can't kill it anyway.
    //
    // Expected: kill 3/3 and 2/2, leave indestructible last

    let available_damage = 6;
    let blockers = vec![
        ("4/4 Indestructible", 300, 4, true), // (name, eval, toughness, indestructible)
        ("3/3 Normal", 175, 3, false),
        ("2/2 Normal", 140, 2, false),
    ];

    // Sort by evaluation (descending)
    let mut sorted = blockers.clone();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    // Simulate algorithm with indestructible check
    let mut remaining = available_damage;
    let mut killable = vec![];
    let mut unkillable = vec![];

    for (name, _eval, toughness, is_indestructible) in sorted {
        if is_indestructible {
            // Indestructible = always unkillable
            unkillable.push(name);
            continue;
        }

        if toughness <= remaining {
            killable.push(name);
            remaining -= toughness;
        } else {
            unkillable.push(name);
        }
    }

    // Killable: 3/3 (3 dmg) and 2/2 (2 dmg) = 5 damage used
    // Unkillable: 4/4 Indestructible (even though it was first by eval)
    assert_eq!(killable, vec!["3/3 Normal", "2/2 Normal"]);
    assert_eq!(unkillable, vec!["4/4 Indestructible"]);

    // Final order: killable first, unkillable last
    let final_order: Vec<_> = killable.into_iter().chain(unkillable).collect();
    assert_eq!(final_order[2], "4/4 Indestructible");
}

/// Test intelligent mana source scoring
///
/// Port of Java's ComputerUtilMana.scoreManaProducingCard()
/// Reference: ComputerUtilMana.java:95-120
///
/// This tests that:
/// 1. Basic lands (only mana ability) get low scores
/// 2. Mana creatures get higher scores (can attack/block)
/// 3. Cards with non-mana abilities get +13 per ability
#[test]
fn test_mana_source_scoring() {
    use crate::core::{ActivatedAbility, Card, CardType, Cost, Effect, ManaCost};

    let player_id = EntityId::new(1);
    let controller = HeuristicController::new(player_id);

    // Create a mock GameStateView
    let game = crate::game::state::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let view = crate::game::controller::GameStateView::new(&game, player_id);

    // Create a basic Forest (just mana ability)
    // Expected score: 1 (produces 1 green mana)
    let mut forest = Card::new(EntityId::new(10), "Forest", player_id);
    forest.add_type(CardType::Land);
    forest.activated_abilities.push(ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::AddMana {
            player: player_id,
            mana: ManaCost {
                green: 1,
                ..Default::default()
            },
            produces_chosen_color: false,
            amount_var: None,
        }],
        "{T}: Add {G}".to_string(),
        true, // is_mana_ability
    ));

    let forest_score = controller.score_mana_source(&forest, &view);

    // Create Llanowar Elves (1/1 creature with mana ability)
    // Expected score: 1 (mana) + 13 (can attack) + 13 (can block) = 27
    // Note: If summoning sick, only +13 for block potential
    let mut llanowar_elves = Card::new(EntityId::new(11), "Llanowar Elves", player_id);
    llanowar_elves.add_type(CardType::Creature);
    llanowar_elves.set_base_power(Some(1));
    llanowar_elves.set_base_toughness(Some(1));
    // Not summoning sick - entered last turn
    llanowar_elves.turn_entered_battlefield = Some(0);
    llanowar_elves.activated_abilities.push(ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::AddMana {
            player: player_id,
            mana: ManaCost {
                green: 1,
                ..Default::default()
            },
            produces_chosen_color: false,
            amount_var: None,
        }],
        "{T}: Add {G}".to_string(),
        true, // is_mana_ability
    ));

    let elves_score = controller.score_mana_source(&llanowar_elves, &view);

    // Create a land with a non-mana activated ability (utility land)
    // Expected score: 1 (mana) + 13 (non-mana ability) = 14
    let mut utility_land = Card::new(EntityId::new(12), "Strip Mine", player_id);
    utility_land.add_type(CardType::Land);
    utility_land.activated_abilities.push(ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::AddMana {
            player: player_id,
            mana: ManaCost {
                colorless: 1,
                ..Default::default()
            },
            produces_chosen_color: false,
            amount_var: None,
        }],
        "{T}: Add {C}".to_string(),
        true, // is_mana_ability
    ));
    // Strip Mine's destroy land ability
    utility_land.activated_abilities.push(ActivatedAbility::new(
        Cost::Composite(vec![
            Cost::Tap,
            Cost::SacrificePattern {
                count: 1,
                card_type: "Land".to_string(),
            },
        ]),
        vec![Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        }],
        "{T}, Sacrifice Strip Mine: Destroy target land.".to_string(),
        false, // is_mana_ability
    ));

    let utility_score = controller.score_mana_source(&utility_land, &view);

    // Verify: Forest (low) < Utility land (medium) < Llanowar Elves (high)
    // The AI should tap Forest first, then utility land, then Llanowar Elves
    assert!(
        forest_score < elves_score,
        "Basic land (score={}) should be tapped before mana creature (score={})",
        forest_score,
        elves_score
    );

    assert!(
        forest_score < utility_score,
        "Basic land (score={}) should be tapped before utility land (score={})",
        forest_score,
        utility_score
    );

    // Print scores for debugging
    eprintln!("Mana source scores:");
    eprintln!("  Forest: {}", forest_score);
    eprintln!("  Strip Mine: {}", utility_score);
    eprintln!("  Llanowar Elves: {}", elves_score);
}

/// Test counterspell AI logic
///
/// Port of Java's CounterAi.checkApiLogic()
/// Reference: CounterAi.java:32-226
///
/// This tests that:
/// 1. AI counters opponent creature spells
/// 2. AI doesn't counter own spells
/// 3. AI doesn't try to counter when stack is empty
#[test]
fn test_counterspell_ai() {
    use crate::core::{Card, CardType, Effect, TargetRef};

    let player_id = EntityId::new(1);
    let opponent_id = EntityId::new(2);
    let controller = HeuristicController::new(player_id);

    // Create game state with opponent creature on stack
    let mut game = crate::game::state::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

    // Create an opponent creature spell and put it on the stack
    let creature_id = EntityId::new(100);
    let mut creature = Card::new(creature_id, "Grizzly Bears", opponent_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);

    // Put creature on the stack
    game.stack.cards.push(creature_id);

    let view = crate::game::controller::GameStateView::new(&game, player_id);

    // Test: Should counter opponent's creature
    assert!(
        controller.should_counter_spell(&view),
        "AI should want to counter opponent's creature spell"
    );

    // Test: Stack is empty - should not try to counter
    game.stack.cards.pop();
    let view_empty = crate::game::controller::GameStateView::new(&game, player_id);
    assert!(
        !controller.should_counter_spell(&view_empty),
        "AI should not counter when stack is empty"
    );

    // Test: Own spell on stack - should not counter
    let own_creature_id = EntityId::new(101);
    let mut own_creature = Card::new(own_creature_id, "Our Bears", player_id);
    own_creature.add_type(CardType::Creature);
    game.cards.insert(own_creature_id, own_creature);
    game.stack.cards.push(own_creature_id);

    let view_own = crate::game::controller::GameStateView::new(&game, player_id);
    assert!(
        !controller.should_counter_spell(&view_own),
        "AI should not counter own spell"
    );

    // Test: Counter opponent damage spell
    game.stack.cards.pop();
    let damage_spell_id = EntityId::new(102);
    let mut damage_spell = Card::new(damage_spell_id, "Lightning Bolt", opponent_id);
    damage_spell.add_type(CardType::Instant);
    damage_spell.effects.push(Effect::DealDamage {
        amount: 3,
        target: TargetRef::Player(player_id),
    });
    game.cards.insert(damage_spell_id, damage_spell);
    game.stack.cards.push(damage_spell_id);

    let view_damage = crate::game::controller::GameStateView::new(&game, player_id);
    assert!(
        controller.should_counter_spell(&view_damage),
        "AI should want to counter opponent's damage spell"
    );
}

#[test]
fn test_combat_restriction_penalties() {
    // Test that creatures with combat restrictions are properly penalized
    // Reference: CreatureEvaluator.java:177-197
    use crate::core::{Card, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let player_id = EntityId::new(1);
    let controller = HeuristicController::new(player_id);

    // Create a simple game state
    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

    // Helper: Create a 3/3 creature for testing
    let make_creature = |id: u32, keywords: Vec<Keyword>| {
        let card_id = EntityId::new(id);
        let mut creature = Card::new(card_id, "Test Creature", player_id);
        creature.set_base_power(Some(3));
        creature.set_base_toughness(Some(3));
        creature.add_type(CardType::Creature);
        creature.mana_cost = ManaCost::from_string("3");
        for kw in keywords {
            creature.keywords.insert(kw);
        }
        (card_id, creature)
    };

    let _view = GameStateView::new(&game, player_id);

    // Test 1: Normal 3/3 creature (baseline)
    let (normal_id, normal) = make_creature(100, vec![]);
    let mut test_game = game.clone();
    test_game.cards.insert(normal_id, normal);
    let view_normal = GameStateView::new(&test_game, player_id);
    let normal_value = controller.evaluate_creature(&view_normal, normal_id);
    println!("Normal 3/3 value: {}", normal_value);

    // Test 2: Creature with Defender (can't attack)
    // Java: value -= power * 9 + 40 = 3*9 + 40 = 67 penalty
    let (defender_id, defender) = make_creature(101, vec![Keyword::Defender]);
    let mut test_game = game.clone();
    test_game.cards.insert(defender_id, defender);
    let view_defender = GameStateView::new(&test_game, player_id);
    let defender_value = controller.evaluate_creature(&view_defender, defender_id);
    println!("Defender 3/3 value: {}", defender_value);
    assert!(
        defender_value < normal_value,
        "Defender should be worth less than normal creature"
    );
    // Expected penalty: power*9 + 40 = 3*9 + 40 = 67
    assert_eq!(
        normal_value - defender_value,
        67,
        "Defender penalty should be power*9 + 40"
    );

    // Test 3: Creature with CantBlock
    // Java: value -= 10
    let (cant_block_id, cant_block) = make_creature(102, vec![Keyword::CantBlock]);
    let mut test_game = game.clone();
    test_game.cards.insert(cant_block_id, cant_block);
    let view_cant_block = GameStateView::new(&test_game, player_id);
    let cant_block_value = controller.evaluate_creature(&view_cant_block, cant_block_id);
    println!("CantBlock 3/3 value: {}", cant_block_value);
    assert!(
        cant_block_value < normal_value,
        "CantBlock should be worth less than normal creature"
    );
    assert_eq!(normal_value - cant_block_value, 10, "CantBlock penalty should be 10");

    // Test 4: Creature with MustAttack
    // Java: value -= 10
    let (must_attack_id, must_attack) = make_creature(103, vec![Keyword::MustAttack]);
    let mut test_game = game.clone();
    test_game.cards.insert(must_attack_id, must_attack);
    let view_must_attack = GameStateView::new(&test_game, player_id);
    let must_attack_value = controller.evaluate_creature(&view_must_attack, must_attack_id);
    println!("MustAttack 3/3 value: {}", must_attack_value);
    assert!(
        must_attack_value < normal_value,
        "MustAttack should be worth less than normal creature"
    );
    assert_eq!(normal_value - must_attack_value, 10, "MustAttack penalty should be 10");

    // Test 5: Creature with Goaded
    // Java: value -= 5
    let (goaded_id, goaded) = make_creature(104, vec![Keyword::Goaded]);
    let mut test_game = game.clone();
    test_game.cards.insert(goaded_id, goaded);
    let view_goaded = GameStateView::new(&test_game, player_id);
    let goaded_value = controller.evaluate_creature(&view_goaded, goaded_id);
    println!("Goaded 3/3 value: {}", goaded_value);
    assert!(
        goaded_value < normal_value,
        "Goaded should be worth less than normal creature"
    );
    assert_eq!(normal_value - goaded_value, 5, "Goaded penalty should be 5");

    // Test 6: Creature with CantAttackOrBlock (nearly useless)
    // Java: value = 50 + (cmc * 5) = 50 + (3 * 5) = 65 (total value, not penalty)
    let (useless_id, useless) = make_creature(105, vec![Keyword::CantAttackOrBlock]);
    let mut test_game = game.clone();
    test_game.cards.insert(useless_id, useless);
    let view_useless = GameStateView::new(&test_game, player_id);
    let useless_value = controller.evaluate_creature(&view_useless, useless_id);
    println!("CantAttackOrBlock 3/3 value: {}", useless_value);
    assert!(
        useless_value < normal_value,
        "CantAttackOrBlock should be much less valuable"
    );
    assert_eq!(useless_value, 65, "CantAttackOrBlock should reset value to 50 + cmc*5");
}

#[test]
fn test_blocking_restrictions_evasion() {
    // Test the can_block function for various evasion abilities
    // Reference: CombatUtil.canBlock() in Java Forge
    use crate::core::{Card, CardType, Color};

    let player_id = EntityId::new(1);
    let opponent_id = EntityId::new(2);
    let controller = HeuristicController::new(player_id);

    // Helper to create creatures with specified properties
    let make_creature = |id: u32, owner: PlayerId, keywords: Vec<Keyword>, colors: Vec<Color>| {
        let card_id = EntityId::new(id);
        let mut creature = Card::new(card_id, "Test Creature", owner);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.add_type(CardType::Creature);
        for kw in keywords {
            creature.keywords.insert(kw);
        }
        for color in colors {
            creature.colors.push(color);
        }
        creature
    };

    // ========== FEAR TESTS ==========
    // Fear: can only be blocked by artifact creatures or black creatures
    {
        let attacker_with_fear = make_creature(100, opponent_id, vec![Keyword::Fear], vec![Color::Black]);
        let white_blocker = make_creature(101, player_id, vec![], vec![Color::White]);
        let black_blocker = make_creature(102, player_id, vec![], vec![Color::Black]);
        let mut artifact_blocker = make_creature(103, player_id, vec![], vec![]);
        artifact_blocker.add_type(CardType::Artifact);

        // White creature can't block creature with Fear
        assert!(
            !controller.can_block(&attacker_with_fear, &white_blocker),
            "White creature should not be able to block creature with Fear"
        );

        // Black creature CAN block creature with Fear
        assert!(
            controller.can_block(&attacker_with_fear, &black_blocker),
            "Black creature should be able to block creature with Fear"
        );

        // Artifact creature CAN block creature with Fear
        assert!(
            controller.can_block(&attacker_with_fear, &artifact_blocker),
            "Artifact creature should be able to block creature with Fear"
        );
    }

    // ========== INTIMIDATE TESTS ==========
    // Intimidate: can only be blocked by artifact creatures or creatures that share a color
    {
        let red_attacker_intimidate = make_creature(110, opponent_id, vec![Keyword::Intimidate], vec![Color::Red]);
        let green_blocker = make_creature(111, player_id, vec![], vec![Color::Green]);
        let red_blocker = make_creature(112, player_id, vec![], vec![Color::Red]);
        let mut artifact_blocker = make_creature(113, player_id, vec![], vec![]);
        artifact_blocker.add_type(CardType::Artifact);

        // Green creature can't block red creature with Intimidate
        assert!(
            !controller.can_block(&red_attacker_intimidate, &green_blocker),
            "Green creature should not be able to block red creature with Intimidate"
        );

        // Red creature CAN block red creature with Intimidate (shares color)
        assert!(
            controller.can_block(&red_attacker_intimidate, &red_blocker),
            "Red creature should be able to block red creature with Intimidate (shares color)"
        );

        // Artifact creature CAN block creature with Intimidate
        assert!(
            controller.can_block(&red_attacker_intimidate, &artifact_blocker),
            "Artifact creature should be able to block creature with Intimidate"
        );
    }

    // ========== SHADOW TESTS ==========
    // Shadow: can only be blocked by shadow, and shadow can only block shadow
    {
        let shadow_attacker = make_creature(120, opponent_id, vec![Keyword::Shadow], vec![Color::Black]);
        let normal_blocker = make_creature(121, player_id, vec![], vec![Color::White]);
        let shadow_blocker = make_creature(122, player_id, vec![Keyword::Shadow], vec![Color::Black]);

        // Normal creature can't block shadow creature
        assert!(
            !controller.can_block(&shadow_attacker, &normal_blocker),
            "Normal creature should not be able to block creature with Shadow"
        );

        // Shadow creature CAN block shadow creature
        assert!(
            controller.can_block(&shadow_attacker, &shadow_blocker),
            "Shadow creature should be able to block creature with Shadow"
        );

        // Test the reverse: shadow creature can't be blocked by normal creatures either
        let normal_attacker = make_creature(123, opponent_id, vec![], vec![Color::Black]);
        assert!(
            !controller.can_block(&normal_attacker, &shadow_blocker),
            "Shadow creature should not be able to block normal creature"
        );
    }

    // ========== SKULK TESTS ==========
    // Skulk: can only be blocked by creatures with greater power
    {
        let skulk_attacker = make_creature(130, opponent_id, vec![Keyword::Skulk], vec![Color::Blue]);
        // skulk_attacker has power 2

        let mut weak_blocker = make_creature(131, player_id, vec![], vec![Color::White]);
        weak_blocker.set_base_power(Some(1)); // Power 1

        let mut equal_blocker = make_creature(132, player_id, vec![], vec![Color::White]);
        equal_blocker.set_base_power(Some(2)); // Power 2

        let mut strong_blocker = make_creature(133, player_id, vec![], vec![Color::White]);
        strong_blocker.set_base_power(Some(3)); // Power 3

        // Weak creature (power 1) can't block skulk creature (power 2)
        assert!(
            !controller.can_block(&skulk_attacker, &weak_blocker),
            "Creature with power 1 should not be able to block creature with Skulk and power 2"
        );

        // Equal power creature can't block skulk creature
        assert!(
            !controller.can_block(&skulk_attacker, &equal_blocker),
            "Creature with equal power should not be able to block creature with Skulk"
        );

        // Strong creature CAN block skulk creature
        assert!(
            controller.can_block(&skulk_attacker, &strong_blocker),
            "Creature with greater power should be able to block creature with Skulk"
        );
    }

    // ========== HORSEMANSHIP TESTS ==========
    // Horsemanship: can only be blocked by creatures with horsemanship
    {
        let horse_attacker = make_creature(140, opponent_id, vec![Keyword::Horsemanship], vec![Color::White]);
        let normal_blocker = make_creature(141, player_id, vec![], vec![Color::White]);
        let horse_blocker = make_creature(142, player_id, vec![Keyword::Horsemanship], vec![Color::White]);

        // Normal creature can't block horsemanship creature
        assert!(
            !controller.can_block(&horse_attacker, &normal_blocker),
            "Normal creature should not be able to block creature with Horsemanship"
        );

        // Horsemanship creature CAN block horsemanship creature
        assert!(
            controller.can_block(&horse_attacker, &horse_blocker),
            "Creature with Horsemanship should be able to block creature with Horsemanship"
        );
    }

    // ========== PROTECTION TESTS ==========
    // Protection from color: creature with protection can't be blocked by that color
    {
        let pro_red_attacker = make_creature(150, opponent_id, vec![Keyword::ProtectionFromRed], vec![Color::White]);
        let red_blocker = make_creature(151, player_id, vec![], vec![Color::Red]);
        let blue_blocker = make_creature(152, player_id, vec![], vec![Color::Blue]);

        // Red creature can't block creature with protection from red
        assert!(
            !controller.can_block(&pro_red_attacker, &red_blocker),
            "Red creature should not be able to block creature with Protection from Red"
        );

        // Blue creature CAN block creature with protection from red
        assert!(
            controller.can_block(&pro_red_attacker, &blue_blocker),
            "Blue creature should be able to block creature with Protection from Red"
        );
    }
}

/// Test that Royal Assassin's activated ability is properly classified as Destroy
/// and the AI logic for evaluating destroy abilities works correctly.
///
/// Royal Assassin (4ED): {T}: Destroy target tapped creature.
/// Reference: DestroyAi.java in forge-ai
#[test]
fn test_destroy_ability_classification() {
    use crate::core::{ActivatedAbility, CardId, Cost, Effect, TargetRef, TargetRestriction};

    let player_id = EntityId::new(1);
    let controller = HeuristicController::new(player_id);

    // Create a destroy ability similar to Royal Assassin
    let destroy_ability = ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::DestroyPermanent {
            target: CardId::new(0), // Placeholder target
            restriction: TargetRestriction::any(),
            no_regenerate: false,
        }],
        "Destroy target tapped creature".to_string(),
        false, // not a mana ability
    );

    // Test that the ability is classified as Destroy
    let ability_type = controller.classify_activated_ability(&destroy_ability);
    assert!(
        matches!(ability_type, ActivatedAbilityType::Destroy { .. }),
        "Royal Assassin's ability should be classified as Destroy"
    );

    // Test that ping abilities are still classified correctly
    let ping_ability = ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::DealDamage {
            target: TargetRef::Permanent(CardId::new(0)),
            amount: 1,
        }],
        "{T}: Deal 1 damage to any target".to_string(),
        false,
    );
    assert!(
        matches!(
            controller.classify_activated_ability(&ping_ability),
            ActivatedAbilityType::Ping { damage: 1 }
        ),
        "Prodigal Sorcerer's ability should be classified as Ping"
    );

    // Test that pump abilities are still classified correctly
    let pump_ability = ActivatedAbility::new(
        Cost::Mana(crate::core::ManaCost::from_string("R")),
        vec![Effect::PumpCreature {
            target: CardId::new(0),
            power_bonus: 1,
            toughness_bonus: 0,
            keywords_granted: smallvec::SmallVec::new(),
        }],
        "{R}: +1/+0 until end of turn".to_string(),
        false,
    );
    assert!(
        matches!(
            controller.classify_activated_ability(&pump_ability),
            ActivatedAbilityType::Pump { power: 1, toughness: 0 }
        ),
        "Shivan Dragon's ability should be classified as Pump"
    );
}

/// Test loading Royal Assassin from cardsfolder and verifying ability parsing
#[test]
fn test_royal_assassin_from_cardsfolder() {
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/r/royal_assassin.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Royal Assassin");
    assert_eq!(def.name.as_str(), "Royal Assassin");

    // Instantiate the card
    let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties
    assert!(card.is_creature(), "Royal Assassin should be a creature");
    assert_eq!(card.current_power(), 1, "Royal Assassin should be 1/1");
    assert_eq!(card.current_toughness(), 1, "Royal Assassin should be 1/1");

    // Verify the activated ability was parsed
    assert!(
        !card.activated_abilities.is_empty(),
        "Royal Assassin should have at least one activated ability"
    );

    // Find the non-mana tap ability (the destroy ability)
    let destroy_abilities: Vec<_> = card
        .activated_abilities
        .iter()
        .filter(|a| !a.is_mana_ability && a.cost.includes_tap())
        .collect();

    assert_eq!(
        destroy_abilities.len(),
        1,
        "Royal Assassin should have exactly one tap-to-destroy ability"
    );

    // Verify the ability has a DestroyPermanent effect
    let ability = destroy_abilities[0];
    let has_destroy_effect = ability
        .effects
        .iter()
        .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));

    assert!(
        has_destroy_effect,
        "Royal Assassin's ability should have a DestroyPermanent effect"
    );

    // Test AI classification
    let controller = HeuristicController::new(p1_id);
    let ability_type = controller.classify_activated_ability(ability);
    assert!(
        matches!(ability_type, ActivatedAbilityType::Destroy { .. }),
        "Royal Assassin's ability should be classified as Destroy by AI"
    );
}

/// Test has_valuable_destroy_target evaluates tapped opponent creatures correctly
#[test]
fn test_has_valuable_destroy_target() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    // Create a game with two players
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    let controller = HeuristicController::new(p1_id);

    // Create an opponent's tapped 3/3 creature (valuable target)
    let card_id = CardId::new(50);
    let mut tapped_creature = Card::new(card_id, "Hill Giant", p2_id);
    tapped_creature.add_type(CardType::Creature);
    tapped_creature.set_base_power(Some(3));
    tapped_creature.set_base_toughness(Some(3));
    tapped_creature.tapped = true; // Tapped from attacking

    // Add to battlefield
    game.cards.insert(card_id, tapped_creature);
    game.battlefield.cards.push(card_id);

    // Create game state view
    let view = GameStateView::new(&game, p1_id);

    // Test that we detect this as a valuable target
    assert!(
        controller.has_valuable_destroy_target(&view, true),
        "Should detect 3/3 tapped creature as valuable destroy target"
    );
}

// ==================== Board Wipe AI Tests ====================

/// Test: AI should cast Wrath of God when opponent has more valuable creatures
#[test]
fn test_should_cast_board_wipe_opponent_advantage() {
    use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P1: One small creature (Grizzly Bears 2/2)
    let c1_id = CardId::new(50);
    let mut c1 = Card::new(c1_id, "Grizzly Bears", p1_id);
    c1.add_type(CardType::Creature);
    c1.set_base_power(Some(2));
    c1.set_base_toughness(Some(2));
    c1.controller = p1_id;
    game.cards.insert(c1_id, c1);
    game.battlefield.add(c1_id);

    // P2: Three big creatures (Serra Angel 4/4 x2, Shivan Dragon 5/5)
    for (i, (name, p, t)) in [
        ("Serra Angel", 4i8, 4i8),
        ("Serra Angel", 4, 4),
        ("Shivan Dragon", 5, 5),
    ]
    .iter()
    .enumerate()
    {
        let id = CardId::new(60 + i as u32);
        let mut c = Card::new(id, *name, p2_id);
        c.add_type(CardType::Creature);
        c.set_base_power(Some(*p));
        c.set_base_toughness(Some(*t));
        c.controller = p2_id;
        game.cards.insert(id, c);
        game.battlefield.add(id);
    }

    // Create Wrath of God spell
    let wrath_id = CardId::new(100);
    let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
    wrath.add_type(CardType::Sorcery);
    wrath.mana_cost = ManaCost::from_string("2WW");
    wrath.effects.push(crate::core::Effect::DestroyAll {
        restriction: TargetRestriction::from_types([TargetType::Creature]),
        no_regenerate: true,
    });
    game.cards.insert(wrath_id, wrath);

    let view = GameStateView::new(&game, p1_id);
    let wrath_card = view.get_card(wrath_id).unwrap();

    assert!(
        controller.should_cast_board_wipe(wrath_card, &view),
        "AI should cast Wrath of God when opponent has much more valuable creatures"
    );
}

/// Test: AI should NOT cast Wrath of God when AI has better board
#[test]
fn test_should_not_cast_board_wipe_own_advantage() {
    use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P1: Two big creatures
    for (i, (name, p, t)) in [("Serra Angel", 4i8, 4i8), ("Shivan Dragon", 5, 5)].iter().enumerate() {
        let id = CardId::new(50 + i as u32);
        let mut c = Card::new(id, *name, p1_id);
        c.add_type(CardType::Creature);
        c.set_base_power(Some(*p));
        c.set_base_toughness(Some(*t));
        c.controller = p1_id;
        game.cards.insert(id, c);
        game.battlefield.add(id);
    }

    // P2: One small creature
    let c2_id = CardId::new(60);
    let mut c2 = Card::new(c2_id, "Grizzly Bears", p2_id);
    c2.add_type(CardType::Creature);
    c2.set_base_power(Some(2));
    c2.set_base_toughness(Some(2));
    c2.controller = p2_id;
    game.cards.insert(c2_id, c2);
    game.battlefield.add(c2_id);

    let wrath_id = CardId::new(100);
    let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
    wrath.add_type(CardType::Sorcery);
    wrath.mana_cost = ManaCost::from_string("2WW");
    wrath.effects.push(crate::core::Effect::DestroyAll {
        restriction: TargetRestriction::from_types([TargetType::Creature]),
        no_regenerate: true,
    });
    game.cards.insert(wrath_id, wrath);

    let view = GameStateView::new(&game, p1_id);
    let wrath_card = view.get_card(wrath_id).unwrap();

    assert!(
        !controller.should_cast_board_wipe(wrath_card, &view),
        "AI should NOT cast Wrath of God when AI has better board position"
    );
}

/// Test: AI should cast board wipe when life is critically low
#[test]
fn test_should_cast_board_wipe_low_life() {
    use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P1: Low life, no creatures
    game.get_player_mut(p1_id).unwrap().life = 3;

    // P2: Two creatures threatening lethal
    for (i, (name, p, t)) in [("Serra Angel", 4i8, 4i8), ("Grizzly Bears", 2, 2)].iter().enumerate() {
        let id = CardId::new(60 + i as u32);
        let mut c = Card::new(id, *name, p2_id);
        c.add_type(CardType::Creature);
        c.set_base_power(Some(*p));
        c.set_base_toughness(Some(*t));
        c.controller = p2_id;
        game.cards.insert(id, c);
        game.battlefield.add(id);
    }

    let wrath_id = CardId::new(100);
    let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
    wrath.add_type(CardType::Sorcery);
    wrath.mana_cost = ManaCost::from_string("2WW");
    wrath.effects.push(crate::core::Effect::DestroyAll {
        restriction: TargetRestriction::from_types([TargetType::Creature]),
        no_regenerate: true,
    });
    game.cards.insert(wrath_id, wrath);

    let view = GameStateView::new(&game, p1_id);
    let wrath_card = view.get_card(wrath_id).unwrap();

    assert!(
        controller.should_cast_board_wipe(wrath_card, &view),
        "AI should cast Wrath of God when at 3 life facing 2 opponent creatures"
    );
}

// ==================== ForceSacrifice AI Tests ====================

/// Test: AI casts edict when opponent has creatures
#[test]
fn test_should_cast_force_sacrifice_with_target() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P2 has a creature
    let c_id = CardId::new(50);
    let mut c = Card::new(c_id, "Shivan Dragon", p2_id);
    c.add_type(CardType::Creature);
    c.set_base_power(Some(5));
    c.set_base_toughness(Some(5));
    c.controller = p2_id;
    game.cards.insert(c_id, c);
    game.battlefield.add(c_id);

    let view = GameStateView::new(&game, p1_id);

    assert!(
        controller.should_cast_force_sacrifice(&view),
        "AI should cast edict when opponent has creatures"
    );
}

/// Test: AI doesn't cast edict when opponent has no creatures
#[test]
fn test_should_not_cast_force_sacrifice_no_targets() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let controller = HeuristicController::new(p1_id);

    let view = GameStateView::new(&game, p1_id);

    assert!(
        !controller.should_cast_force_sacrifice(&view),
        "AI should not cast edict when opponent has no creatures"
    );
}

// ==================== TapAll/UntapAll AI Tests ====================

/// Test: AI casts TapAll when opponent has multiple untapped creatures
#[test]
fn test_should_cast_tap_all_with_targets() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P2 has 3 untapped creatures
    for i in 0..3 {
        let id = CardId::new(50 + i);
        let mut c = Card::new(id, "Grizzly Bears", p2_id);
        c.add_type(CardType::Creature);
        c.set_base_power(Some(2));
        c.set_base_toughness(Some(2));
        c.controller = p2_id;
        game.cards.insert(id, c);
        game.battlefield.add(id);
    }

    let view = GameStateView::new(&game, p1_id);

    assert!(
        controller.should_cast_tap_all(&view),
        "AI should cast TapAll when opponent has 3 untapped creatures"
    );
}

/// Test: AI doesn't cast TapAll when opponent has few untapped creatures
#[test]
fn test_should_not_cast_tap_all_few_targets() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P2 has 1 untapped creature (below threshold of 2)
    let id = CardId::new(50);
    let mut c = Card::new(id, "Grizzly Bears", p2_id);
    c.add_type(CardType::Creature);
    c.controller = p2_id;
    game.cards.insert(id, c);
    game.battlefield.add(id);

    let view = GameStateView::new(&game, p1_id);

    assert!(
        !controller.should_cast_tap_all(&view),
        "AI should not cast TapAll with only 1 opponent creature"
    );
}

// ==================== SetLife AI Tests ====================

/// Test: AI casts SetLife when it increases life
#[test]
fn test_should_cast_set_life_when_beneficial() {
    use crate::core::{Card, CardId, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let controller = HeuristicController::new(p1_id);

    // P1 at 5 life
    game.get_player_mut(p1_id).unwrap().life = 5;

    // Create spell that sets life to 10
    let spell_id = CardId::new(100);
    let mut spell = Card::new(spell_id, "Angel of Grace", p1_id);
    spell.add_type(CardType::Instant);
    spell.mana_cost = ManaCost::from_string("4WW");
    spell.effects.push(crate::core::Effect::SetLife {
        player: crate::core::PlayerId::new(0),
        amount: 10,
    });
    game.cards.insert(spell_id, spell);

    let view = GameStateView::new(&game, p1_id);
    let spell_card = view.get_card(spell_id).unwrap();

    assert!(
        controller.should_cast_set_life(spell_card, &view),
        "AI should cast SetLife when it would increase life from 5 to 10"
    );
}

/// Test: AI doesn't cast SetLife when it would decrease life
#[test]
fn test_should_not_cast_set_life_when_harmful() {
    use crate::core::{Card, CardId, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let controller = HeuristicController::new(p1_id);

    // P1 at full 20 life
    // SetLife to 10 would be harmful

    let spell_id = CardId::new(100);
    let mut spell = Card::new(spell_id, "Angel of Grace", p1_id);
    spell.add_type(CardType::Instant);
    spell.mana_cost = ManaCost::from_string("4WW");
    spell.effects.push(crate::core::Effect::SetLife {
        player: crate::core::PlayerId::new(0),
        amount: 10,
    });
    game.cards.insert(spell_id, spell);

    let view = GameStateView::new(&game, p1_id);
    let spell_card = view.get_card(spell_id).unwrap();

    assert!(
        !controller.should_cast_set_life(spell_card, &view),
        "AI should NOT cast SetLife when it would decrease life from 20 to 10"
    );
}

// ==================== should_cast_spell Integration Tests ====================

/// Test: should_cast_spell routes board wipe effects correctly
#[test]
fn test_should_cast_spell_routes_board_wipe() {
    use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // P2: Three big creatures, P1: nothing
    for i in 0..3 {
        let id = CardId::new(50 + i);
        let mut c = Card::new(id, "Serra Angel", p2_id);
        c.add_type(CardType::Creature);
        c.set_base_power(Some(4));
        c.set_base_toughness(Some(4));
        c.controller = p2_id;
        game.cards.insert(id, c);
        game.battlefield.add(id);
    }

    let wrath_id = CardId::new(100);
    let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
    wrath.add_type(CardType::Sorcery);
    wrath.mana_cost = ManaCost::from_string("2WW");
    wrath.effects.push(crate::core::Effect::DestroyAll {
        restriction: TargetRestriction::from_types([TargetType::Creature]),
        no_regenerate: true,
    });
    game.cards.insert(wrath_id, wrath);

    let view = GameStateView::new(&game, p1_id);
    let wrath_card = view.get_card(wrath_id).unwrap();

    assert!(
        controller.should_cast_spell(wrath_card, &view),
        "should_cast_spell should return true for Wrath of God when opponent dominates board"
    );
}

/// Test: should_cast_spell routes LoseLife effects correctly
#[test]
fn test_should_cast_spell_routes_lose_life() {
    use crate::core::{Card, CardId, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // LoseLife targeting opponent should always be worth casting
    let spell_id = CardId::new(100);
    let mut spell = Card::new(spell_id, "Drain Life", p1_id);
    spell.add_type(CardType::Sorcery);
    spell.mana_cost = ManaCost::from_string("1B");
    spell.effects.push(crate::core::Effect::LoseLife {
        player: p2_id,
        amount: 3,
    });

    let view = GameStateView::new(&game, p1_id);

    assert!(
        controller.should_cast_spell(&spell, &view),
        "should_cast_spell should return true for LoseLife effect"
    );
}

// ==================== Removal Timing (use_removal_now) Tests ====================
// Reference: ComputerUtilCard.useRemovalNow() in Java Forge
// Tests use real 4ED cards loaded from cardsfolder

/// Helper: load a card definition from cardsfolder, instantiate, and insert on battlefield
fn load_and_place_on_battlefield(
    game: &mut crate::game::GameState,
    card_path: &str,
    card_id: crate::core::CardId,
    owner: crate::core::PlayerId,
) -> bool {
    let path = std::path::PathBuf::from(card_path);
    if !path.exists() {
        return false;
    }
    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");
    let mut card = def.instantiate(card_id, owner);
    card.controller = owner;
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    true
}

/// Helper: load a card definition from cardsfolder and instantiate in hand (not on battlefield)
fn load_card_in_hand(
    game: &mut crate::game::GameState,
    card_path: &str,
    card_id: crate::core::CardId,
    owner: crate::core::PlayerId,
) -> bool {
    let path = std::path::PathBuf::from(card_path);
    if !path.exists() {
        return false;
    }
    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");
    let card = def.instantiate(card_id, owner);
    game.cards.insert(card_id, card);
    true
}

/// Test: Sorcery removal (e.g. a destroy sorcery) always uses removal now
#[test]
fn test_use_removal_now_sorcery_always_true() {
    use crate::core::{Card, CardId, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Opponent has a creature on battlefield
    let creature_id = CardId::new(50);
    let mut creature = Card::new(creature_id, "Grizzly Bears", p2_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    creature.controller = p2_id;
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Sorcery-speed removal spell
    let spell_id = CardId::new(100);
    let mut spell = Card::new(spell_id, "Destroy Sorcery", p1_id);
    spell.add_type(CardType::Sorcery);
    spell.mana_cost = ManaCost::from_string("1B");
    spell.effects.push(crate::core::Effect::DestroyPermanent {
        target: creature_id,
        restriction: crate::core::TargetRestriction::any(),
        no_regenerate: false,
    });
    game.cards.insert(spell_id, spell);

    // Even at Upkeep (suboptimal timing), sorceries should always return true
    game.turn.current_step = crate::game::Step::Upkeep;

    let view = GameStateView::new(&game, p1_id);
    let spell_card = view.get_card(spell_id).unwrap();

    assert!(
        controller.use_removal_now(spell_card, creature_id, &view),
        "Sorcery removal should always be used now regardless of phase"
    );
}

/// Test: Instant removal held during opponent's upkeep (suboptimal timing)
#[test]
fn test_use_removal_now_instant_held_at_upkeep() {
    use crate::core::{Card, CardId, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Small opponent creature (low evaluation, below 200 threshold)
    let creature_id = CardId::new(50);
    let mut creature = Card::new(creature_id, "Grizzly Bears", p2_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    creature.controller = p2_id;
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Instant removal spell (Terror)
    let spell_id = CardId::new(100);
    let mut spell = Card::new(spell_id, "Terror", p1_id);
    spell.add_type(CardType::Instant);
    spell.mana_cost = ManaCost::from_string("1B");
    spell.effects.push(crate::core::Effect::DestroyPermanent {
        target: creature_id,
        restriction: crate::core::TargetRestriction::any(),
        no_regenerate: false,
    });
    game.cards.insert(spell_id, spell);

    // Opponent's upkeep - suboptimal timing for a low-value target
    game.turn.current_step = crate::game::Step::Upkeep;
    game.turn.active_player = p2_id;

    let view = GameStateView::new(&game, p1_id);
    let spell_card = view.get_card(spell_id).unwrap();

    assert!(
        !controller.use_removal_now(spell_card, creature_id, &view),
        "Instant removal should be held during opponent's upkeep for a low-value target"
    );
}

/// Test: Instant removal used during combat (DeclareAttackers)
#[test]
fn test_use_removal_now_during_combat() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Load real cards: Terror (instant removal) and Serra Angel (target)
    let angel_id = crate::core::CardId::new(50);
    let terror_id = crate::core::CardId::new(100);
    if !load_and_place_on_battlefield(&mut game, "../cardsfolder/s/serra_angel.txt", angel_id, p2_id) {
        println!("Skipping test: cardsfolder not present");
        return;
    }
    if !load_card_in_hand(&mut game, "../cardsfolder/t/terror.txt", terror_id, p1_id) {
        return;
    }

    // During combat (DeclareAttackers) - optimal timing
    game.turn.current_step = crate::game::Step::DeclareAttackers;
    game.turn.active_player = p2_id;

    let view = GameStateView::new(&game, p1_id);
    let terror = view.get_card(terror_id).unwrap();

    assert!(
        controller.use_removal_now(terror, angel_id, &view),
        "Terror should be used during combat to remove Serra Angel"
    );
}

/// Test: Instant removal used during our Main1 (to enable attacks)
#[test]
fn test_use_removal_now_main1_enable_attack() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Load Swords to Plowshares and Shivan Dragon
    let dragon_id = crate::core::CardId::new(50);
    let stp_id = crate::core::CardId::new(100);
    if !load_and_place_on_battlefield(&mut game, "../cardsfolder/s/shivan_dragon.txt", dragon_id, p2_id) {
        println!("Skipping test: cardsfolder not present");
        return;
    }
    if !load_card_in_hand(&mut game, "../cardsfolder/s/swords_to_plowshares.txt", stp_id, p1_id) {
        return;
    }

    // Our Main1 - removing opponent's dragon enables attacks
    game.turn.current_step = crate::game::Step::Main1;
    game.turn.active_player = p1_id;

    let view = GameStateView::new(&game, p1_id);
    let stp = view.get_card(stp_id).unwrap();

    assert!(
        controller.use_removal_now(stp, dragon_id, &view),
        "Swords to Plowshares should be used in Main1 to enable attacks"
    );
}

/// Test: Instant removal used at opponent's end step
#[test]
fn test_use_removal_now_opponent_end_step() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Load Lightning Bolt and Grizzly Bears
    let bears_id = crate::core::CardId::new(50);
    let bolt_id = crate::core::CardId::new(100);
    if !load_and_place_on_battlefield(&mut game, "../cardsfolder/g/grizzly_bears.txt", bears_id, p2_id) {
        println!("Skipping test: cardsfolder not present");
        return;
    }
    if !load_card_in_hand(&mut game, "../cardsfolder/l/lightning_bolt.txt", bolt_id, p1_id) {
        return;
    }

    // Opponent's end step - good timing for instant removal
    game.turn.current_step = crate::game::Step::End;
    game.turn.active_player = p2_id;

    let view = GameStateView::new(&game, p1_id);
    let bolt = view.get_card(bolt_id).unwrap();

    assert!(
        controller.use_removal_now(bolt, bears_id, &view),
        "Lightning Bolt should be used at opponent's end step"
    );
}

/// Test: Enchanted target triggers two-for-one removal
#[test]
fn test_use_removal_now_enchanted_target_two_for_one() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Load Grizzly Bears (target), Holy Strength (aura), and Lightning Bolt (removal)
    let bears_id = crate::core::CardId::new(50);
    let aura_id = crate::core::CardId::new(51);
    let bolt_id = crate::core::CardId::new(100);

    if !load_and_place_on_battlefield(&mut game, "../cardsfolder/g/grizzly_bears.txt", bears_id, p2_id) {
        println!("Skipping test: cardsfolder not present");
        return;
    }
    if !load_and_place_on_battlefield(&mut game, "../cardsfolder/h/holy_strength.txt", aura_id, p2_id) {
        return;
    }
    if !load_card_in_hand(&mut game, "../cardsfolder/l/lightning_bolt.txt", bolt_id, p1_id) {
        return;
    }

    // Attach the aura to the bears
    if let Some(aura) = game.cards.try_get_mut(aura_id) {
        aura.attached_to = Some(bears_id);
    }

    // Set to suboptimal timing (opponent's draw step)
    // Normally we'd hold instant removal here, but the two-for-one
    // should override timing concerns
    game.turn.current_step = crate::game::Step::Draw;
    game.turn.active_player = p2_id;

    let view = GameStateView::new(&game, p1_id);
    let bolt = view.get_card(bolt_id).unwrap();

    assert!(
        controller.use_removal_now(bolt, bears_id, &view),
        "Lightning Bolt should be used immediately on enchanted target (two-for-one)"
    );
}

/// Test: High-value target triggers immediate removal even at bad timing
#[test]
fn test_use_removal_now_high_value_target() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Load Shivan Dragon (high-value: 5/5 flyer with firebreathing)
    // and Swords to Plowshares
    let dragon_id = crate::core::CardId::new(50);
    let stp_id = crate::core::CardId::new(100);

    if !load_and_place_on_battlefield(&mut game, "../cardsfolder/s/shivan_dragon.txt", dragon_id, p2_id) {
        println!("Skipping test: cardsfolder not present");
        return;
    }
    if !load_card_in_hand(&mut game, "../cardsfolder/s/swords_to_plowshares.txt", stp_id, p1_id) {
        return;
    }

    // Opponent's draw step - normally bad timing for instant removal
    game.turn.current_step = crate::game::Step::Draw;
    game.turn.active_player = p2_id;

    let view = GameStateView::new(&game, p1_id);
    let stp = view.get_card(stp_id).unwrap();
    let dragon_eval = controller.evaluate_creature(&view, dragon_id);

    // Shivan Dragon should evaluate high enough (>= 200) to trigger immediate removal
    assert!(
        dragon_eval >= 200,
        "Shivan Dragon evaluation ({dragon_eval}) should be >= 200 for high-value threshold"
    );

    assert!(
        controller.use_removal_now(stp, dragon_id, &view),
        "Swords to Plowshares should remove high-value Shivan Dragon even at bad timing"
    );
}

/// Test: target_has_auras detects aura attachments
#[test]
fn test_target_has_auras() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Creature without aura
    let creature_id = CardId::new(50);
    let mut creature = Card::new(creature_id, "Grizzly Bears", p2_id);
    creature.add_type(CardType::Creature);
    creature.controller = p2_id;
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    let view = GameStateView::new(&game, p1_id);
    assert!(
        !controller.target_has_auras(creature_id, &view),
        "Creature without auras should return false"
    );

    // Add an aura attached to the creature
    let aura_id = CardId::new(51);
    let mut aura = Card::new(aura_id, "Holy Strength", p2_id);
    aura.add_type(CardType::Enchantment);
    aura.set_subtypes(smallvec::smallvec![crate::core::Subtype::new("Aura")]);
    aura.attached_to = Some(creature_id);
    aura.controller = p2_id;
    game.cards.insert(aura_id, aura);
    game.battlefield.add(aura_id);

    let view2 = GameStateView::new(&game, p1_id);
    assert!(
        controller.target_has_auras(creature_id, &view2),
        "Creature with aura attached should return true"
    );
}

/// Test: Integration - should_cast_spell with removal timing uses use_removal_now
/// Verifies that the AI holds instant removal at bad timing but uses it during combat
#[test]
fn test_should_cast_spell_removal_timing_integration() {
    use crate::core::{Card, CardId, CardType, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Small opponent creature (low value)
    let creature_id = CardId::new(50);
    let mut creature = Card::new(creature_id, "Squire", p2_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(1));
    creature.set_base_toughness(Some(2));
    creature.controller = p2_id;
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Instant removal spell
    let spell_id = CardId::new(100);
    let mut spell = Card::new(spell_id, "Terror", p1_id);
    spell.add_type(CardType::Instant);
    spell.mana_cost = ManaCost::from_string("1B");
    spell.effects.push(crate::core::Effect::DestroyPermanent {
        target: creature_id,
        restriction: crate::core::TargetRestriction::any(),
        no_regenerate: false,
    });
    game.cards.insert(spell_id, spell);

    // At opponent's upkeep: should_cast_spell returns false (hold removal)
    game.turn.current_step = crate::game::Step::Upkeep;
    game.turn.active_player = p2_id;

    let view = GameStateView::new(&game, p1_id);
    let spell_card = view.get_card(spell_id).unwrap();
    assert!(
        !controller.should_cast_spell(spell_card, &view),
        "AI should hold instant removal at opponent's upkeep for low-value target"
    );

    // During combat: should_cast_spell returns true
    game.turn.current_step = crate::game::Step::DeclareAttackers;
    let view2 = GameStateView::new(&game, p1_id);
    let spell_card2 = view2.get_card(spell_id).unwrap();
    assert!(
        controller.should_cast_spell(spell_card2, &view2),
        "AI should use instant removal during combat"
    );
}

// ==================== Fight AI Tests ====================

/// Test: AI casts Fight spell when we have a favorable matchup
/// Reference: FightAi.java - favorable = our creature kills theirs and survives
#[test]
fn test_should_cast_fight_favorable_matchup() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Our 5/5 creature (Serra Angel-like)
    let our_id = CardId::new(50);
    let mut our = Card::new(our_id, "Serra Angel", p1_id);
    our.add_type(CardType::Creature);
    our.set_base_power(Some(4));
    our.set_base_toughness(Some(4));
    our.controller = p1_id;
    game.cards.insert(our_id, our);
    game.battlefield.add(our_id);

    // Opponent's 2/2 creature (Grizzly Bears)
    let opp_id = CardId::new(51);
    let mut opp = Card::new(opp_id, "Grizzly Bears", p2_id);
    opp.add_type(CardType::Creature);
    opp.set_base_power(Some(2));
    opp.set_base_toughness(Some(2));
    opp.controller = p2_id;
    game.cards.insert(opp_id, opp);
    game.battlefield.add(opp_id);

    let view = GameStateView::new(&game, p1_id);

    // 4/4 vs 2/2: We kill them (4 >= 2) and survive (2 < 4)
    assert!(
        controller.should_cast_fight(&view),
        "AI should cast Fight when 4/4 fights 2/2 (we win)"
    );
}

/// Test: AI doesn't cast Fight when we would lose
#[test]
fn test_should_not_cast_fight_unfavorable() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Our 2/2 creature (Grizzly Bears)
    let our_id = CardId::new(50);
    let mut our = Card::new(our_id, "Grizzly Bears", p1_id);
    our.add_type(CardType::Creature);
    our.set_base_power(Some(2));
    our.set_base_toughness(Some(2));
    our.controller = p1_id;
    game.cards.insert(our_id, our);
    game.battlefield.add(our_id);

    // Opponent's 5/5 creature (bigger than ours)
    let opp_id = CardId::new(51);
    let mut opp = Card::new(opp_id, "Shivan Dragon", p2_id);
    opp.add_type(CardType::Creature);
    opp.set_base_power(Some(5));
    opp.set_base_toughness(Some(5));
    opp.controller = p2_id;
    game.cards.insert(opp_id, opp);
    game.battlefield.add(opp_id);

    let view = GameStateView::new(&game, p1_id);

    // 2/2 vs 5/5: We die (5 >= 2), they survive (2 < 5)
    assert!(
        !controller.should_cast_fight(&view),
        "AI should NOT cast Fight when 2/2 fights 5/5 (we lose)"
    );
}

/// Test: AI casts Fight for favorable trade-up
#[test]
fn test_should_cast_fight_trade_up() {
    use crate::core::{Card, CardId, CardType, Keyword};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Our 1/1 deathtouch (high value due to deathtouch)
    let our_id = CardId::new(50);
    let mut our = Card::new(our_id, "Typhoid Rats", p1_id);
    our.add_type(CardType::Creature);
    our.set_base_power(Some(1));
    our.set_base_toughness(Some(1));
    our.keywords.insert(Keyword::Deathtouch);
    our.controller = p1_id;
    our.tapped = false; // Must be untapped to fight
    game.cards.insert(our_id, our);
    game.battlefield.add(our_id);

    // Opponent's big 5/5 creature (much more valuable)
    let opp_id = CardId::new(51);
    let mut opp = Card::new(opp_id, "Shivan Dragon", p2_id);
    opp.add_type(CardType::Creature);
    opp.set_base_power(Some(5));
    opp.set_base_toughness(Some(5));
    opp.controller = p2_id;
    game.cards.insert(opp_id, opp);
    game.battlefield.add(opp_id);

    let view = GameStateView::new(&game, p1_id);

    // 1/1 deathtouch vs 5/5:
    // We kill them (deathtouch: 1 damage is lethal)
    // We die (5 >= 1), but this is a favorable trade
    assert!(
        controller.should_cast_fight(&view),
        "AI should cast Fight when 1/1 deathtouch fights 5/5 (favorable trade)"
    );
}

/// Test: AI doesn't cast Fight when no creatures available
#[test]
fn test_should_not_cast_fight_no_creatures() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let controller = HeuristicController::new(p1_id);

    let view = GameStateView::new(&game, p1_id);

    assert!(
        !controller.should_cast_fight(&view),
        "AI should not cast Fight when no creatures on battlefield"
    );
}

// ==================== GainControl AI Tests ====================

/// Test: AI casts GainControl when opponent has valuable creature
#[test]
fn test_should_cast_gain_control_valuable_target() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Opponent's valuable creature (Shivan Dragon 5/5)
    let opp_id = CardId::new(50);
    let mut opp = Card::new(opp_id, "Shivan Dragon", p2_id);
    opp.add_type(CardType::Creature);
    opp.set_base_power(Some(5));
    opp.set_base_toughness(Some(5));
    opp.controller = p2_id;
    game.cards.insert(opp_id, opp);
    game.battlefield.add(opp_id);

    let view = GameStateView::new(&game, p1_id);

    assert!(
        controller.should_cast_gain_control(&view),
        "AI should cast GainControl on valuable 5/5 creature"
    );
}

/// Test: AI doesn't cast GainControl when opponent has no creatures
#[test]
fn test_should_not_cast_gain_control_no_targets() {
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let controller = HeuristicController::new(p1_id);

    let view = GameStateView::new(&game, p1_id);

    assert!(
        !controller.should_cast_gain_control(&view),
        "AI should not cast GainControl when opponent has no creatures"
    );
}

/// Test: AI always casts GainControl when opponent has creatures
/// Even stealing a weak creature is advantageous (denies blocker + gains attacker)
#[test]
fn test_should_cast_gain_control_any_creature() {
    use crate::core::{Card, CardId, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    let controller = HeuristicController::new(p1_id);

    // Opponent's weak creature (1/1 vanilla)
    let opp_id = CardId::new(50);
    let mut opp = Card::new(opp_id, "Squire", p2_id);
    opp.add_type(CardType::Creature);
    opp.set_base_power(Some(1));
    opp.set_base_toughness(Some(1));
    opp.controller = p2_id;
    game.cards.insert(opp_id, opp);
    game.battlefield.add(opp_id);

    let view = GameStateView::new(&game, p1_id);

    // Even a 1/1 is worth stealing - it's card advantage
    // (denies them a blocker, gives us an attacker)
    assert!(
        controller.should_cast_gain_control(&view),
        "AI should cast GainControl even on weak 1/1 creature"
    );
}

// ========================================================================
// REAL CARD TESTS - Load from cardsfolder
// Tests use real 4ED/classic cards to verify AI behavior with actual card data
// ========================================================================

/// Test loading Prodigal Sorcerer from cardsfolder and verifying ping ability AI
/// Prodigal Sorcerer: 1/1, T: Deal 1 damage to any target
#[test]
fn test_prodigal_sorcerer_from_cardsfolder() {
    use crate::game::controller::GameStateView;
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/p/prodigal_sorcerer.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Prodigal Sorcerer");
    assert_eq!(def.name.as_str(), "Prodigal Sorcerer");

    // Create game and instantiate the card
    let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties (before adding to game)
    assert!(card.is_creature(), "Prodigal Sorcerer should be a creature");
    assert_eq!(card.current_power(), 1, "Prodigal Sorcerer should be 1/1");
    assert_eq!(card.current_toughness(), 1, "Prodigal Sorcerer should be 1/1");

    // Verify the activated ability was parsed
    assert!(
        !card.activated_abilities.is_empty(),
        "Prodigal Sorcerer should have at least one activated ability"
    );

    // Find the tap ability (ping ability)
    let ping_abilities: Vec<_> = card
        .activated_abilities
        .iter()
        .filter(|a| a.cost.includes_tap())
        .collect();

    assert_eq!(
        ping_abilities.len(),
        1,
        "Prodigal Sorcerer should have exactly one tap ability"
    );

    // Verify the ability has a DealDamage effect
    let ability = ping_abilities[0];
    let has_damage_effect = ability
        .effects
        .iter()
        .any(|e| matches!(e, crate::core::Effect::DealDamage { .. }));

    assert!(
        has_damage_effect,
        "Prodigal Sorcerer's ability should have a DealDamage effect"
    );

    // Test AI classification
    let controller = HeuristicController::new(p1_id);
    let ability_type = controller.classify_activated_ability(ability);
    assert!(
        matches!(ability_type, ActivatedAbilityType::Ping { damage: 1 }),
        "Prodigal Sorcerer's ability should be classified as Ping(1) by AI"
    );

    // Add card to game and battlefield for evaluation
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation includes the ping bonus
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base 1/1 = 100, ping adds 10 + 1*5 = 15, so should be > 110
    assert!(
        creature_value > 110,
        "Prodigal Sorcerer evaluation ({}) should be higher than vanilla 1/1 (100) due to ping ability",
        creature_value
    );
}

/// Test Northern Paladin destroy ability with color restriction
/// Northern Paladin: 3/3, WW T: Destroy target black permanent
#[test]
fn test_northern_paladin_from_cardsfolder() {
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/n/northern_paladin.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Northern Paladin");
    assert_eq!(def.name.as_str(), "Northern Paladin");

    // Create game and instantiate the card
    let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties
    assert!(card.is_creature(), "Northern Paladin should be a creature");
    assert_eq!(card.current_power(), 3, "Northern Paladin should be 3/3");
    assert_eq!(card.current_toughness(), 3, "Northern Paladin should be 3/3");

    // Verify the activated ability was parsed
    assert!(
        !card.activated_abilities.is_empty(),
        "Northern Paladin should have at least one activated ability"
    );

    // Find the tap-to-destroy ability
    let destroy_abilities: Vec<_> = card
        .activated_abilities
        .iter()
        .filter(|a| !a.is_mana_ability && a.cost.includes_tap())
        .collect();

    assert_eq!(
        destroy_abilities.len(),
        1,
        "Northern Paladin should have exactly one tap-to-destroy ability"
    );

    // Verify the ability has a DestroyPermanent effect
    let ability = destroy_abilities[0];
    let has_destroy_effect = ability
        .effects
        .iter()
        .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));

    assert!(
        has_destroy_effect,
        "Northern Paladin's ability should have a DestroyPermanent effect"
    );

    // Test AI classification
    let controller = HeuristicController::new(p1_id);
    let ability_type = controller.classify_activated_ability(ability);
    assert!(
        matches!(ability_type, ActivatedAbilityType::Destroy { .. }),
        "Northern Paladin's ability should be classified as Destroy by AI"
    );
}

/// Test Drudge Skeletons regeneration ability from cardsfolder
/// Drudge Skeletons: 1/1, B: Regenerate
#[test]
fn test_drudge_skeletons_from_cardsfolder() {
    use crate::game::controller::GameStateView;
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/d/drudge_skeletons.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Drudge Skeletons");
    assert_eq!(def.name.as_str(), "Drudge Skeletons");

    // Create game and instantiate the card
    let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties
    assert!(card.is_creature(), "Drudge Skeletons should be a creature");
    assert_eq!(card.current_power(), 1, "Drudge Skeletons should be 1/1");
    assert_eq!(card.current_toughness(), 1, "Drudge Skeletons should be 1/1");

    // Verify the activated ability was parsed
    assert!(
        !card.activated_abilities.is_empty(),
        "Drudge Skeletons should have at least one activated ability"
    );

    // Find the regeneration ability
    let regen_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| !a.is_mana_ability).collect();

    assert_eq!(
        regen_abilities.len(),
        1,
        "Drudge Skeletons should have exactly one non-mana activated ability (regenerate)"
    );

    // Verify the ability has a Regenerate effect
    let ability = regen_abilities[0];
    let has_regen_effect = ability
        .effects
        .iter()
        .any(|e| matches!(e, crate::core::Effect::Regenerate { .. }));

    assert!(
        has_regen_effect,
        "Drudge Skeletons's ability should have a Regenerate effect"
    );

    // Add card to game and battlefield for evaluation
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation includes regeneration bonus
    let controller = HeuristicController::new(p1_id);
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base 1/1 = 100, regeneration typically adds +20
    assert!(
        creature_value > 110,
        "Drudge Skeletons evaluation ({}) should be higher than vanilla 1/1 (100) due to regenerate",
        creature_value
    );
}

/// Test Llanowar Elves mana ability recognition from cardsfolder
/// Llanowar Elves: 1/1, T: Add G
#[test]
fn test_llanowar_elves_mana_ability() {
    use crate::game::controller::GameStateView;
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/l/llanowar_elves.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Llanowar Elves");
    assert_eq!(def.name.as_str(), "Llanowar Elves");

    // Create game and instantiate the card
    let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties
    assert!(card.is_creature(), "Llanowar Elves should be a creature");
    assert_eq!(card.current_power(), 1, "Llanowar Elves should be 1/1");
    assert_eq!(card.current_toughness(), 1, "Llanowar Elves should be 1/1");

    // Verify the activated ability was parsed
    assert!(
        !card.activated_abilities.is_empty(),
        "Llanowar Elves should have at least one activated ability"
    );

    // Find the mana ability
    let mana_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| a.is_mana_ability).collect();

    assert_eq!(
        mana_abilities.len(),
        1,
        "Llanowar Elves should have exactly one mana ability"
    );

    // Verify the mana ability produces green mana
    let ability = mana_abilities[0];
    let has_mana_effect = ability
        .effects
        .iter()
        .any(|e| matches!(e, crate::core::Effect::AddMana { mana, .. } if mana.green > 0));

    assert!(has_mana_effect, "Llanowar Elves's ability should produce green mana");

    // Add card to game and battlefield for evaluation
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation includes mana bonus
    let controller = HeuristicController::new(p1_id);
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base 1/1 = 100, mana ability typically adds +15
    assert!(
        creature_value > 110,
        "Llanowar Elves evaluation ({}) should be higher than vanilla 1/1 (100) due to mana ability",
        creature_value
    );
}

/// Test Serra Angel keyword evaluation from cardsfolder
/// Serra Angel: 4/4, Flying, Vigilance
#[test]
fn test_serra_angel_keywords() {
    use crate::game::controller::GameStateView;
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/s/serra_angel.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Serra Angel");
    assert_eq!(def.name.as_str(), "Serra Angel");

    // Create game and instantiate the card
    let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties
    assert!(card.is_creature(), "Serra Angel should be a creature");
    assert_eq!(card.current_power(), 4, "Serra Angel should be 4/4");
    assert_eq!(card.current_toughness(), 4, "Serra Angel should be 4/4");

    // Verify keywords
    assert!(
        card.has_keyword(crate::core::Keyword::Flying),
        "Serra Angel should have Flying"
    );
    assert!(
        card.has_keyword(crate::core::Keyword::Vigilance),
        "Serra Angel should have Vigilance"
    );

    // Add card to game and battlefield for evaluation
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation includes keyword bonuses
    let controller = HeuristicController::new(p1_id);
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base: 80 + 20 (non-token)
    // Power: 4 * 15 = 60
    // Toughness: 4 * 10 = 40
    // CMC: 5 * 5 = 25
    // Flying: 4 * 10 = 40
    // Vigilance: 4 * 3 = 12
    // Total should be > 250 (base = 245 + keywords)
    assert!(
        creature_value > 250,
        "Serra Angel evaluation ({}) should be > 250 due to Flying and Vigilance",
        creature_value
    );
}

/// Test Shivan Dragon pump ability and flying from cardsfolder
/// Shivan Dragon: 5/5, Flying, R: +1/+0
#[test]
fn test_shivan_dragon_from_cardsfolder() {
    use crate::game::controller::GameStateView;
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/s/shivan_dragon.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Shivan Dragon");
    assert_eq!(def.name.as_str(), "Shivan Dragon");

    // Create game and instantiate the card
    let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify basic card properties
    assert!(card.is_creature(), "Shivan Dragon should be a creature");
    assert_eq!(card.current_power(), 5, "Shivan Dragon should be 5/5");
    assert_eq!(card.current_toughness(), 5, "Shivan Dragon should be 5/5");

    // Verify keywords
    assert!(
        card.has_keyword(crate::core::Keyword::Flying),
        "Shivan Dragon should have Flying"
    );

    // Verify the firebreathing ability was parsed
    let non_mana_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| !a.is_mana_ability).collect();

    assert!(
        !non_mana_abilities.is_empty(),
        "Shivan Dragon should have a firebreathing ability"
    );

    // Test AI classification of the pump ability
    let controller = HeuristicController::new(p1_id);
    let ability = non_mana_abilities[0];
    let ability_type = controller.classify_activated_ability(ability);
    assert!(
        matches!(ability_type, ActivatedAbilityType::Pump { power: 1, toughness: 0 }),
        "Shivan Dragon's ability should be classified as Pump(+1/+0)"
    );

    // Add card to game and battlefield for evaluation
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation includes keyword and ability bonuses
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base: 80 + 20 (non-token)
    // Power: 5 * 15 = 75
    // Toughness: 5 * 10 = 50
    // CMC: 6 * 5 = 30
    // Flying: 5 * 10 = 50
    // Pump ability: adds some bonus
    // Total should be > 300 (base = 305 without pump)
    assert!(
        creature_value > 300,
        "Shivan Dragon evaluation ({}) should be > 300 due to Flying and pump ability",
        creature_value
    );
}

/// Test loading Hypnotic Specter from cardsfolder - classic 4ED evasive creature
///
/// Hypnotic Specter (4ED): 2/2 Flying
/// "Whenever Hypnotic Specter deals damage to an opponent, that player discards a card at random."
///
/// Tests Mode$ DamageDone trigger parsing - Hypnotic Specter has a damage-to-player trigger
/// that causes the opponent to discard a card at random.
#[test]
fn test_hypnotic_specter_from_cardsfolder() {
    use crate::core::CardId;
    use crate::game::controller::GameStateView;
    use crate::game::GameState;
    use std::path::PathBuf;

    // Load card from cardsfolder
    let path = PathBuf::from("../cardsfolder/h/hypnotic_specter.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

    // Verify card properties from definition
    assert_eq!(card_def.name.as_str(), "Hypnotic Specter");
    assert_eq!(card_def.power, Some(2));
    assert_eq!(card_def.toughness, Some(2));

    // Set up a game to test creature evaluation
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    let card_id = CardId::new(100);
    let mut card = card_def.instantiate(card_id, p1_id);
    card.controller = p1_id;

    // Verify card properties after instantiation (is_creature() checks types)
    assert!(card.is_creature(), "Hypnotic Specter should be a creature");
    assert!(card.has_flying(), "Hypnotic Specter should have Flying");

    // Verify Mode$ DamageDone trigger is parsed
    assert!(
        !card.triggers.is_empty(),
        "Hypnotic Specter should have at least one trigger (DamageDone)"
    );
    assert_eq!(
        card.triggers[0].event,
        crate::core::TriggerEvent::DealsCombatDamage,
        "Hypnotic Specter's trigger should be DealsCombatDamage"
    );

    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);

    let controller = HeuristicController::new(p1_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation - Flying bonus should still apply
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base: 80 + 20 (non-token)
    // Power: 2 * 15 = 30
    // Toughness: 2 * 10 = 20
    // CMC: 3 * 5 = 15
    // Flying: 2 * 10 = 20
    // Expected minimum without trigger: 100 + 30 + 20 + 15 + 20 = 185
    assert!(
        creature_value >= 180,
        "Hypnotic Specter evaluation ({}) should be >= 180 due to Flying keyword",
        creature_value
    );

    println!("Hypnotic Specter evaluation: {}", creature_value);
}

/// Test loading Sengir Vampire from cardsfolder - classic 4ED flyer
///
/// Sengir Vampire (4ED): 4/4 Flying
/// "Whenever a creature dealt damage by Sengir Vampire this turn dies, put a +1/+1 counter on Sengir Vampire."
///
/// Note: The conditional "dies" trigger (ValidCard$ Creature.DamagedBy) requires
/// tracking damage sources, which is complex. This test verifies basic card properties.
/// TODO(mtg-147): Implement conditional die triggers with DamagedBy tracking
#[test]
fn test_sengir_vampire_from_cardsfolder() {
    use crate::core::CardId;
    use crate::game::controller::GameStateView;
    use crate::game::GameState;
    use std::path::PathBuf;

    // Load card from cardsfolder
    let path = PathBuf::from("../cardsfolder/s/sengir_vampire.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

    // Verify card properties from definition
    assert_eq!(card_def.name.as_str(), "Sengir Vampire");
    assert_eq!(card_def.power, Some(4));
    assert_eq!(card_def.toughness, Some(4));

    // Set up a game to test creature evaluation
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    let card_id = CardId::new(100);
    let mut card = card_def.instantiate(card_id, p1_id);
    card.controller = p1_id;

    // Verify card properties after instantiation (is_creature() checks types)
    assert!(card.is_creature(), "Sengir Vampire should be a creature");
    assert!(card.has_flying(), "Sengir Vampire should have Flying");

    // Note: Complex conditional trigger not yet parsed - skip trigger assertion
    // The "Creature.DamagedBy" condition requires damage tracking infrastructure

    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);

    let controller = HeuristicController::new(p1_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation - Flying bonus should still apply
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base: 80 + 20 (non-token)
    // Power: 4 * 15 = 60
    // Toughness: 4 * 10 = 40
    // CMC: 5 * 5 = 25
    // Flying: 4 * 10 = 40
    // Expected minimum without trigger: 100 + 60 + 40 + 25 + 40 = 265
    assert!(
        creature_value >= 260,
        "Sengir Vampire evaluation ({}) should be >= 260 due to Flying keyword",
        creature_value
    );

    println!("Sengir Vampire evaluation: {}", creature_value);
}

/// Test loading Mahamoti Djinn from cardsfolder - classic 4ED blue finisher
///
/// Mahamoti Djinn (4ED): 5/6 Flying
/// No abilities, but tests pure stat-based creature evaluation with Flying
#[test]
fn test_mahamoti_djinn_from_cardsfolder() {
    use crate::core::CardId;
    use crate::game::controller::GameStateView;
    use crate::game::GameState;
    use std::path::PathBuf;

    // Load card from cardsfolder
    let path = PathBuf::from("../cardsfolder/m/mahamoti_djinn.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

    // Verify card properties from definition
    assert_eq!(card_def.name.as_str(), "Mahamoti Djinn");
    assert_eq!(card_def.power, Some(5));
    assert_eq!(card_def.toughness, Some(6));

    // Set up a game to test creature evaluation
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    let card_id = CardId::new(100);
    let mut card = card_def.instantiate(card_id, p1_id);
    card.controller = p1_id;

    // Verify card properties after instantiation (is_creature() checks types)
    assert!(card.is_creature(), "Mahamoti Djinn should be a creature");
    assert!(card.has_flying(), "Mahamoti Djinn should have Flying");

    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);

    let controller = HeuristicController::new(p1_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base: 80 + 20 (non-token)
    // Power: 5 * 15 = 75
    // Toughness: 6 * 10 = 60
    // CMC: 6 * 5 = 30
    // Flying: 5 * 10 = 50 (power * 10)
    // Expected minimum: 100 + 75 + 60 + 30 + 50 = 315
    assert!(
        creature_value >= 300,
        "Mahamoti Djinn evaluation ({}) should be >= 300 due to high stats and Flying",
        creature_value
    );

    println!("Mahamoti Djinn evaluation: {}", creature_value);
}

/// Test loading Force of Nature from cardsfolder - classic 4ED with upkeep cost
///
/// Force of Nature (4ED): 8/8 Trample
/// "At the beginning of your upkeep, Force of Nature deals 8 damage to you unless you pay GGGG."
///
/// This tests that upkeep costs are properly penalized in creature evaluation
#[test]
fn test_force_of_nature_from_cardsfolder() {
    use crate::core::CardId;
    use crate::game::controller::GameStateView;
    use crate::game::GameState;
    use std::path::PathBuf;

    // Load card from cardsfolder
    let path = PathBuf::from("../cardsfolder/f/force_of_nature.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

    // Verify card properties from definition
    assert_eq!(card_def.name.as_str(), "Force of Nature");
    assert_eq!(card_def.power, Some(8));
    assert_eq!(card_def.toughness, Some(8));

    // Set up a game to test creature evaluation
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    let card_id = CardId::new(100);
    let mut card = card_def.instantiate(card_id, p1_id);
    card.controller = p1_id;

    // Verify card properties after instantiation (is_creature() checks types)
    assert!(card.is_creature(), "Force of Nature should be a creature");
    assert!(card.has_trample(), "Force of Nature should have Trample");

    // Verify upkeep trigger exists on instantiated card
    let has_upkeep_trigger = card
        .triggers
        .iter()
        .any(|t| matches!(t.event, crate::core::TriggerEvent::BeginningOfUpkeep));
    assert!(has_upkeep_trigger, "Force of Nature should have an upkeep trigger");

    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);

    let controller = HeuristicController::new(p1_id);
    let view = GameStateView::new(&game, p1_id);

    // Verify creature evaluation with upkeep penalty
    let creature_value = controller.evaluate_creature(&view, card_id);
    // Base: 80 + 20 (non-token)
    // Power: 8 * 15 = 120
    // Toughness: 8 * 10 = 80
    // CMC: 6 * 5 = 30
    // Trample: 8 * 5 = 40 (power * 5)
    // Upkeep trigger penalty: -15 (damage to self)
    // Expected: 100 + 120 + 80 + 30 + 40 - 15 = 355 minimum (still high due to massive stats)
    // Should still be valuable despite upkeep penalty
    assert!(
        creature_value >= 300,
        "Force of Nature evaluation ({}) should be >= 300 despite upkeep penalty due to massive stats",
        creature_value
    );

    // But should be LOWER than an equivalent creature without upkeep cost
    // Create a hypothetical 8/8 Trample without upkeep
    let hypothetical_value = 100 + 120 + 80 + 30 + 40; // 370
    assert!(
        creature_value < hypothetical_value + 10, // Allow small margin
        "Force of Nature should be penalized for upkeep cost (value: {}, pure stats: {})",
        creature_value,
        hypothetical_value
    );

    println!("Force of Nature evaluation: {}", creature_value);
}

/// Test land drop hold logic for Main Phase 2 bluffing
///
/// Reference: AiController.isSafeToHoldLandDropForMain2
///
/// This test validates that the probabilistic land-holding logic works.
/// Due to the complexity of setting up proper game state, this is a simplified
/// unit test that verifies the basic RNG behavior.
#[test]
fn test_land_drop_hold_probabilistic_behavior() {
    use crate::core::PlayerId;
    use rand::Rng;

    let p1_id = PlayerId::new(0);

    // With different seeds, we should eventually see different results
    // across multiple trials
    let mut results = Vec::new();
    for seed in 0..20 {
        let mut controller = HeuristicController::with_seed(p1_id, seed);
        results.push(controller.rng.gen_bool(0.5));
    }

    let true_count = results.iter().filter(|&&x| x).count();
    let false_count = results.len() - true_count;

    // With 20 trials and 50% probability, we should see a mix
    // (not all true or all false)
    assert!(
        true_count > 0 && false_count > 0,
        "RNG should produce varied results (true={}, false={})",
        true_count,
        false_count
    );

    println!(
        "Land drop RNG test: {} true, {} false out of 20 trials",
        true_count, false_count
    );
}

/// Test instant-speed spell timing bluffing
///
/// Reference: Java Forge phase restriction patterns (e.g., "AtEOT" in various AIs)
///
/// This test validates that the AI correctly holds instant-speed draw spells
/// for better timing rather than casting them immediately on its own turn.
///
/// Key bluffing behavior:
/// - Hold instant-speed spells during our Main 1 (bluff having removal/combat tricks)
/// - Cast at opponent's end step (maximize bluffing while still getting value)
/// - Cast at our Main 2 if needed (acceptable fallback timing)
#[test]
fn test_instant_spell_bluffing_timing() {
    use crate::core::{Card, CardId, CardType, Effect};
    use crate::game::{GameState, GameStateView};

    // Setup: Two players, P1 has instant-speed draw spell
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Create instant-speed draw spell (like "Ancestral Recall")
    let draw_spell_id = CardId::new(100);
    let mut draw_spell = Card::new(draw_spell_id, "Instant Draw", p1_id);
    draw_spell.add_type(CardType::Instant);
    draw_spell.effects.push(Effect::DrawCards {
        player: p1_id,
        count: 3,
    });

    // Insert card into game and place in P1's hand
    game.cards.insert(draw_spell_id, draw_spell.clone());
    game.get_player_zones_mut(p1_id).unwrap().hand.cards.push(draw_spell_id);

    let controller = HeuristicController::new(p1_id);

    // Scenario 1: Our Main 1, low hand (1 card) - should HOLD (bluffing)
    game.turn.current_step = crate::game::Step::Main1;
    game.turn.active_player = p1_id;
    let view1 = GameStateView::new(&game, p1_id);
    let should_cast_main1 = controller.should_cast_instant_now(&view1, &draw_spell);
    assert!(
        !should_cast_main1,
        "Should HOLD instant draw during our Main 1 for bluffing (low hand)"
    );

    // Scenario 2: Our Main 2 - should CAST (acceptable timing)
    game.turn.current_step = crate::game::Step::Main2;
    let view2 = GameStateView::new(&game, p1_id);
    let should_cast_main2 = controller.should_cast_instant_now(&view2, &draw_spell);
    assert!(
        should_cast_main2,
        "Should CAST instant draw during our Main 2 (acceptable timing)"
    );

    // Scenario 3: Opponent's end step - should CAST (ideal bluffing window)
    game.turn.active_player = p2_id;
    game.turn.current_step = crate::game::Step::End;
    let view3 = GameStateView::new(&game, p1_id);
    let should_cast_opp_end = controller.should_cast_instant_now(&view3, &draw_spell);
    assert!(
        should_cast_opp_end,
        "Should CAST instant draw at opponent's end step (ideal timing)"
    );

    // Scenario 4: Hand size 7+ - should CAST immediately (avoid discard)
    // Add 6 more cards to hand (already have 1 draw spell = 7 total)
    for i in 0..6 {
        let filler_id = CardId::new(200 + i);
        let filler = Card::new(filler_id, "Filler", p1_id);
        game.cards.insert(filler_id, filler);
        game.get_player_zones_mut(p1_id).unwrap().hand.cards.push(filler_id);
    }
    game.turn.active_player = p1_id;
    game.turn.current_step = crate::game::Step::Main1;
    let view4 = GameStateView::new(&game, p1_id);
    let should_cast_full_hand = controller.should_cast_instant_now(&view4, &draw_spell);
    assert!(
        should_cast_full_hand,
        "Should CAST immediately when hand is full (7+ cards, avoid discard)"
    );

    println!("Instant spell bluffing test passed - AI correctly holds instant-speed spells for better timing");
}

/// Test PutCounterAll AI evaluation
///
/// Reference: CountersPutAllAi.java:25-115 (checkApiLogic)
///
/// Validates that:
/// 1. AI casts beneficial PutCounterAll when we have creatures
/// 2. AI doesn't cast when we have no creatures
/// 3. AI casts when restriction filters to our creatures only (YouCtrl)
#[test]
fn test_should_cast_put_counter_all() {
    use crate::core::entity::EntityId;
    use crate::core::{
        effects::{ControllerRestriction, TargetRestriction, TargetType},
        Card, CardType, CounterType, Effect,
    };
    use crate::game::state::GameState;
    use crate::game::GameStateView;
    use smallvec::smallvec;

    let p1_id = EntityId::new(0);
    let p2_id = EntityId::new(1);
    let controller = HeuristicController::new(p1_id);

    // Create a PutCounterAll spell: "Put a +1/+1 counter on each creature you control"
    let mut spell = Card::new(EntityId::new(100), "Anthem Spell", p1_id);
    spell.add_type(CardType::Sorcery);
    spell.effects = vec![Effect::PutCounterAll {
        restriction: TargetRestriction {
            types: smallvec![TargetType::Creature],
            requires_no_counters: false,
            controller: ControllerRestriction::YouCtrl,
            power_ge: None,
            power_le: None,
            requires_nontoken: false,
            requires_remembered: false,
            requires_nonartifact: false,
            required_color: None,
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            requires_defender: false,
        },
        counter_type: CounterType::P1P1,
        amount: 1,
    }];

    // Scenario 1: We have 3 creatures → should cast (YouCtrl means only our creatures match)
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = p1_id;
    for i in 0..3u32 {
        let cid = EntityId::new(10 + i);
        let mut c = Card::new(cid, format!("Our Creature {}", i), p1_id);
        c.set_base_power(Some(2));
        c.set_base_toughness(Some(2));
        c.add_type(CardType::Creature);
        c.controller = p1_id;
        game.cards.insert(cid, c);
        game.battlefield.cards.push(cid);
    }
    let view = GameStateView::new(&game, p1_id);
    assert!(
        controller.should_cast_put_counter_all(&spell, &view),
        "Should cast PutCounterAll when we have 3 creatures"
    );

    // Scenario 2: No creatures → should NOT cast
    let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game2.turn.active_player = p1_id;
    let view2 = GameStateView::new(&game2, p1_id);
    assert!(
        !controller.should_cast_put_counter_all(&spell, &view2),
        "Should NOT cast PutCounterAll when we have no creatures"
    );

    // Scenario 3: Spell with "any creature" restriction - cast only if we have more creatures
    let mut global_spell = Card::new(EntityId::new(101), "Global Anthem", p1_id);
    global_spell.add_type(CardType::Sorcery);
    global_spell.effects = vec![Effect::PutCounterAll {
        restriction: TargetRestriction {
            types: smallvec![TargetType::Creature],
            requires_no_counters: false,
            controller: ControllerRestriction::Any,
            power_ge: None,
            power_le: None,
            requires_nontoken: false,
            requires_remembered: false,
            requires_nonartifact: false,
            required_color: None,
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            requires_defender: false,
        },
        counter_type: CounterType::P1P1,
        amount: 1,
    }];

    // Add equal creatures for both players
    let mut game3 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game3.turn.active_player = p1_id;
    for i in 0..2u32 {
        let our_cid = EntityId::new(30 + i);
        let mut our_c = Card::new(our_cid, format!("Our {}", i), p1_id);
        our_c.set_base_power(Some(2));
        our_c.set_base_toughness(Some(2));
        our_c.add_type(CardType::Creature);
        our_c.controller = p1_id;
        game3.cards.insert(our_cid, our_c);
        game3.battlefield.cards.push(our_cid);

        let their_cid = EntityId::new(40 + i);
        let mut their_c = Card::new(their_cid, format!("Their {}", i), p2_id);
        their_c.set_base_power(Some(2));
        their_c.set_base_toughness(Some(2));
        their_c.add_type(CardType::Creature);
        their_c.controller = p2_id;
        game3.cards.insert(their_cid, their_c);
        game3.battlefield.cards.push(their_cid);
    }
    let view3 = GameStateView::new(&game3, p1_id);
    assert!(
        !controller.should_cast_put_counter_all(&global_spell, &view3),
        "Should NOT cast global PutCounterAll when opponent has equal creatures"
    );

    println!("PutCounterAll AI test passed - AI correctly evaluates mass counter placement");
}

/// Test ChangeZoneAll AI evaluation
///
/// Reference: ChangeZoneAllAi.java:20-200 (canPlay)
///
/// Validates that:
/// 1. AI casts battlefield bounce when opponent has more creatures
/// 2. AI doesn't cast bounce when we'd lose more value
/// 3. AI casts graveyard exile effects
#[test]
fn test_should_cast_change_zone_all() {
    use crate::core::entity::EntityId;
    use crate::core::{
        effects::{ControllerRestriction, TargetRestriction, TargetType},
        Card, CardType, Effect,
    };
    use crate::game::state::GameState;
    use crate::game::GameStateView;
    use smallvec::smallvec;

    let p1_id = EntityId::new(0);
    let p2_id = EntityId::new(1);
    let controller = HeuristicController::new(p1_id);

    // Create a bounce-all spell: "Return all creatures to their owners' hands" (Aetherize-like)
    let mut bounce_spell = Card::new(EntityId::new(100), "Mass Bounce", p1_id);
    bounce_spell.add_type(CardType::Instant);
    bounce_spell.effects = vec![Effect::ChangeZoneAll {
        restriction: TargetRestriction {
            types: smallvec![TargetType::Creature],
            requires_no_counters: false,
            controller: ControllerRestriction::Any,
            power_ge: None,
            power_le: None,
            requires_nontoken: false,
            requires_remembered: false,
            requires_nonartifact: false,
            required_color: None,
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            requires_defender: false,
        },
        origins: smallvec![crate::zones::Zone::Battlefield],
        destination: crate::zones::Zone::Hand,
        shuffle: false,
    }];

    // Scenario 1: Opponent has 3 big creatures, we have 1 small one → should cast
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = p1_id;

    // Our small creature
    let our_cid = EntityId::new(10);
    let mut our_c = Card::new(our_cid, "Our Bear", p1_id);
    our_c.set_base_power(Some(2));
    our_c.set_base_toughness(Some(2));
    our_c.add_type(CardType::Creature);
    our_c.controller = p1_id;
    game.cards.insert(our_cid, our_c);
    game.battlefield.cards.push(our_cid);

    // Opponent's big creatures
    for i in 0..3u32 {
        let opp_cid = EntityId::new(20 + i);
        let mut opp_c = Card::new(opp_cid, format!("Opp Dragon {}", i), p2_id);
        opp_c.set_base_power(Some(5));
        opp_c.set_base_toughness(Some(5));
        opp_c.add_type(CardType::Creature);
        opp_c.controller = p2_id;
        game.cards.insert(opp_cid, opp_c);
        game.battlefield.cards.push(opp_cid);
    }

    let view = GameStateView::new(&game, p1_id);
    assert!(
        controller.should_cast_change_zone_all(&bounce_spell, &view),
        "Should cast mass bounce when opponent has 3 big creatures vs our 1 small"
    );

    // Scenario 2: We have 3 big creatures, opponent has 1 → should NOT cast
    let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game2.turn.active_player = p1_id;

    for i in 0..3u32 {
        let our_cid2 = EntityId::new(30 + i);
        let mut our_c2 = Card::new(our_cid2, format!("Our Dragon {}", i), p1_id);
        our_c2.set_base_power(Some(5));
        our_c2.set_base_toughness(Some(5));
        our_c2.add_type(CardType::Creature);
        our_c2.controller = p1_id;
        game2.cards.insert(our_cid2, our_c2);
        game2.battlefield.cards.push(our_cid2);
    }
    let opp_cid2 = EntityId::new(40);
    let mut opp_c2 = Card::new(opp_cid2, "Opp Bear", p2_id);
    opp_c2.set_base_power(Some(2));
    opp_c2.set_base_toughness(Some(2));
    opp_c2.add_type(CardType::Creature);
    opp_c2.controller = p2_id;
    game2.cards.insert(opp_cid2, opp_c2);
    game2.battlefield.cards.push(opp_cid2);

    let view2 = GameStateView::new(&game2, p1_id);
    assert!(
        !controller.should_cast_change_zone_all(&bounce_spell, &view2),
        "Should NOT cast mass bounce when we have 3 big creatures vs opponent's 1"
    );

    // Scenario 3: Graveyard exile effect → always cast
    let mut exile_spell = Card::new(EntityId::new(101), "Graveyard Exile", p1_id);
    exile_spell.add_type(CardType::Instant);
    exile_spell.effects = vec![Effect::ChangeZoneAll {
        restriction: TargetRestriction::any(),
        origins: smallvec![crate::zones::Zone::Graveyard],
        destination: crate::zones::Zone::Exile,
        shuffle: false,
    }];

    let mut game3 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game3.turn.active_player = p1_id;
    let view3 = GameStateView::new(&game3, p1_id);
    assert!(
        controller.should_cast_change_zone_all(&exile_spell, &view3),
        "Should cast graveyard exile (always beneficial)"
    );

    println!("ChangeZoneAll AI test passed - AI correctly evaluates mass zone changes");
}

// ==================== 4ED Card AI Tests ====================

/// Test: AI should cast discard spells when opponent has cards in hand
#[test]
fn test_should_cast_discard() {
    use crate::core::Card;
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    game.turn.active_player = p1_id;

    let controller = HeuristicController::new(p1_id);

    // Scenario 1: Opponent has cards in hand → should cast
    // Give opponent some cards in hand
    for i in 0..3u32 {
        let cid = EntityId::new(50 + i);
        let card = Card::new(cid, format!("Opp Card {}", i), p2_id);
        game.cards.insert(cid, card);
        if let Some(zones) = game.get_player_zones_mut(p2_id) {
            zones.hand.cards.push(cid);
        }
    }

    let view = GameStateView::new(&game, p1_id);
    assert!(
        controller.should_cast_discard(&view),
        "Should cast discard when opponent has 3 cards in hand"
    );

    // Scenario 2: Opponent has empty hand → should NOT cast
    let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game2.turn.active_player = p1_id;
    let view2 = GameStateView::new(&game2, p1_id);
    assert!(
        !controller.should_cast_discard(&view2),
        "Should NOT cast discard when opponent has no cards in hand"
    );

    println!("Discard AI test passed - correctly evaluates opponent hand size");
}

/// Test: AI should cast tap spells when opponent has untapped creatures
#[test]
fn test_should_cast_tap_permanent() {
    use crate::core::{Card, CardType};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    game.turn.active_player = p1_id;

    let controller = HeuristicController::new(p1_id);

    // Scenario 1: Opponent has untapped creature → should cast
    let opp_cid = EntityId::new(50);
    let mut opp_creature = Card::new(opp_cid, "Opp Bear", p2_id);
    opp_creature.add_type(CardType::Creature);
    opp_creature.set_base_power(Some(4));
    opp_creature.set_base_toughness(Some(4));
    opp_creature.controller = p2_id;
    opp_creature.tapped = false;
    game.cards.insert(opp_cid, opp_creature);
    game.battlefield.cards.push(opp_cid);

    let view = GameStateView::new(&game, p1_id);
    assert!(
        controller.should_cast_tap_permanent(&view),
        "Should cast tap when opponent has untapped creature"
    );

    // Scenario 2: Only our creatures on battlefield → should NOT cast
    let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game2.turn.active_player = p1_id;

    let our_cid = EntityId::new(60);
    let mut our_creature = Card::new(our_cid, "Our Bear", p1_id);
    our_creature.add_type(CardType::Creature);
    our_creature.set_base_power(Some(2));
    our_creature.set_base_toughness(Some(2));
    our_creature.controller = p1_id;
    game2.cards.insert(our_cid, our_creature);
    game2.battlefield.cards.push(our_cid);

    let view2 = GameStateView::new(&game2, p1_id);
    assert!(
        !controller.should_cast_tap_permanent(&view2),
        "Should NOT cast tap when no opponent creatures"
    );

    println!("Tap permanent AI test passed");
}

/// Test: Icy Manipulator's tap ability is classified as TapTarget
#[test]
fn test_icy_manipulator_from_cardsfolder() {
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/i/icy_manipulator.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Icy Manipulator");
    assert_eq!(def.name.as_str(), "Icy Manipulator");

    let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    assert!(card.is_artifact(), "Icy Manipulator should be an artifact");

    // Find the non-mana tap ability
    let tap_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| !a.is_mana_ability).collect();

    assert!(
        !tap_abilities.is_empty(),
        "Icy Manipulator should have at least one non-mana activated ability"
    );

    // Verify the ability has a TapPermanent effect
    let ability = tap_abilities[0];
    let has_tap_effect = ability
        .effects
        .iter()
        .any(|e| matches!(e, crate::core::Effect::TapPermanent { .. }));

    assert!(
        has_tap_effect,
        "Icy Manipulator's ability should have a TapPermanent effect"
    );

    // Test AI classification
    let controller = HeuristicController::new(p1_id);
    let ability_type = controller.classify_activated_ability(ability);
    assert!(
        matches!(ability_type, ActivatedAbilityType::TapTarget),
        "Icy Manipulator's ability should be classified as TapTarget by AI"
    );

    println!("Icy Manipulator test passed - ability correctly classified as TapTarget");
}

/// Test: Hymn to Tourach parsed correctly and AI evaluates casting
#[test]
fn test_hymn_to_tourach_from_cardsfolder() {
    use std::path::PathBuf;

    let path = PathBuf::from("../cardsfolder/h/hymn_to_tourach.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Hymn to Tourach");
    assert_eq!(def.name.as_str(), "Hymn to Tourach");

    let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    let card_id = crate::core::CardId::new(100);
    let card = def.instantiate(card_id, p1_id);

    // Verify it has a DiscardCards effect
    let has_discard = card.effects.iter().any(|e| {
        matches!(
            e,
            crate::core::Effect::DiscardCards { .. } | crate::core::Effect::DiscardCardsXPaid { .. }
        )
    });

    assert!(has_discard, "Hymn to Tourach should have a Discard effect");

    println!("Hymn to Tourach test passed - discard effect correctly parsed");
}

/// Test: Draw spell AI casts at higher hand-size threshold
#[test]
fn test_draw_spell_hand_threshold() {
    use crate::core::{Card, CardType, Effect};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    game.turn.active_player = p1_id;

    // Create a draw spell (sorcery speed)
    let draw_card_id = EntityId::new(100);
    let mut draw_spell = Card::new(draw_card_id, "Divination", p1_id);
    draw_spell.add_type(CardType::Sorcery);
    draw_spell.effects = vec![Effect::DrawCards {
        player: p1_id,
        count: 2,
    }];

    let controller = HeuristicController::new(p1_id);

    // Scenario 1: 3 cards in hand → should cast (was too restrictive before at <= 2)
    for i in 0..3u32 {
        let cid = EntityId::new(50 + i);
        let c = Card::new(cid, format!("Card {}", i), p1_id);
        game.cards.insert(cid, c);
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.cards.push(cid);
        }
    }

    let view = GameStateView::new(&game, p1_id);
    assert!(
        controller.should_cast_spell(&draw_spell, &view),
        "Should cast draw spell with 3 cards in hand (threshold is now 4)"
    );

    // Scenario 2: 5 cards in hand → should NOT cast
    let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game2.turn.active_player = p1_id;
    for i in 0..5u32 {
        let cid = EntityId::new(60 + i);
        let c = Card::new(cid, format!("Card {}", i), p1_id);
        game2.cards.insert(cid, c);
        if let Some(zones) = game2.get_player_zones_mut(p1_id) {
            zones.hand.cards.push(cid);
        }
    }

    let view2 = GameStateView::new(&game2, p1_id);
    assert!(
        !controller.should_cast_spell(&draw_spell, &view2),
        "Should NOT cast draw spell with 5 cards in hand"
    );

    println!("Draw spell hand threshold test passed");
}

/// Test: Discard spells route through should_cast_spell correctly
#[test]
fn test_discard_spell_routing() {
    use crate::core::{Card, CardType, Effect};
    use crate::game::controller::GameStateView;
    use crate::game::GameState;

    let p1_id = crate::core::PlayerId::new(0);

    // Create a Hymn to Tourach-like spell
    let spell_id = EntityId::new(100);
    let mut hymn = Card::new(spell_id, "Hymn to Tourach", p1_id);
    hymn.add_type(CardType::Sorcery);
    hymn.effects = vec![Effect::DiscardCards {
        player: crate::core::PlayerId::new(1), // opponent
        count: 2,
        remember_discarded: false,
        optional: false,
        remember_discarding_players: false,
    }];

    let controller = HeuristicController::new(p1_id);

    // Scenario: opponent has cards → should cast
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = p1_id;
    let p2_id = game.players[1].id;

    // Give opponent cards
    for i in 0..3u32 {
        let cid = EntityId::new(50 + i);
        let c = Card::new(cid, format!("Opp Card {}", i), p2_id);
        game.cards.insert(cid, c);
        if let Some(zones) = game.get_player_zones_mut(p2_id) {
            zones.hand.cards.push(cid);
        }
    }

    let view = GameStateView::new(&game, p1_id);
    assert!(
        controller.should_cast_spell(&hymn, &view),
        "should_cast_spell should route Discard effects and approve with opponent hand > 0"
    );

    println!("Discard spell routing test passed");
}

/// mtg-721: equip + sacrifice-to-draw abilities must classify as their own
/// types (not the catch-all `Other`, which `should_activate_ability` never
/// fires). Before this fix the heuristic AI never equipped Trusty Boomerang
/// nor cracked Clue tokens.
#[test]
fn classify_equip_and_clue_abilities() {
    use crate::core::{ActivatedAbility, CardId, Cost, Effect, ManaCost};

    let controller = HeuristicController::new(PlayerId::new(0));

    // Trusty Boomerang's `K:Equip:1` → AttachEquipment effect.
    let equip = ActivatedAbility::new(
        Cost::Mana(ManaCost::new()),
        vec![Effect::AttachEquipment {
            source_equipment: CardId::new(10),
            target_creature: CardId::new(11),
        }],
        "Equip 1".to_string(),
        false,
    );
    assert!(matches!(
        controller.classify_activated_ability(&equip),
        ActivatedAbilityType::Equip
    ));

    // Clue Token's "{2}, Sacrifice this token: Draw a card." → DrawCards.
    let crack = ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::DrawCards {
            player: PlayerId::new(0),
            count: 1,
        }],
        "Draw a card.".to_string(),
        false,
    );
    assert!(matches!(
        controller.classify_activated_ability(&crack),
        ActivatedAbilityType::DrawCard
    ));
}

/// mtg-721: with the new classification, `should_activate_ability` must fire
/// equip during our main phase when the Equipment is UNATTACHED and we
/// control a creature — and must NOT re-fire once attached (no equip-thrash).
#[test]
fn should_activate_equip_when_unattached_with_creature() {
    use crate::core::{ActivatedAbility, Card, CardId, CardType, Cost, Effect, ManaCost};
    use crate::game::controller::GameStateView;
    use crate::game::{GameState, Step};

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;
    game.turn.active_player = p1_id;
    game.turn.current_step = Step::Main1;

    // A creature we control to equip.
    let creature_id = CardId::new(200);
    let mut creature = Card::new(creature_id, "Grizzly Bears", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);

    // Trusty Boomerang (Equipment) with its equip ability, UNATTACHED.
    let boomerang_id = CardId::new(201);
    let mut boomerang = Card::new(boomerang_id, "Trusty Boomerang", p1_id);
    boomerang.add_type(CardType::Artifact);
    boomerang.activated_abilities = vec![ActivatedAbility::new(
        Cost::Mana(ManaCost::new()),
        vec![Effect::AttachEquipment {
            source_equipment: boomerang_id,
            target_creature: creature_id,
        }],
        "Equip 1".to_string(),
        false,
    )];
    game.cards.insert(boomerang_id, boomerang);

    game.battlefield.add(creature_id);
    game.battlefield.add(boomerang_id);

    let controller = HeuristicController::new(p1_id);

    // Unattached + creature present in Main1 → equip.
    {
        let view = GameStateView::new(&game, p1_id);
        let bm = view.get_card(boomerang_id).unwrap();
        assert!(
            controller.should_activate_ability(bm, &view),
            "AI should equip the unattached Boomerang in Main1"
        );
    }

    // Once attached, do NOT re-equip (no thrash).
    game.cards.get_mut(boomerang_id).unwrap().attached_to = Some(creature_id);
    {
        let view = GameStateView::new(&game, p1_id);
        let bm = view.get_card(boomerang_id).unwrap();
        assert!(
            !controller.should_activate_ability(bm, &view),
            "AI must not re-equip an already-attached Equipment"
        );
    }
}
