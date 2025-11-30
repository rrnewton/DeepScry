#[cfg(test)]
mod tests {
    use crate::core::{Card, CardName, CardType, ManaCost, Subtype};
    use crate::game::{mana_engine::ManaEngine, GameState};
    use smallvec::SmallVec;

    #[test]
    fn test_spider_suit_mana_payment() {
        // Create a game with Spider-Suit in hand
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Spider-Suit (costs {1})
        let spider_suit_id = game.cards.next_id();
        let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
        spider_suit.mana_cost = ManaCost::from_string("1");
        spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
        spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
        game.cards.insert(spider_suit_id, spider_suit);

        // Create a Mountain
        let mountain_id = game.cards.next_id();
        let mut mountain = Card::new(mountain_id, CardName::from("Mountain"), p1_id);
        mountain.types = SmallVec::from_vec(vec![CardType::Land]);
        mountain.subtypes = SmallVec::from_vec(vec![Subtype::from("Basic"), Subtype::from("Mountain")]);
        game.cards.insert(mountain_id, mountain);

        // Put Spider-Suit in hand and Mountain on battlefield
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spider_suit_id);
        }
        game.battlefield.add(mountain_id);

        // Update mana engine
        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Check if we can pay for Spider-Suit
        let cost = game.cards.get(spider_suit_id).unwrap().mana_cost;
        println!("Spider-Suit cost: {:?}", cost);
        println!("Can pay: {}", engine.can_pay(&cost));
        println!("Mana sources: {:?}", engine.all_sources());

        assert!(
            engine.can_pay(&cost),
            "Should be able to pay for Spider-Suit with a Mountain"
        );
    }

    #[test]
    fn test_equipment_enters_battlefield() {
        // Create a game
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Spider-Suit (costs {1}, Equipment)
        let spider_suit_id = game.cards.next_id();
        let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
        spider_suit.mana_cost = ManaCost::from_string("1");
        spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
        spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
        game.cards.insert(spider_suit_id, spider_suit);

        // Put Spider-Suit on the stack (simulating a cast)
        game.stack.add(spider_suit_id);

        // Check initial state
        assert!(
            game.stack.contains(spider_suit_id),
            "Spider-Suit should be on stack before resolve"
        );
        assert!(
            !game.battlefield.contains(spider_suit_id),
            "Spider-Suit should not be on battlefield before resolve"
        );

        // Resolve the spell (no targets needed for Equipment)
        game.resolve_spell(spider_suit_id, &[])
            .expect("resolve_spell should succeed");

        // Check final state
        assert!(
            !game.stack.contains(spider_suit_id),
            "Spider-Suit should not be on stack after resolve"
        );
        assert!(
            game.battlefield.contains(spider_suit_id),
            "Spider-Suit should be on battlefield after resolve"
        );
    }

    #[test]
    fn test_equipment_has_equip_ability() {
        // Test that Equipment with K:Equip:X gets an implicit activated ability
        use crate::loader::CardLoader;

        // Create a game
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Parse Spider-Suit from its card data text
        let spider_suit_content = r#"
Name:Spider-Suit
ManaCost:1
Types:Artifact Equipment
S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2 | AddType$ Spider & Hero | Description$ Equipped creature gets +2/+2 and is a Spider Hero in addition to its other types.
K:Equip:3
"#;
        let spider_suit_def = CardLoader::parse(spider_suit_content).expect("Should parse Spider-Suit");

        // Instantiate the card
        let spider_suit_id = game.cards.next_id();
        let spider_suit = spider_suit_def.instantiate(spider_suit_id, p1_id);

        // Verify it has the Equip keyword
        use crate::core::Keyword;
        assert!(
            spider_suit.keywords.contains(Keyword::Equip),
            "Spider-Suit should have Equip keyword"
        );

        // Verify it has an activated ability
        assert_eq!(
            spider_suit.activated_abilities.len(),
            1,
            "Spider-Suit should have 1 activated ability (Equip)"
        );

        // Verify the ability has the right cost (Equip:3 means {{3}})
        let equip_ability = &spider_suit.activated_abilities[0];
        use crate::core::Cost;
        match &equip_ability.cost {
            Cost::Mana(mana_cost) => {
                assert_eq!(mana_cost.generic, 3, "Equip ability should cost {{3}}");
            }
            other => panic!("Expected Mana cost, got {:?}", other),
        }

        // Verify the ability has an AttachEquipment effect
        use crate::core::Effect;
        assert_eq!(equip_ability.effects.len(), 1, "Equip ability should have 1 effect");
        match &equip_ability.effects[0] {
            Effect::AttachEquipment { source_equipment, .. } => {
                assert_eq!(
                    *source_equipment, spider_suit_id,
                    "AttachEquipment effect should reference Spider-Suit"
                );
            }
            other => panic!("Expected AttachEquipment effect, got {:?}", other),
        }

        // Verify the description (ManaCost Display doesn't include {})
        assert_eq!(
            equip_ability.description, "Equip 3",
            "Equip ability should have correct description"
        );

        // Verify it's marked as sorcery-speed (CR 702.6a: "Activate only as a sorcery")
        assert!(equip_ability.sorcery_speed, "Equip ability should be sorcery-speed");
    }

    #[test]
    fn test_equip_ability_target_validation() {
        // Test that Equip ability only targets creatures you control
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create Spider-Suit (Equipment with Equip:3)
        let spider_suit_id = game.cards.next_id();
        let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
        spider_suit.mana_cost = ManaCost::from_string("1");
        spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
        spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
        spider_suit.controller = p1_id;

        // Add the Equip activated ability manually (normally added by instantiate)
        use crate::core::{ActivatedAbility, Cost, Effect};
        let equip_cost = Cost::Mana(ManaCost::from_string("3"));
        let equip_effects = vec![Effect::AttachEquipment {
            source_equipment: spider_suit_id,
            target_creature: crate::core::CardId::new(0), // Placeholder
        }];
        spider_suit.activated_abilities.push(ActivatedAbility::new(
            equip_cost,
            equip_effects,
            "Equip 3".to_string(),
            false,
        ));
        game.cards.insert(spider_suit_id, spider_suit);

        // Create P1's creature (valid target)
        let p1_creature_id = game.cards.next_id();
        let mut p1_creature = Card::new(p1_creature_id, CardName::from("Grizzly Bears"), p1_id);
        p1_creature.types = SmallVec::from_vec(vec![CardType::Creature]);
        p1_creature.controller = p1_id;
        game.cards.insert(p1_creature_id, p1_creature);

        // Create P2's creature (invalid target - opponent controls it)
        let p2_creature_id = game.cards.next_id();
        let mut p2_creature = Card::new(p2_creature_id, CardName::from("Savannah Lions"), p2_id);
        p2_creature.types = SmallVec::from_vec(vec![CardType::Creature]);
        p2_creature.controller = p2_id;
        game.cards.insert(p2_creature_id, p2_creature);

        // Put Equipment and creatures on battlefield
        game.battlefield.add(spider_suit_id);
        game.battlefield.add(p1_creature_id);
        game.battlefield.add(p2_creature_id);

        // Get valid targets for Equip ability (ability index 0)
        let valid_targets = game
            .get_valid_targets_for_ability(spider_suit_id, 0)
            .expect("Should get valid targets");

        // Should only include P1's creature, not P2's creature
        assert_eq!(
            valid_targets.len(),
            1,
            "Should have exactly 1 valid target (P1's creature)"
        );
        assert!(
            valid_targets.contains(&p1_creature_id),
            "Should include P1's creature as valid target"
        );
        assert!(
            !valid_targets.contains(&p2_creature_id),
            "Should NOT include P2's creature (opponent controls it)"
        );
    }

    #[test]
    fn test_equipment_full_cast_and_resolve() {
        // Create a game with Spider-Suit in hand and a Mountain on the battlefield
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Spider-Suit (costs {1}, Equipment)
        let spider_suit_id = game.cards.next_id();
        let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
        spider_suit.mana_cost = ManaCost::from_string("1");
        spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
        spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
        spider_suit.controller = p1_id;
        game.cards.insert(spider_suit_id, spider_suit);

        // Create a Mountain
        let mountain_id = game.cards.next_id();
        let mut mountain = Card::new(mountain_id, CardName::from("Mountain"), p1_id);
        mountain.types = SmallVec::from_vec(vec![CardType::Land]);
        mountain.subtypes = SmallVec::from_vec(vec![Subtype::from("Basic"), Subtype::from("Mountain")]);
        mountain.controller = p1_id;
        game.cards.insert(mountain_id, mountain);

        // Put Spider-Suit in hand and Mountain on battlefield
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spider_suit_id);
        }
        game.battlefield.add(mountain_id);

        // Add mana to pool (tap the Mountain)
        if let Ok(player) = game.get_player_mut(p1_id) {
            player.mana_pool.red = 1;
        }

        println!("=== Initial state ===");
        println!(
            "Hand contains Spider-Suit: {}",
            game.get_player_zones(p1_id).unwrap().hand.contains(spider_suit_id)
        );
        println!(
            "Battlefield contains Mountain: {}",
            game.battlefield.contains(mountain_id)
        );
        println!(
            "Battlefield contains Spider-Suit: {}",
            game.battlefield.contains(spider_suit_id)
        );
        println!("Player mana pool: {:?}", game.get_player(p1_id).unwrap().mana_pool);

        // Cast the spell using GameState's cast_spell method
        let cast_result = game.cast_spell(p1_id, spider_suit_id, Vec::new());
        println!("\n=== After cast ===");
        println!("Cast result: {:?}", cast_result);
        println!("Stack contains Spider-Suit: {}", game.stack.contains(spider_suit_id));
        println!(
            "Hand contains Spider-Suit: {}",
            game.get_player_zones(p1_id).unwrap().hand.contains(spider_suit_id)
        );
        println!("Player mana pool: {:?}", game.get_player(p1_id).unwrap().mana_pool);
        assert!(cast_result.is_ok(), "Should be able to cast Spider-Suit");
        assert!(
            game.stack.contains(spider_suit_id),
            "Spider-Suit should be on stack after cast"
        );

        // Resolve the spell using GameState's resolve_spell method
        let resolve_result = game.resolve_spell(spider_suit_id, &[]);
        println!("\n=== After resolve ===");
        println!("Resolve result: {:?}", resolve_result);
        println!("Stack contains Spider-Suit: {}", game.stack.contains(spider_suit_id));
        println!(
            "Battlefield contains Spider-Suit: {}",
            game.battlefield.contains(spider_suit_id)
        );

        // Check if card is anywhere
        let in_hand = game.get_player_zones(p1_id).unwrap().hand.contains(spider_suit_id);
        let in_graveyard = game.get_player_zones(p1_id).unwrap().graveyard.contains(spider_suit_id);
        let in_exile = game.get_player_zones(p1_id).unwrap().exile.contains(spider_suit_id);
        println!(
            "Card locations - Hand:{} Battlefield:{} Stack:{} Graveyard:{} Exile:{}",
            in_hand,
            game.battlefield.contains(spider_suit_id),
            game.stack.contains(spider_suit_id),
            in_graveyard,
            in_exile
        );

        assert!(resolve_result.is_ok(), "Should be able to resolve Spider-Suit");
        assert!(
            !game.stack.contains(spider_suit_id),
            "Spider-Suit should not be on stack after resolve"
        );
        assert!(
            game.battlefield.contains(spider_suit_id),
            "Spider-Suit should be on battlefield after resolve"
        );
    }

    #[test]
    fn test_equip_ability_e2e_activation() {
        // E2E test: Directly test Equip ability activation and attachment
        use crate::core::Keyword;
        use crate::loader::CardLoader;

        // Create game with two players
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Load Spider-Suit from card data (has Equip:3 and grants +2/+2)
        let spider_suit_content = r#"
Name:Spider-Suit
ManaCost:1
Types:Artifact Equipment
S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2 | AddType$ Spider & Hero | Description$ Equipped creature gets +2/+2 and is a Spider Hero in addition to its other types.
K:Equip:3
"#;
        let spider_suit_def = CardLoader::parse(spider_suit_content).expect("Should parse Spider-Suit");
        let spider_suit_id = game.cards.next_id();
        let mut spider_suit = spider_suit_def.instantiate(spider_suit_id, p1_id);
        spider_suit.controller = p1_id;

        // Verify Spider-Suit has Equip keyword and ability
        assert!(spider_suit.keywords.contains(Keyword::Equip));
        assert_eq!(spider_suit.activated_abilities.len(), 1);
        assert!(spider_suit.activated_abilities[0].sorcery_speed);

        game.cards.insert(spider_suit_id, spider_suit);

        // Create a 2/2 creature for P1
        let creature_id = game.cards.next_id();
        let mut creature = Card::new(creature_id, CardName::from("Grizzly Bears"), p1_id);
        creature.types = SmallVec::from_vec(vec![CardType::Creature]);
        creature.set_power(Some(2));
        creature.set_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);

        // Put both on battlefield
        game.battlefield.add(spider_suit_id);
        game.battlefield.add(creature_id);

        // Verify creature starts at 2/2 with no Equipment
        let creature_before = game.cards.get(creature_id).expect("Creature should exist");
        assert_eq!(creature_before.base_power(), Some(2));
        assert_eq!(creature_before.base_toughness(), Some(2));

        let spider_suit_before = game.cards.get(spider_suit_id).expect("Spider-Suit should exist");
        assert!(
            spider_suit_before.attached_to.is_none(),
            "Spider-Suit should not be attached yet"
        );

        // Manually activate the Equip ability by calling attach_equipment
        // (This is what would happen when the ability resolves)
        game.attach_equipment(spider_suit_id, creature_id)
            .expect("Should be able to attach Equipment");

        // Verify Spider-Suit is now attached to creature
        let spider_suit_after = game.cards.get(spider_suit_id).expect("Spider-Suit should exist");
        assert_eq!(
            spider_suit_after.attached_to,
            Some(creature_id),
            "Spider-Suit should be attached to creature"
        );

        // Verify the static ability is present
        // Note: The actual application of static abilities to power/toughness bonuses
        // requires integration with the layer system and continuous effect processing,
        // which is tracked in a separate issue. For now, we verify the attachment works.
        assert_eq!(
            spider_suit_after.static_abilities.len(),
            1,
            "Spider-Suit should have 1 static ability"
        );

        println!("✓ E2E test passed: Spider-Suit successfully equipped to creature");
        println!("  - Equipment attached: Spider-Suit → Grizzly Bears");
        println!("  - Static ability present: grants +2/+2 to equipped creature");
        println!("  - Note: P/T bonus application requires layer system integration (future work)");
    }
}
