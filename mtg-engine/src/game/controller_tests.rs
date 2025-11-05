/// Tests for GameStateView
use super::*;
use crate::core::{Player, PlayerId};
use crate::game::GameState;

#[test]
fn test_opponent_life_two_player() {
    // Create a 2-player game
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

    // Set different life totals
    game.players[0].life = 18;
    game.players[1].life = 15;

    // Create view from P1's perspective
    let view = GameStateView::new(&game, PlayerId::new(0));

    // Check that opponent_life() returns P2's life
    assert_eq!(view.opponent_life(), 15);
    assert_eq!(view.life(), 18);
    assert_eq!(view.player_life(PlayerId::new(0)), 18);
    assert_eq!(view.player_life(PlayerId::new(1)), 15);
}

#[test]
fn test_opponent_life_multiplayer() {
    // Create a 2-player game first, then add a third player
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p3 = Player::new(PlayerId::new(2), "P3".to_string(), 20);
    game.players.push(p3);

    // Set different life totals
    game.players[0].life = 20;
    game.players[1].life = 15;
    game.players[2].life = 12;

    // Create view from P1's perspective
    let view = GameStateView::new(&game, PlayerId::new(0));

    // opponent_life() should return sum of all opponents (P2 + P3 = 15 + 12 = 27)
    assert_eq!(view.opponent_life(), 27);
    assert_eq!(view.life(), 20);
}

#[test]
fn test_opponents_iterator() {
    // Create a 2-player game first, then add a third player
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p3 = Player::new(PlayerId::new(2), "P3".to_string(), 20);
    game.players.push(p3);

    // Create view from P1's perspective (ID 0)
    let view = GameStateView::new(&game, PlayerId::new(0));

    // opponents() should return IDs 1 and 2
    let opponents: Vec<PlayerId> = view.opponents().collect();
    assert_eq!(opponents.len(), 2);
    assert!(opponents.contains(&PlayerId::new(1)));
    assert!(opponents.contains(&PlayerId::new(2)));
    assert!(!opponents.contains(&PlayerId::new(0)));

    // Create view from P2's perspective (ID 1)
    let view2 = GameStateView::new(&game, PlayerId::new(1));
    let opponents2: Vec<PlayerId> = view2.opponents().collect();
    assert_eq!(opponents2.len(), 2);
    assert!(opponents2.contains(&PlayerId::new(0)));
    assert!(opponents2.contains(&PlayerId::new(2)));
    assert!(!opponents2.contains(&PlayerId::new(1)));
}

#[test]
fn test_player_life() {
    // Create a 2-player game
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

    game.players[0].life = 10;
    game.players[1].life = 25;

    let view = GameStateView::new(&game, PlayerId::new(0));

    // Test player_life() for specific players
    assert_eq!(view.player_life(PlayerId::new(0)), 10);
    assert_eq!(view.player_life(PlayerId::new(1)), 25);

    // Test that life() returns current player's life
    assert_eq!(view.life(), 10);
}
