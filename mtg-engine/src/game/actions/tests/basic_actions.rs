use crate::core::{Card, CardType};
use crate::game::state::GameState;
use crate::zones::Zone;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_play_land() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a mountain card
        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, "Mountain".to_string(), p1_id);
        card.add_type(CardType::Land);
        game.cards.insert(card_id, card);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(card_id);
        }

        // Play the land
        assert!(game.play_land(p1_id, card_id).is_ok());

        // Check it's on battlefield
        assert!(game.battlefield.contains(card_id));

        // Check player used their land drop
        let player = game.get_player(p1_id).unwrap();
        assert!(!player.can_play_land());
    }

    #[test]
    fn test_tap_for_mana() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a mountain on battlefield
        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, "Mountain".to_string(), p1_id);
        card.add_type(CardType::Land);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        // Tap for mana
        assert!(game.tap_for_mana(p1_id, card_id).is_ok());

        // Check mana was added
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.mana_pool.red, 1);

        // Check land is tapped
        let card = game.cards.get(card_id).unwrap();
        assert!(card.tapped);
    }

    #[test]
    fn test_deal_damage_to_player() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Deal 3 damage
        assert!(game.deal_damage(p1_id, 3).is_ok());

        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.life, 17);
    }

    #[test]
    fn test_move_card_battlefield_to_graveyard() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a creature on battlefield
        let card_id = game.next_entity_id();
        let card = Card::new(card_id, "Test Card".to_string(), p1_id);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        // Test move_card directly
        let result = game.move_card(card_id, Zone::Battlefield, Zone::Graveyard, p1_id);
        if let Err(e) = &result {
            panic!("move_card failed: {e:?}");
        }

        // Check it moved
        assert!(!game.battlefield.contains(card_id), "Card still on battlefield");
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(card_id), "Card not in graveyard");
        }
    }

    #[test]
    fn test_deal_damage_to_creature() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a 2/2 creature on battlefield
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, "Grizzly Bears".to_string(), p1_id);
        card.add_type(CardType::Creature);
        card.set_base_power(Some(2));
        card.set_base_toughness(Some(2));
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        // Deal 2 damage (marks damage, doesn't kill immediately)
        let result = game.deal_damage_to_creature(card_id, 2);
        assert!(result.is_ok(), "deal_damage_to_creature failed: {result:?}");

        // Check state-based actions for lethal damage
        game.check_lethal_damage().unwrap();

        // Check it's in graveyard
        assert!(!game.battlefield.contains(card_id), "Card still on battlefield");
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(card_id), "Card not in graveyard");
        }
    }

    #[test]
    fn test_aura_dies_when_creature_destroyed() {
        // Test CR 704.5d: Auras not attached to valid permanent go to graveyard
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Create an Aura attached to the creature
        let aura_id = game.next_card_id();
        let mut aura = Card::new(aura_id, "Pacifism".to_string(), p1_id);
        aura.add_type(CardType::Enchantment);
        aura.definition.cache.is_enchantment = true;
        aura.definition.cache.is_aura = true;
        aura.attached_to = Some(creature_id);
        game.cards.insert(aura_id, aura);
        game.battlefield.add(aura_id);

        // Verify setup: both on battlefield, aura attached
        assert!(game.battlefield.contains(creature_id));
        assert!(game.battlefield.contains(aura_id));
        let aura_attached = game.cards.get(aura_id).unwrap().get_attached_to();
        assert_eq!(aura_attached, Some(creature_id));

        // Move creature to graveyard (simulating death)
        game.move_card(creature_id, Zone::Battlefield, Zone::Graveyard, p1_id)
            .unwrap();
        assert!(!game.battlefield.contains(creature_id));

        // Check aura SBA - aura should go to graveyard
        game.check_aura_attachment().unwrap();

        // Aura should now be in graveyard
        assert!(
            !game.battlefield.contains(aura_id),
            "Aura still on battlefield after creature died"
        );
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(aura_id), "Aura not in graveyard");
        }
    }

    #[test]
    fn test_equipment_unattaches_when_creature_leaves() {
        // Test CR 704.5n: Equipment attached to nothing becomes unattached
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Create Equipment attached to the creature
        let equip_id = game.next_card_id();
        let mut equipment = Card::new(equip_id, "Bonesplitter".to_string(), p1_id);
        equipment.add_type(CardType::Artifact);
        equipment.definition.cache.is_equipment = true;
        equipment.attached_to = Some(creature_id);
        game.cards.insert(equip_id, equipment);
        game.battlefield.add(equip_id);

        // Verify setup: both on battlefield, equipment attached
        assert!(game.battlefield.contains(creature_id));
        assert!(game.battlefield.contains(equip_id));
        let equip_attached = game.cards.get(equip_id).unwrap().get_attached_to();
        assert_eq!(equip_attached, Some(creature_id));

        // Move creature to graveyard (simulating death)
        game.move_card(creature_id, Zone::Battlefield, Zone::Graveyard, p1_id)
            .unwrap();
        assert!(!game.battlefield.contains(creature_id));

        // Check equipment SBA - equipment should become unattached
        game.check_equipment_attachment().unwrap();

        // Equipment should still be on battlefield but unattached
        assert!(game.battlefield.contains(equip_id), "Equipment left battlefield");
        let equip_attached_after = game.cards.get(equip_id).unwrap().get_attached_to();
        assert_eq!(
            equip_attached_after, None,
            "Equipment still attached after creature died"
        );
    }

    #[test]
    fn test_equipment_unattaches_when_becomes_creature() {
        // Test CR 704.5n: Equipment that becomes a creature becomes unattached
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a creature that the equipment is attached to
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Create Equipment attached to the creature
        let equip_id = game.next_card_id();
        let mut equipment = Card::new(equip_id, "Animated Sword".to_string(), p1_id);
        equipment.add_type(CardType::Artifact);
        equipment.definition.cache.is_equipment = true;
        equipment.attached_to = Some(creature_id);
        game.cards.insert(equip_id, equipment);
        game.battlefield.add(equip_id);

        // Now make the equipment also a creature (simulating Animate Artifact effect)
        let equip = game.cards.get_mut(equip_id).unwrap();
        equip.add_type(CardType::Creature);
        equip.set_base_power(Some(3));
        equip.set_base_toughness(Some(3));

        // Both still on battlefield
        assert!(game.battlefield.contains(creature_id));
        assert!(game.battlefield.contains(equip_id));

        // Check equipment SBA - equipment-creature should become unattached
        game.check_equipment_attachment().unwrap();

        // Equipment-creature should still be on battlefield but unattached
        assert!(
            game.battlefield.contains(equip_id),
            "Equipment-creature left battlefield"
        );
        let equip_attached_after = game.cards.get(equip_id).unwrap().get_attached_to();
        assert_eq!(equip_attached_after, None, "Equipment-creature still attached");
    }
}
