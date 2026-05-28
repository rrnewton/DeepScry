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

#[test]
fn test_build_choice_context_your_turn() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = PlayerId::new(0);
    game.turn.current_step = crate::game::Step::Main1;

    let view = GameStateView::new(&game, PlayerId::new(0));
    assert_eq!(view.build_choice_context(), "[Your_Main1]");
}

#[test]
fn test_build_choice_context_their_turn() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = PlayerId::new(1);
    game.turn.current_step = crate::game::Step::End;

    let view = GameStateView::new(&game, PlayerId::new(0));
    assert_eq!(view.build_choice_context(), "[Their_EndStep]");
}

#[test]
fn test_build_choice_context_combat_step() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = PlayerId::new(0);
    game.turn.current_step = crate::game::Step::DeclareAttackers;

    let view = GameStateView::new(&game, PlayerId::new(0));
    assert_eq!(view.build_choice_context(), "[Your_Combat_DeclareAttackers]");
}

#[test]
fn test_build_choice_context_with_stack() {
    use crate::core::Card;
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.turn.active_player = PlayerId::new(0);
    game.turn.current_step = crate::game::Step::Main1;

    // Add a card to the stack
    let card_id = game.next_card_id();
    let card = Card::new(card_id, "Lightning Bolt", PlayerId::new(0));
    game.cards.insert(card_id, card);
    game.stack.add(card_id);

    let view = GameStateView::new(&game, PlayerId::new(0));
    assert_eq!(view.build_choice_context(), "[Your_Main1 | Lightning Bolt on stack]");
}

#[test]
fn test_scry_decision_keep_all_on_top_preserves_order() {
    use crate::core::CardId;
    use crate::game::ScryDecision;

    // revealed is top-down: revealed[0] is the current top of library.
    let revealed = vec![CardId::new(10), CardId::new(20), CardId::new(30)];
    let decision = ScryDecision::keep_all_on_top(&revealed);

    // No cards go to bottom in the no-op default.
    assert!(decision.bottom.is_empty());

    // top is bottom-up so library.cards.push() restores the original
    // top order. After applying:
    //   library.push(decision.top[0])  →  inserts CardId(30) just below
    //   library.push(decision.top[1])  →  CardId(20)
    //   library.push(decision.top[2])  →  CardId(10) (top of library)
    // i.e. the new top is the original top (CardId(10)), preserved.
    assert_eq!(
        decision.top.as_slice(),
        &[CardId::new(30), CardId::new(20), CardId::new(10)]
    );
}

#[test]
fn test_surveil_decision_keep_all_on_top_preserves_order() {
    use crate::core::CardId;
    use crate::game::SurveilDecision;

    let revealed = vec![CardId::new(11), CardId::new(22), CardId::new(33)];
    let decision = SurveilDecision::keep_all_on_top(&revealed);

    assert!(decision.graveyard.is_empty());
    // Same bottom-up convention: top of library after restore matches before.
    assert_eq!(
        decision.top.as_slice(),
        &[CardId::new(33), CardId::new(22), CardId::new(11)]
    );
}

#[test]
fn test_scry_decision_partition_is_total() {
    use crate::core::CardId;
    use crate::game::ScryDecision;
    use smallvec::SmallVec;

    // A custom decision: send the middle card to the bottom, keep top + bottom
    // of revealed pile on top in original order.
    let _revealed = [CardId::new(1), CardId::new(2), CardId::new(3)];
    let decision = ScryDecision {
        // bottom-up: cards.push order = [3, 1] → top is CardId(1)
        top: SmallVec::from_slice(&[CardId::new(3), CardId::new(1)]),
        // bottom-up: deepest first → CardId(2) ends up at absolute bottom
        bottom: SmallVec::from_slice(&[CardId::new(2)]),
    };

    // Sanity: every revealed CardId appears exactly once.
    let mut all: Vec<CardId> = decision.top.iter().copied().collect();
    all.extend(decision.bottom.iter().copied());
    all.sort_by_key(|c| c.as_u32());
    assert_eq!(all, vec![CardId::new(1), CardId::new(2), CardId::new(3)]);
}

/// mtg-p43i3: player-target choices must label the viewer "(you)" and an
/// opponent "(them)" — never "(theirs)"/"(yours)" (those are CARD-target
/// labels) — and must list opponents BEFORE the viewer by default (most
/// targeted spells, e.g. Lightning Bolt, are aimed at an opponent).
#[test]
fn test_player_target_choice_labels_and_ordering() {
    use crate::core::{player_as_target_sentinel, reorder_player_targets_opponents_first, CardId};
    use crate::game::controller::{format_card_choice, format_target_choices};
    use crate::game::GameState;

    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = PlayerId::new(0); // caster / viewer (active player)
    let p2 = PlayerId::new(1); // opponent
    let view = GameStateView::new(&game, p1);

    // Single player-sentinel labels: viewer "(you)", opponent "(them)".
    let you = format_card_choice(&view, player_as_target_sentinel(p1), p1, &Default::default());
    let them = format_card_choice(&view, player_as_target_sentinel(p2), p1, &Default::default());
    assert!(
        you.ends_with("(you)"),
        "viewer's own player must be '(you)', got: {you}"
    );
    assert!(them.ends_with("(them)"), "opponent must be '(them)', got: {them}");
    assert!(!you.contains("(theirs)") && !you.contains("(yours)"));
    assert!(!them.contains("(theirs)") && !them.contains("(yours)"));

    // Ordering: simulate the post-sort valid_targets for an "any target" spell
    // cast by P1. Numeric sort puts the caster's low-id sentinel first; the
    // reorder must flip the opponent ahead of the caster.
    let mut targets: Vec<CardId> = vec![player_as_target_sentinel(p1), player_as_target_sentinel(p2)];
    targets.sort();
    assert_eq!(
        targets[0],
        player_as_target_sentinel(p1),
        "pre-reorder: caster first (sort artifact)"
    );
    reorder_player_targets_opponents_first(&mut targets, p1);
    assert_eq!(targets[0], player_as_target_sentinel(p2), "opponent must come first");
    assert_eq!(targets[1], player_as_target_sentinel(p1), "viewer last");

    // The full choice list as a front-end would render it.
    let choices = format_target_choices(&view, &targets, p1);
    assert_eq!(choices[0], "No target");
    assert!(
        choices[1].ends_with("(them)"),
        "[1] must be opponent '(them)', got: {}",
        choices[1]
    );
    assert!(
        choices[2].ends_with("(you)"),
        "[2] must be viewer '(you)', got: {}",
        choices[2]
    );
}
