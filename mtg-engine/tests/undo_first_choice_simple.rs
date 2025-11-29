//! Simple direct test for undoing the first choice
//!
//! This test directly calls undo_to_previous_choice_point() to verify
//! the undo mechanism works correctly.
//!
//! This test requires the `undo` feature to be enabled.

#![cfg(feature = "undo")]

use mtg_forge_rs::{
    core::{Card, CardType},
    game::GameState,
    zones::Zone,
    Result,
};

#[tokio::test]
async fn test_undo_first_choice_direct() -> Result<()> {
    println!("\n=== Direct Test: Undo First Choice ===\n");

    // Create game
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Add a Forest to P1's hand
    let forest_id = game.next_card_id();
    let mut forest = Card::new(forest_id, "Forest", p1_id);
    forest.types.push(CardType::Land);
    game.cards.insert(forest_id, forest);

    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.hand.add(forest_id);
    }

    println!("Initial state:");
    println!("  Undo log size: {}", game.undo_log.len());
    println!(
        "  Forest in hand: {}",
        game.get_player_zones(p1_id)
            .map(|z| z.hand.contains(forest_id))
            .unwrap_or(false)
    );
    println!("  Forest on battlefield: {}", game.battlefield.contains(forest_id));

    // Log a choice point (simulating player making a choice)
    // Note: ChoicePoint has fields: player_id, choice_id, choice (not description)
    let prior_log_size = game.logger.log_count();
    game.undo_log.log(
        mtg_forge_rs::undo::GameAction::ChoicePoint {
            player_id: p1_id,
            choice_id: 1, // First choice
            choice: None, // Not replaying, so no choice data
        },
        prior_log_size,
    );

    println!("\nChoice point logged. Undo log size: {}", game.undo_log.len());

    // Play the forest
    game.move_card(forest_id, Zone::Hand, Zone::Battlefield, p1_id)?;

    println!("\nAfter playing forest:");
    println!("  Undo log size: {}", game.undo_log.len());
    println!(
        "  Forest in hand: {}",
        game.get_player_zones(p1_id)
            .map(|z| z.hand.contains(forest_id))
            .unwrap_or(false)
    );
    println!("  Forest on battlefield: {}", game.battlefield.contains(forest_id));

    assert!(
        !game.get_player_zones(p1_id).unwrap().hand.contains(forest_id),
        "Forest should not be in hand"
    );
    assert!(game.battlefield.contains(forest_id), "Forest should be on battlefield");

    // Add some more actions to simulate game progression
    game.advance_step()?; // Advance to next step
    game.advance_step()?; // Advance again

    println!("\nAfter advancing steps:");
    println!("  Undo log size: {}", game.undo_log.len());
    println!("  Current step: {:?}", game.turn.current_step);

    // Now undo to the choice point (for p1)
    println!("\n--- Calling undo_to_previous_choice_point(p1_id) ---");

    let undo_result = game.undo_to_previous_choice_point(p1_id)?;

    if let Some((actions_undone, log_size)) = undo_result {
        println!("Undo result:");
        println!("  Actions undone: {}", actions_undone);
        println!("  Log size to truncate to: {}", log_size);

        // Truncate logger
        game.logger.truncate_to(log_size);

        println!("\nAfter undo:");
        println!("  Undo log size: {}", game.undo_log.len());
        println!(
            "  Forest in hand: {}",
            game.get_player_zones(p1_id)
                .map(|z| z.hand.contains(forest_id))
                .unwrap_or(false)
        );
        println!("  Forest on battlefield: {}", game.battlefield.contains(forest_id));
        println!("  Current step: {:?}", game.turn.current_step);

        // Verify forest is back in hand
        assert!(
            game.get_player_zones(p1_id).unwrap().hand.contains(forest_id),
            "Forest should be back in hand after undo"
        );
        assert!(
            !game.battlefield.contains(forest_id),
            "Forest should NOT be on battlefield after undo"
        );

        println!("\n✓ Test passed! Undo correctly moved Forest back to hand.");
    } else {
        panic!("Undo failed - no choice point found");
    }

    Ok(())
}
