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
        target.set_power(Some(2));
        target.set_toughness(Some(2));
        target.controller = p2_id;
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // Create a Destroy spell (like Terror)
        let destroy_spell_id = game.next_card_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will be replaced with an opponent's creature
        destroy_spell
            .effects
            .push(Effect::DestroyPermanent { target: CardId::new(0) });
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
}
