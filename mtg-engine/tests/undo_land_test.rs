//! Test that verifies undo correctly restores turn_entered_battlefield field
//!
//! This test demonstrates the bug where playing a land sets turn_entered_battlefield
//! but this mutation is not logged to the undo system.

use mtg_forge_rs::{
    core::{Card, CardType},
    game::{compute_undo_test_hash, GameState},
    Result,
};

#[test]
fn test_undo_play_land_restores_turn_entered_battlefield() -> Result<()> {
    // Create a simple game
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);

    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let p1_id = players[0];

    // Create a land card
    let card_id = game.next_card_id();
    let mut card = Card::new(card_id, "Mountain", p1_id);
    card.types.push(CardType::Land);
    game.cards.insert(card_id, card);

    // Add to hand
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.hand.add(card_id);
    }

    // Take snapshot before playing land
    let snapshot_before = game.clone();
    let hash_before = compute_undo_test_hash(&snapshot_before);

    println!("State before playing land:");
    println!("  turn_entered_battlefield: {:?}", game.cards.get(card_id)?.turn_entered_battlefield);
    println!("  hash: {:08x}", hash_before);

    // Play the land (this sets turn_entered_battlefield = Some(turn_number))
    game.play_land(p1_id, card_id)?;

    println!("\nState after playing land:");
    println!("  turn_entered_battlefield: {:?}", game.cards.get(card_id)?.turn_entered_battlefield);
    println!("  undo_log size: {}", game.undo_log.len());

    // Undo the play (need to undo all actions logged by play_land)
    // play_land logs: MoveCard, SetTurnEnteredBattlefield, SetLandsPlayedThisTurn
    let actions_to_undo = game.undo_log.len();
    for _ in 0..actions_to_undo {
        if game.undo()?.is_none() {
            break; // No more actions to undo
        }
    }

    println!("\nState after undo:");
    println!("  turn_entered_battlefield: {:?}", game.cards.get(card_id)?.turn_entered_battlefield);

    // Compute hash after undo
    let hash_after = compute_undo_test_hash(&game);
    println!("  hash: {:08x}", hash_after);

    // Hashes should match!
    assert_eq!(
        hash_before, hash_after,
        "State hash mismatch! turn_entered_battlefield was not restored by undo.\n\
         Before: {:?}, After: {:?}",
        snapshot_before.cards.get(card_id)?.turn_entered_battlefield,
        game.cards.get(card_id)?.turn_entered_battlefield
    );

    Ok(())
}
