use mtg_engine::{
    core::{Card, CardType},
    game::state::GameState,
    zones::Zone,
    Result,
};

#[tokio::test]
async fn test_combustion_technique_zone_transition_cleanup() -> Result<()> {
    // Create game
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature card
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Grizzly Bears", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));

    // Set transient flags (simulating Combustion Technique targeting it)
    creature.exile_if_would_die_this_turn = true;
    creature.prevent_all_combat_damage_this_turn = true;

    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Verify it is on the battlefield and has the transient flags set to true
    assert!(game.battlefield.contains(creature_id));
    assert!(game.cards.get(creature_id).unwrap().exile_if_would_die_this_turn);
    assert!(game.cards.get(creature_id).unwrap().prevent_all_combat_damage_this_turn);

    // Move the creature from Battlefield to Hand (bouncing it)
    game.move_card(creature_id, Zone::Battlefield, Zone::Hand, p1_id)?;

    // Verify it is in P1's hand
    assert!(game.get_player_zones(p1_id).unwrap().hand.contains(creature_id));

    // Verify transient flags are reset to false
    assert!(!game.cards.get(creature_id).unwrap().exile_if_would_die_this_turn);
    assert!(!game.cards.get(creature_id).unwrap().prevent_all_combat_damage_this_turn);

    // Move it back to Battlefield
    game.move_card(creature_id, Zone::Hand, Zone::Battlefield, p1_id)?;
    assert!(game.battlefield.contains(creature_id));

    // Determine the death destination of the card (simulating dying)
    let dest = game.death_destination_for_card(creature_id);

    // Verify it goes to Graveyard (not Exile)
    assert_eq!(dest, Zone::Graveyard);

    // Move it to the death destination (Graveyard)
    game.move_card(creature_id, Zone::Battlefield, dest, p1_id)?;

    // Verify it is in Graveyard
    assert!(!game.battlefield.contains(creature_id));
    assert!(game.get_player_zones(p1_id).unwrap().graveyard.contains(creature_id));

    Ok(())
}
