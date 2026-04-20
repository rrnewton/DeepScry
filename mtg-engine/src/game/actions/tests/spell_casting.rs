use crate::core::{Card, CardId, CardType, PlayerId};
use crate::game::state::GameState;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cast_spell_with_mana_payment() {
        use crate::core::{Color, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a Lightning Bolt in hand (cost: R)
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        game.cards.insert(bolt_id, bolt);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(bolt_id);
        }

        // Try to cast without mana - should fail
        let result = game.cast_spell(p1_id, bolt_id, vec![]);
        assert!(result.is_err());

        // Add mana to pool
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Red);

        // Now cast should succeed
        let result = game.cast_spell(p1_id, bolt_id, vec![]);
        assert!(result.is_ok(), "cast_spell failed: {result:?}");

        // Check mana was deducted
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.mana_pool.red, 0);

        // Check card is on stack
        assert!(game.stack.contains(bolt_id));
    }

    #[test]
    fn test_cast_spell_with_generic_mana() {
        use crate::core::{Color, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a spell with cost 2R
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Lava Spike".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("2R");
        game.cards.insert(spell_id, spell);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Add mana: 2R + 1U = 4 mana total
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Red);
        player.mana_pool.add_color(Color::Red);
        player.mana_pool.add_color(Color::Blue);

        // Cast spell - should use 1R for R, and 2R for generic 2
        let result = game.cast_spell(p1_id, spell_id, vec![]);
        assert!(result.is_ok(), "cast_spell failed: {result:?}");

        // Check mana was deducted properly (should have 1 blue left)
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.mana_pool.red, 0);
        assert_eq!(player.mana_pool.blue, 0); // Blue was used for generic cost
        assert_eq!(player.mana_pool.total(), 0);

        // Check card is on stack
        assert!(game.stack.contains(spell_id));
    }

    #[test]
    fn test_execute_damage_effect_to_player() {
        use crate::core::{Effect, TargetRef};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p2_id = players[1];

        let effect = Effect::DealDamage {
            target: TargetRef::Player(p2_id),
            amount: 3,
        };

        assert!(game.execute_effect(&effect).is_ok());

        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17);
    }

    #[test]
    fn test_execute_draw_effect() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        let effect = Effect::DrawCards {
            player: p1_id,
            count: 2,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check cards were drawn
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 2);
            assert_eq!(zones.library.cards.len(), 3);
        }
    }

    #[test]
    fn test_resolve_spell_with_effects() {
        use crate::core::{Effect, ManaCost, TargetRef};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create Lightning Bolt with damage effect
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);

        // Put it on the stack (simulating cast)
        game.stack.add(bolt_id);

        // Resolve the spell
        assert!(game.resolve_spell(bolt_id, &[]).is_ok());

        // Check damage was dealt
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17);

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(bolt_id));
        }
    }

    #[test]
    fn test_resolve_draw_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Add cards to P1's library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Create a Draw spell (like Divination)
        let draw_spell_id = game.next_card_id();
        let mut draw_spell = Card::new(draw_spell_id, "Divination".to_string(), p1_id);
        draw_spell.add_type(CardType::Sorcery);
        draw_spell.mana_cost = ManaCost::from_string("2U");
        // Use placeholder player ID 0 which will be replaced with card owner
        draw_spell.effects.push(Effect::DrawCards {
            player: PlayerId::new(0),
            count: 2,
        });
        game.cards.insert(draw_spell_id, draw_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(draw_spell_id);

        // Check initial state
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 0, "Should start with 0 cards in hand");
            assert_eq!(zones.library.cards.len(), 5, "Should have 5 cards in library");
        }

        // Resolve the spell
        assert!(
            game.resolve_spell(draw_spell_id, &[]).is_ok(),
            "Failed to resolve draw spell"
        );

        // Check cards were drawn
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 2, "Should have drawn 2 cards");
            assert_eq!(zones.library.cards.len(), 3, "Should have 3 cards left in library");
        }

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(draw_spell_id),
                "Draw spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_resolve_destroy_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature for P2 (the target)
        let target_creature_id = game.next_card_id();
        let mut target = Card::new(target_creature_id, "Grizzly Bears".to_string(), p2_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        target.controller = p2_id;
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // Create a Destroy spell (like Terror)
        let destroy_spell_id = game.next_card_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will be replaced with an opponent's creature
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
        });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(destroy_spell_id);

        // Check initial state
        assert!(
            game.battlefield.contains(target_creature_id),
            "Target creature should be on battlefield"
        );

        // Resolve the spell with the target creature
        assert!(
            game.resolve_spell(destroy_spell_id, &[target_creature_id]).is_ok(),
            "Failed to resolve destroy spell"
        );

        // Check target creature was destroyed (moved to graveyard)
        assert!(
            !game.battlefield.contains(target_creature_id),
            "Target creature should not be on battlefield"
        );

        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(target_creature_id),
                "Target creature should be in graveyard"
            );
        }

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(destroy_spell_id),
                "Destroy spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_resolve_gainlife_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a GainLife spell (like Angel's Mercy)
        let gainlife_spell_id = game.next_card_id();
        let mut gainlife_spell = Card::new(gainlife_spell_id, "Angel's Mercy".to_string(), p1_id);
        gainlife_spell.add_type(CardType::Instant);
        gainlife_spell.mana_cost = ManaCost::from_string("2WW");
        // Use placeholder player ID 0 which will be replaced with card controller
        gainlife_spell.effects.push(Effect::GainLife {
            player: PlayerId::new(0),
            amount: 7,
        });
        game.cards.insert(gainlife_spell_id, gainlife_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(gainlife_spell_id);

        // Check initial life total
        let p1_before = game.get_player(p1_id).unwrap();
        assert_eq!(p1_before.life, 20, "Should start with 20 life");

        // Resolve the spell
        assert!(
            game.resolve_spell(gainlife_spell_id, &[]).is_ok(),
            "Failed to resolve gain life spell"
        );

        // Check life was gained
        let p1_after = game.get_player(p1_id).unwrap();
        assert_eq!(p1_after.life, 27, "Should have gained 7 life (20 + 7)");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(gainlife_spell_id),
                "GainLife spell should be in graveyard"
            );
        }
    }

    /// Test that mana ritual spells (like Dark Ritual) add mana to the caster's pool
    /// This tests that the AddMana effect correctly resolves player placeholder (0) to card owner.
    #[test]
    fn test_resolve_mana_ritual_spell() {
        use crate::core::{Color, Effect, ManaCost, PlayerId};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;
        let p2_id = game.players.get(1).unwrap().id;

        // Create a Dark Ritual style spell in P2's hand (owned and cast by P2)
        // Cost: B, adds BBB to controller's pool
        let ritual_id = game.next_card_id();
        let mut ritual = Card::new(ritual_id, "Dark Ritual".to_string(), p2_id);
        ritual.controller = p2_id; // P2 controls this spell
        ritual.add_type(CardType::Instant);
        ritual.mana_cost = ManaCost::from_string("B");

        // AddMana effect with placeholder player ID (0) - should resolve to card_owner (P2)
        ritual.effects.push(Effect::AddMana {
            player: PlayerId::new(0), // Placeholder - will be resolved to card owner
            mana: ManaCost {
                white: 0,
                blue: 0,
                black: 3, // BBB
                red: 0,
                green: 0,
                colorless: 0,
                generic: 0,
                x_count: 0,
            },
            produces_chosen_color: false,
            amount_var: None,
        });
        game.cards.insert(ritual_id, ritual);

        // Add mana for casting (not the ritual's effect)
        let p2 = game.get_player_mut(p2_id).unwrap();
        p2.mana_pool.add_color(Color::Black); // For casting cost

        // Add spell to P2's hand and cast it
        if let Some(zones) = game.get_player_zones_mut(p2_id) {
            zones.hand.add(ritual_id);
        }

        // Cast the spell (this puts it on stack and pays cost)
        assert!(
            game.cast_spell(p2_id, ritual_id, vec![]).is_ok(),
            "P2 should be able to cast Dark Ritual"
        );

        // Check P2's mana pool before resolution (should be empty after paying B)
        let p2_before = game.get_player(p2_id).unwrap();
        assert_eq!(
            p2_before.mana_pool.black, 0,
            "P2 should have no black mana after paying cost"
        );

        // P1's pool should also be empty
        let p1_before = game.get_player(p1_id).unwrap();
        assert_eq!(p1_before.mana_pool.black, 0, "P1 should have no black mana initially");

        // Resolve the spell - this should add BBB to P2's pool (the caster), NOT P1
        assert!(
            game.resolve_spell(ritual_id, &[]).is_ok(),
            "Failed to resolve Dark Ritual"
        );

        // P2 (the caster) should now have 3 black mana
        let p2_after = game.get_player(p2_id).unwrap();
        assert_eq!(
            p2_after.mana_pool.black, 3,
            "P2 should have 3 black mana from Dark Ritual"
        );

        // P1 (the opponent) should NOT have gained any mana
        let p1_after = game.get_player(p1_id).unwrap();
        assert_eq!(
            p1_after.mana_pool.black, 0,
            "P1 should NOT have gained mana from opponent's Dark Ritual"
        );

        // Spell should be in graveyard (it's an instant)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(ritual_id),
                "Dark Ritual should be in P2's graveyard"
            );
        }
    }

    /// Test modal spell data structure (Effect::ModalChoice)
    #[test]
    fn test_modal_spell_effect_structure() {
        use crate::core::{Effect, ModalMode, TargetRestriction};
        use smallvec::smallvec;

        // Create a modal spell like Heartless Act:
        // Choose one —
        // • Destroy target creature with no counters on it.
        // • Remove up to three counters from target creature.

        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0), // Placeholder
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy target creature with no counters on it.".to_string(),
            svar_name: "Destroy".to_string(),
        };

        // For mode2, we'd use RemoveCounter but it's not implemented yet
        // So we use a placeholder effect for testing the structure
        let mode2 = ModalMode {
            effect: Box::new(Effect::DrawCards {
                player: PlayerId::new(0),
                count: 0,
            }), // Placeholder for RemoveCounter
            description: "Remove up to three counters from target creature.".to_string(),
            svar_name: "Remove".to_string(),
        };

        let modal_effect = Effect::ModalChoice {
            modes: smallvec![mode1, mode2],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        };

        // Verify the structure
        if let Effect::ModalChoice {
            modes,
            num_to_choose,
            min_to_choose,
            can_repeat_modes,
        } = modal_effect
        {
            assert_eq!(modes.len(), 2, "Should have 2 modes");
            assert_eq!(num_to_choose, 1, "Should choose 1 mode");
            assert_eq!(min_to_choose, 1, "Minimum 1 mode");
            assert!(!can_repeat_modes, "Cannot repeat modes");

            // Check mode descriptions
            assert!(modes[0].description.contains("Destroy"));
            assert!(modes[1].description.contains("Remove"));

            // Check first mode is DestroyPermanent
            assert!(
                matches!(*modes[0].effect, Effect::DestroyPermanent { .. }),
                "Mode 1 should be DestroyPermanent"
            );
        } else {
            panic!("Expected ModalChoice effect");
        }
    }

    /// Test get_modal_choice_info() detection
    #[test]
    fn test_get_modal_choice_info() {
        use crate::core::{Effect, ModalMode, TargetRestriction};
        use smallvec::smallvec;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a modal spell
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Test Modal Spell".to_string(), p1_id);
        spell.add_type(CardType::Instant);

        // Add modal effect
        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0),
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy target creature".to_string(),
            svar_name: "Destroy".to_string(),
        };

        spell.effects.push(Effect::ModalChoice {
            modes: smallvec![mode1],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        });

        game.cards.insert(spell_id, spell);

        // Test detection
        let modal_info = game.get_modal_choice_info(spell_id);
        assert!(modal_info.is_ok());
        assert!(modal_info.unwrap().is_some(), "Should detect modal spell");

        // Create a non-modal spell
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::None,
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);

        // Non-modal spell should return None
        let non_modal_info = game.get_modal_choice_info(bolt_id);
        assert!(non_modal_info.is_ok());
        assert!(non_modal_info.unwrap().is_none(), "Should not detect non-modal spell");
    }

    /// Test apply_selected_modes() replaces ModalChoice with selected mode effects
    #[test]
    fn test_apply_selected_modes() {
        use crate::core::{Effect, ModalMode, TargetRestriction};
        use smallvec::smallvec;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a modal spell with 2 modes
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Test Modal Spell".to_string(), p1_id);
        spell.add_type(CardType::Instant);

        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0),
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy".to_string(),
            svar_name: "Destroy".to_string(),
        };

        let mode2 = ModalMode {
            effect: Box::new(Effect::DealDamage {
                target: crate::core::TargetRef::None,
                amount: 3,
            }),
            description: "Deal 3 damage".to_string(),
            svar_name: "Damage".to_string(),
        };

        spell.effects.push(Effect::ModalChoice {
            modes: smallvec![mode1, mode2],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        });

        game.cards.insert(spell_id, spell);

        // Verify spell has ModalChoice before applying
        assert!(
            matches!(game.cards.get(spell_id).unwrap().effects[0], Effect::ModalChoice { .. }),
            "Should have ModalChoice before applying modes"
        );

        // Apply mode 1 (index 0 = Destroy)
        let result = game.apply_selected_modes(spell_id, &[0]);
        assert!(result.is_ok());
        assert!(result.unwrap(), "Should return true for modal spell");

        // Verify ModalChoice was replaced with DestroyPermanent
        let spell_after = game.cards.get(spell_id).unwrap();
        assert_eq!(spell_after.effects.len(), 1, "Should have 1 effect after applying");
        assert!(
            matches!(spell_after.effects[0], Effect::DestroyPermanent { .. }),
            "Effect should be DestroyPermanent, got: {:?}",
            spell_after.effects[0]
        );
    }

    /// End-to-end test: Modal spell (Heartless Act) destroying a creature
    ///
    /// Tests the complete flow:
    /// 1. Create a modal spell with ModalChoice effect
    /// 2. Apply mode selection (choose "Destroy")
    /// 3. Put spell on stack
    /// 4. Resolve spell with target
    /// 5. Verify creature is destroyed
    #[test]
    fn test_modal_spell_e2e_heartless_act_destroy() {
        use crate::core::{Color, Effect, ManaCost, ModalMode, TargetRestriction};
        use smallvec::smallvec;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature for P2 (no counters - valid target for mode 1)
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(3));
        creature.set_base_toughness(Some(3));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Create Heartless Act with modal effect
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Heartless Act".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("1B");

        // Mode 1: Destroy target creature with no counters
        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0), // Placeholder
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy target creature with no counters on it.".to_string(),
            svar_name: "Destroy".to_string(),
        };

        // Mode 2: Remove counters (not used in this test)
        let mode2 = ModalMode {
            effect: Box::new(Effect::RemoveCounter {
                target: CardId::new(0),
                counter_type: Some(crate::core::CounterType::P1P1),
                amount: 3,
            }),
            description: "Remove up to three counters from target creature.".to_string(),
            svar_name: "Remove".to_string(),
        };

        spell.effects.push(Effect::ModalChoice {
            modes: smallvec![mode1, mode2],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        });

        game.cards.insert(spell_id, spell);

        // Add spell to P1's hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Add mana for casting (1B)
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Black);
        player.mana_pool.add_color(Color::Colorless);

        // Cast the spell
        let cast_result = game.cast_spell(p1_id, spell_id, vec![]);
        assert!(cast_result.is_ok(), "Should be able to cast spell: {:?}", cast_result);

        // Apply mode selection: choose mode 0 (Destroy)
        let mode_result = game.apply_selected_modes(spell_id, &[0]);
        assert!(mode_result.is_ok(), "Should be able to apply mode selection");

        // Verify spell is on stack
        assert!(game.stack.contains(spell_id), "Spell should be on stack");

        // Verify creature is still on battlefield (spell not resolved yet)
        assert!(
            game.battlefield.contains(creature_id),
            "Creature should still be on battlefield before resolution"
        );

        // Resolve the spell with the creature as target
        let resolve_result = game.resolve_spell(spell_id, &[creature_id]);
        assert!(resolve_result.is_ok(), "Should resolve spell: {:?}", resolve_result);

        // Verify creature was destroyed (moved to graveyard)
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should no longer be on battlefield"
        );

        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(creature_id),
                "Creature should be in owner's graveyard"
            );
        }

        // Verify spell is in P1's graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(spell_id),
                "Heartless Act should be in P1's graveyard"
            );
        }
    }

    /// Test modal spell with RemoveCounter mode (Heartless Act mode 2)
    #[test]
    fn test_modal_spell_e2e_heartless_act_remove_counters() {
        use crate::core::{Color, CounterType, Effect, ManaCost, ModalMode, TargetRestriction};
        use smallvec::smallvec;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature for P2 with +1/+1 counters
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Hydra Hatchling".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(0));
        creature.set_base_toughness(Some(0));
        creature.controller = p2_id;
        // Add 5 +1/+1 counters
        creature.add_counter(CounterType::P1P1, 5);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Verify initial counter count
        assert_eq!(
            game.cards.get(creature_id).unwrap().get_counter(CounterType::P1P1),
            5,
            "Creature should start with 5 +1/+1 counters"
        );

        // Create Heartless Act with modal effect
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Heartless Act".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("1B");

        // Mode 1: Destroy (not used)
        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0),
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy target creature with no counters on it.".to_string(),
            svar_name: "Destroy".to_string(),
        };

        // Mode 2: Remove up to 3 counters
        let mode2 = ModalMode {
            effect: Box::new(Effect::RemoveCounter {
                target: CardId::new(0),
                counter_type: Some(CounterType::P1P1),
                amount: 3,
            }),
            description: "Remove up to three counters from target creature.".to_string(),
            svar_name: "Remove".to_string(),
        };

        spell.effects.push(Effect::ModalChoice {
            modes: smallvec![mode1, mode2],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        });

        game.cards.insert(spell_id, spell);

        // Add spell to P1's hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Add mana for casting (1B)
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Black);
        player.mana_pool.add_color(Color::Colorless);

        // Cast the spell
        let cast_result = game.cast_spell(p1_id, spell_id, vec![]);
        assert!(cast_result.is_ok(), "Should be able to cast spell");

        // Apply mode selection: choose mode 1 (RemoveCounter)
        let mode_result = game.apply_selected_modes(spell_id, &[1]);
        assert!(mode_result.is_ok(), "Should be able to apply mode selection");

        // Verify spell effect was changed to RemoveCounter
        let spell = game.cards.get(spell_id).unwrap();
        assert!(
            matches!(spell.effects[0], Effect::RemoveCounter { amount: 3, .. }),
            "Effect should be RemoveCounter after mode selection"
        );

        // Resolve the spell with the creature as target
        let resolve_result = game.resolve_spell(spell_id, &[creature_id]);
        assert!(resolve_result.is_ok(), "Should resolve spell: {:?}", resolve_result);

        // Verify 3 counters were removed (5 - 3 = 2 remaining)
        assert_eq!(
            game.cards.get(creature_id).unwrap().get_counter(CounterType::P1P1),
            2,
            "Creature should have 2 +1/+1 counters remaining"
        );

        // Creature should still be on battlefield (just smaller now)
        assert!(
            game.battlefield.contains(creature_id),
            "Creature should still be on battlefield"
        );
    }

    /// Test that mimics the WASM flow: targets stored in spell_targets, then looked up during resolution
    /// This tests the flow used by GameLoop where spell_targets is populated during casting
    /// and then read during resolve_top_spell_from_stack
    #[test]
    fn test_modal_spell_with_spell_targets_storage() {
        use crate::core::{Color, Effect, ManaCost, ModalMode, TargetRestriction};
        use smallvec::smallvec;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature for P2 (no counters - valid target for mode 1)
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(3));
        creature.set_base_toughness(Some(3));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Create Heartless Act with modal effect
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Heartless Act".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("1B");

        // Mode 1: Destroy target creature with no counters
        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0), // Placeholder
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy target creature with no counters on it.".to_string(),
            svar_name: "Destroy".to_string(),
        };

        // Mode 2: Remove counters
        let mode2 = ModalMode {
            effect: Box::new(Effect::RemoveCounter {
                target: CardId::new(0),
                counter_type: Some(crate::core::CounterType::P1P1),
                amount: 3,
            }),
            description: "Remove up to three counters from target creature.".to_string(),
            svar_name: "Remove".to_string(),
        };

        spell.effects.push(Effect::ModalChoice {
            modes: smallvec![mode1, mode2],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        });

        game.cards.insert(spell_id, spell);

        // Add spell to P1's hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Add mana for casting (1B)
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Black);
        player.mana_pool.add_color(Color::Colorless);

        // Step 1: Apply mode selection BEFORE casting (like priority.rs line 468)
        let mode_result = game.apply_selected_modes(spell_id, &[0]);
        assert!(mode_result.is_ok(), "Should be able to apply mode selection");

        // Verify effect was changed to DestroyPermanent
        let spell = game.cards.get(spell_id).unwrap();
        assert!(
            matches!(spell.effects[0], Effect::DestroyPermanent { .. }),
            "Effect should be DestroyPermanent after mode selection, got: {:?}",
            spell.effects[0]
        );

        // Step 2: Cast the spell (moves to stack)
        let cast_result = game.cast_spell(p1_id, spell_id, vec![]);
        assert!(cast_result.is_ok(), "Should be able to cast spell: {:?}", cast_result);

        // Verify spell is on stack
        assert!(game.stack.contains(spell_id), "Spell should be on stack");

        // Step 3: Store targets in spell_targets (like priority.rs line 519)
        game.spell_targets.push((spell_id, smallvec::smallvec![creature_id]));

        // Step 4: Look up targets and resolve (like resolve_top_spell_from_stack does)
        let targets: smallvec::SmallVec<[CardId; 2]> = game
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        assert_eq!(targets.len(), 1, "Should have 1 target stored");
        assert_eq!(targets[0], creature_id, "Target should be the creature");

        // Step 5: Resolve the spell using the looked-up targets
        let resolve_result = game.resolve_spell(spell_id, &targets);
        assert!(resolve_result.is_ok(), "Should resolve spell: {:?}", resolve_result);

        // Verify creature was destroyed (moved to graveyard)
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should no longer be on battlefield after Heartless Act resolves"
        );

        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(creature_id),
                "Creature should be in owner's graveyard"
            );
        }
    }

    /// Test that Heartless Act loaded from card file correctly destroys creatures
    /// This tests the full parsing path including TargetRestriction with !HasCounters
    #[test]
    fn test_heartless_act_from_card_file_destroys_creature() {
        use crate::loader::CardLoader;
        use std::path::PathBuf;

        // Load Heartless Act from the actual card file
        let path = PathBuf::from("cardsfolder/h/heartless_act.txt");
        if !path.exists() {
            // Skip test if cardsfolder is not available
            return;
        }

        let card_def = CardLoader::load_from_file(&path).expect("Should load Heartless Act");
        assert_eq!(card_def.name.as_str(), "Heartless Act");

        // Create game state
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature for P2 (no counters - valid target)
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "The Boulder".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(4));
        creature.set_base_toughness(Some(4));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Create Heartless Act card from the loaded definition using instantiate()
        let spell_id = game.next_card_id();
        let mut spell = card_def.instantiate(spell_id, p1_id);
        spell.controller = p1_id;

        // Verify it's a ModalChoice before we insert
        use crate::core::Effect;
        assert!(
            matches!(&spell.effects[0], Effect::ModalChoice { modes, .. } if modes.len() == 2),
            "Should have ModalChoice with 2 modes, got: {:?}",
            spell.effects[0]
        );

        // Get the modal choice and check the destroy mode's restriction
        if let Effect::ModalChoice { modes, .. } = &spell.effects[0] {
            if let Effect::DestroyPermanent { restriction, .. } = &*modes[0].effect {
                assert!(
                    restriction.requires_no_counters,
                    "Destroy mode should have requires_no_counters=true"
                );
            } else {
                panic!("Mode 0 should be DestroyPermanent, got: {:?}", modes[0].effect);
            }
        }

        game.cards.insert(spell_id, spell);

        // Add spell to P1's hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Apply mode selection (choose Destroy mode)
        let mode_result = game.apply_selected_modes(spell_id, &[0]);
        assert!(mode_result.is_ok(), "Should apply mode selection");

        // Verify effect is now DestroyPermanent
        let spell = game.cards.get(spell_id).unwrap();
        assert!(
            matches!(&spell.effects[0], Effect::DestroyPermanent { .. }),
            "Effect should be DestroyPermanent after mode selection"
        );

        // Check that the creature is a valid target
        let valid_targets = game.get_valid_targets_for_spell(spell_id).unwrap();
        assert!(
            valid_targets.contains(&creature_id),
            "The Boulder should be a valid target (no counters)"
        );

        // Cast the spell
        use crate::core::Color;
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Black);
        player.mana_pool.add_color(Color::Colorless);
        let cast_result = game.cast_spell(p1_id, spell_id, vec![]);
        assert!(cast_result.is_ok(), "Should cast spell");

        // Store targets (like WASM flow)
        game.spell_targets.push((spell_id, smallvec::smallvec![creature_id]));

        // Resolve the spell
        let targets: smallvec::SmallVec<[CardId; 2]> = game
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        let resolve_result = game.resolve_spell(spell_id, &targets);
        assert!(resolve_result.is_ok(), "Should resolve spell");

        // Verify creature was destroyed
        assert!(
            !game.battlefield.contains(creature_id),
            "The Boulder should no longer be on battlefield"
        );

        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(creature_id),
                "The Boulder should be in graveyard"
            );
        }
    }

    #[test]
    fn test_affinity_cost_reduction() {
        use crate::core::{KeywordArgs, ManaCost, Subtype};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a spell with Affinity for Ally (like Allies at Last: 2G with Affinity:Ally)
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Allies at Last".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("2G"); // Base cost: 2G
        spell.keywords.insert_complex(KeywordArgs::Affinity {
            card_type: Subtype::new("Ally"),
        });
        game.cards.insert(spell_id, spell);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Test 1: Without any Allies, effective cost should be 2G (generic = 2)
        let cost_no_allies = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(cost_no_allies.generic, 2, "Without allies, generic cost should be 2");
        assert_eq!(cost_no_allies.green, 1, "Green cost should remain 1");

        // Create 2 Ally creatures on battlefield
        for i in 0..2 {
            let ally_id = game.next_card_id();
            let mut ally = Card::new(ally_id, format!("Ally Creature {}", i), p1_id);
            ally.add_type(CardType::Creature);
            ally.subtypes.push(Subtype::new("Ally"));
            ally.controller = p1_id;
            game.cards.insert(ally_id, ally);
            game.battlefield.add(ally_id);
        }

        // Test 2: With 2 Allies, effective cost should be G (generic = 0)
        let cost_with_allies = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(
            cost_with_allies.generic, 0,
            "With 2 allies, generic cost should be reduced to 0"
        );
        assert_eq!(cost_with_allies.green, 1, "Green cost should remain 1");

        // Create 3 more Ally creatures (total 5)
        for i in 2..5 {
            let ally_id = game.next_card_id();
            let mut ally = Card::new(ally_id, format!("Ally Creature {}", i), p1_id);
            ally.add_type(CardType::Creature);
            ally.subtypes.push(Subtype::new("Ally"));
            ally.controller = p1_id;
            game.cards.insert(ally_id, ally);
            game.battlefield.add(ally_id);
        }

        // Test 3: With 5 Allies, cost should still be G (generic can't go negative)
        let cost_many_allies = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(
            cost_many_allies.generic, 0,
            "With 5 allies, generic cost should stay at 0 (not negative)"
        );
        assert_eq!(cost_many_allies.green, 1, "Green cost should remain 1");
    }

    #[test]
    fn test_affinity_different_subtypes() {
        use crate::core::{KeywordArgs, ManaCost, Subtype};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a spell with Affinity for Artifact (like Thoughtcast: 4U with Affinity:Artifact)
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Thoughtcast".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("4U"); // Base cost: 4U
        spell.keywords.insert_complex(KeywordArgs::Affinity {
            card_type: Subtype::new("Artifact"),
        });
        game.cards.insert(spell_id, spell);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Create some Artifacts on battlefield
        for i in 0..3 {
            let artifact_id = game.next_card_id();
            let mut artifact = Card::new(artifact_id, format!("Artifact {}", i), p1_id);
            artifact.add_type(CardType::Artifact);
            // Note: Artifact is a card type, not a subtype. But Affinity:Artifact traditionally
            // works based on artifact permanents. Let's also add Artifact as a subtype to match
            // the Affinity pattern for this test.
            artifact.subtypes.push(Subtype::new("Artifact"));
            artifact.controller = p1_id;
            game.cards.insert(artifact_id, artifact);
            game.battlefield.add(artifact_id);
        }

        // With 3 Artifacts, cost should be 1U (4 - 3 = 1 generic)
        let cost = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(cost.generic, 1, "With 3 artifacts, generic cost should be 1");
        assert_eq!(cost.blue, 1, "Blue cost should remain 1");
    }

    #[test]
    fn test_reduce_cost_static_ability() {
        use crate::core::{CostReductionCondition, CostReductionTarget, ManaCost, StaticAbility, Subtype};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create Gran-Gran on battlefield with ReduceCost ability:
        // Non-creature spells cost {1} less if you have 3+ Lessons in graveyard
        let gran_gran_id = game.next_card_id();
        let mut gran_gran = Card::new(gran_gran_id, "Gran-Gran".to_string(), p1_id);
        gran_gran.add_type(CardType::Creature);
        gran_gran.controller = p1_id;
        gran_gran.static_abilities.push(StaticAbility::ReduceCost {
            valid_card: CostReductionTarget::NonCreature,
            amount: 1,
            condition: Some(CostReductionCondition {
                is_present: "Lesson.YouOwn".to_string(),
                present_zone: crate::zones::Zone::Graveyard,
                min_count: 3,
            }),
            description: "Non-creature spells cost {1} less".to_string(),
        });
        game.cards.insert(gran_gran_id, gran_gran);
        game.battlefield.add(gran_gran_id);

        // Create a non-creature spell in hand (cost: 2U)
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Some Instant".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("2U");
        game.cards.insert(spell_id, spell);

        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Test 1: Without any Lessons in graveyard, cost should remain 2U
        let cost_no_lessons = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(
            cost_no_lessons.generic, 2,
            "Without lessons, generic cost should remain 2"
        );

        // Add 2 Lessons to graveyard (not enough)
        for i in 0..2 {
            let lesson_id = game.next_card_id();
            let mut lesson = Card::new(lesson_id, format!("Lesson {}", i), p1_id);
            lesson.add_type(CardType::Sorcery);
            lesson.subtypes.push(Subtype::new("Lesson"));
            lesson.owner = p1_id;
            game.cards.insert(lesson_id, lesson);

            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.graveyard.add(lesson_id);
            }
        }

        // Test 2: With only 2 Lessons, cost should still be 2U
        let cost_2_lessons = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(
            cost_2_lessons.generic, 2,
            "With only 2 lessons, generic cost should remain 2"
        );

        // Add 1 more Lesson (now 3 total)
        let lesson_id = game.next_card_id();
        let mut lesson = Card::new(lesson_id, "Lesson 2".to_string(), p1_id);
        lesson.add_type(CardType::Sorcery);
        lesson.subtypes.push(Subtype::new("Lesson"));
        lesson.owner = p1_id;
        game.cards.insert(lesson_id, lesson);

        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.graveyard.add(lesson_id);
        }

        // Test 3: With 3 Lessons, cost should be reduced to 1U
        let cost_3_lessons = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(
            cost_3_lessons.generic, 1,
            "With 3 lessons, generic cost should be reduced to 1"
        );
        assert_eq!(cost_3_lessons.blue, 1, "Blue cost should remain 1");

        // Create a creature spell in hand - should NOT get reduction
        let creature_spell_id = game.next_card_id();
        let mut creature_spell = Card::new(creature_spell_id, "Some Creature".to_string(), p1_id);
        creature_spell.add_type(CardType::Creature);
        creature_spell.mana_cost = ManaCost::from_string("3G");
        game.cards.insert(creature_spell_id, creature_spell);

        // Test 4: Creature spell should NOT get the reduction (NonCreature only)
        let creature_cost = game.calculate_effective_cost(creature_spell_id, p1_id);
        assert_eq!(
            creature_cost.generic, 3,
            "Creature spell should NOT get cost reduction from Gran-Gran"
        );
    }

    #[test]
    fn test_reduce_cost_unconditional() {
        use crate::core::{CostReductionTarget, ManaCost, StaticAbility};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a permanent with unconditional cost reduction (no condition)
        let reducer_id = game.next_card_id();
        let mut reducer = Card::new(reducer_id, "Cost Reducer".to_string(), p1_id);
        reducer.add_type(CardType::Artifact);
        reducer.controller = p1_id;
        reducer.static_abilities.push(StaticAbility::ReduceCost {
            valid_card: CostReductionTarget::AllSpells,
            amount: 1,
            condition: None, // No condition - always active
            description: "All spells cost {1} less".to_string(),
        });
        game.cards.insert(reducer_id, reducer);
        game.battlefield.add(reducer_id);

        // Create a spell in hand (cost: 3RR)
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Big Spell".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("3RR");
        game.cards.insert(spell_id, spell);

        // Test: Cost should be reduced to 2RR
        let cost = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(cost.generic, 2, "Generic cost should be reduced by 1");
        assert_eq!(cost.red, 2, "Red cost should remain 2");
    }

    #[test]
    fn test_raise_cost_mana_increase() {
        use crate::core::{CostReductionTarget, ManaCost, RaisedCost, StaticAbility};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a permanent that increases non-creature spell costs by {1}
        // (like Thalia, Guardian of Thraben)
        let thalia_id = game.next_card_id();
        let mut thalia = Card::new(thalia_id, "Thalia".to_string(), p1_id);
        thalia.add_type(CardType::Creature);
        thalia.controller = p1_id;
        thalia.static_abilities.push(StaticAbility::RaiseCost {
            valid_card: CostReductionTarget::NonCreature,
            raised_cost: RaisedCost::Mana(1),
            description: "Non-creature spells cost {1} more".to_string(),
        });
        game.cards.insert(thalia_id, thalia);
        game.battlefield.add(thalia_id);

        // Create a non-creature spell in hand (cost: 1U)
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Counterspell".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("1U");
        game.cards.insert(spell_id, spell);

        // Test: Cost should be increased to 2U
        let cost = game.calculate_effective_cost(spell_id, p1_id);
        assert_eq!(cost.generic, 2, "Generic cost should be increased by 1");
        assert_eq!(cost.blue, 1, "Blue cost should remain 1");

        // Create a creature spell - should NOT be affected
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Bear".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.mana_cost = ManaCost::from_string("1G");
        game.cards.insert(creature_id, creature);

        let creature_cost = game.calculate_effective_cost(creature_id, p1_id);
        assert_eq!(
            creature_cost.generic, 1,
            "Creature spell should NOT get cost increase from Thalia"
        );
    }

    /// Test that Sandbenders' Storm Earthbend mode properly targets a land.
    /// Regression test: Earthbend was listed in "no targeting requirements" catch-all
    /// in get_valid_targets_for_spell, causing the spell to resolve with placeholder target
    /// and silently fizzle.
    #[test]
    fn test_modal_spell_earthbend_mode_targets_land() {
        use crate::core::{Effect, ManaCost, ModalMode, TargetRestriction};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a land for P1 to earthbend
        let land_id = game.next_card_id();
        let mut land = Card::new(land_id, "Forest".to_string(), p1_id);
        land.add_type(CardType::Land);
        land.controller = p1_id;
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // Create Sandbenders' Storm with modal effect
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Sandbenders' Storm".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("3W");

        // Mode 1: Destroy creature with power 4+
        let mode1 = ModalMode {
            effect: Box::new(Effect::DestroyPermanent {
                target: CardId::new(0),
                restriction: TargetRestriction::from_types([crate::core::TargetType::Creature]),
            }),
            description: "Destroy target creature with power 4 or greater.".to_string(),
            svar_name: "DBDestroy".to_string(),
        };

        // Mode 2: Earthbend 3
        let mode2 = ModalMode {
            effect: Box::new(Effect::Earthbend {
                target: CardId::new(0),
                num_counters: 3,
            }),
            description: "Earthbend 3.".to_string(),
            svar_name: "DBEarthbend".to_string(),
        };

        spell.effects.push(Effect::ModalChoice {
            modes: smallvec::smallvec![mode1, mode2],
            num_to_choose: 1,
            min_to_choose: 1,
            can_repeat_modes: false,
        });
        game.cards.insert(spell_id, spell);

        // Step 1: Verify earthbend mode has valid targets (lands you control)
        let valid_modes = game.get_valid_modes_for_spell(spell_id, p1_id).unwrap();
        let earthbend_mode = valid_modes.iter().find(|(idx, _)| *idx == 1);
        assert!(
            earthbend_mode.is_some(),
            "Earthbend mode should be present in valid modes"
        );
        assert!(
            earthbend_mode.unwrap().1,
            "Earthbend mode should have valid targets (P1 controls a land)"
        );

        // Step 2: Apply earthbend mode selection
        let applied = game.apply_selected_modes(spell_id, &[1]).unwrap();
        assert!(applied, "Should successfully apply earthbend mode");

        // Step 3: Get valid targets for the spell (after mode selection)
        let valid_targets = game.get_valid_targets_for_spell(spell_id).unwrap();
        assert!(
            !valid_targets.is_empty(),
            "Earthbend spell should have valid land targets after mode selection"
        );
        assert!(
            valid_targets.contains(&land_id),
            "P1's Forest should be a valid earthbend target"
        );
    }
}
