//! Proof tests for the LATENT per-action-undo holes closed by the mtg-ey2vf
//! consolidation.
//!
//! Before consolidation, `GameState::undo_to_previous_choice_point` (the human
//! undo / MCTS rewind-to-choice-point path) reversed actions with its OWN inline
//! `match action { ... _ => {} }` that handled only a SUBSET of variants and
//! silently dropped the rest (`ShuffleLibrary`, `DeclareAttacker`,
//! `DeclareBlocker`, `ClearCombat`, `CloneCard`, `AnimateTypeline`,
//! `SetCommanderDamage`, `PushExtraTurn`, ...). Those actions were POPPED off the
//! log but never reverted, leaving stale state after an undo-to-choice-point —
//! a real correctness hole, not just refactor debt.
//!
//! The consolidation made that path DELEGATE to the single canonical
//! `GameAction::undo`, so every variant is now reverted. These tests PIN that:
//! each undoes across a previously-skipped action and asserts the state returns
//! to its pre-action value. Run against the pre-consolidation code (restore the
//! inline `_ => {}` match) they FAIL (stale shuffled library / still-declared
//! attacker); after consolidation they PASS.

use mtg_engine::{
    core::{Card, CardType},
    game::GameState,
    Result,
};

/// Undo across a `ShuffleLibrary` via `undo_to_previous_choice_point` must
/// restore the exact previous library order.
///
/// Pre-consolidation the nested `_ => {}` skipped `ShuffleLibrary`, so the
/// library stayed shuffled after the undo — a silent hole.
#[tokio::test]
async fn undo_to_choice_point_reverts_shuffle_library() -> Result<()> {
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    game.seed_rng(12345);
    let p1_id = game.players[0].id;

    // Give P1 a multi-card, distinctly-ordered library so a shuffle visibly
    // reorders it.
    for i in 0..12 {
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, format!("Card {i}").as_str(), p1_id);
        card.add_type(CardType::Land);
        game.cards.insert(card_id, card);
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.library.add(card_id);
        }
    }

    let order_before: Vec<_> = game.get_player_zones(p1_id).unwrap().library.cards.to_vec();

    // Log a choice point for P1, then shuffle (which logs ShuffleLibrary).
    let prior_log_size = game.logger.log_count();
    game.undo_log.log(
        mtg_engine::undo::GameAction::ChoicePoint {
            player_id: p1_id,
            choice_id: 1,
            choice: None,
        },
        prior_log_size,
    );
    game.shuffle_library(p1_id);

    let order_after_shuffle: Vec<_> = game.get_player_zones(p1_id).unwrap().library.cards.to_vec();
    assert_ne!(
        order_before, order_after_shuffle,
        "precondition: the shuffle must actually reorder the library for this test to be meaningful"
    );

    // Undo back to the choice point.
    let result = game.undo_to_previous_choice_point(p1_id)?;
    assert!(result.is_some(), "a ChoicePoint for P1 must be found");
    if let Some((_, log_size)) = result {
        game.logger.truncate_to(log_size);
    }

    let order_after_undo: Vec<_> = game.get_player_zones(p1_id).unwrap().library.cards.to_vec();
    assert_eq!(
        order_before, order_after_undo,
        "mtg-732: undo_to_previous_choice_point must revert ShuffleLibrary (previously silently skipped by the nested `_ => {{}}`)"
    );

    Ok(())
}

/// Undo across a `DeclareAttacker` via `undo_to_previous_choice_point` must
/// clear the attacker from combat.
///
/// Pre-consolidation the nested `_ => {}` skipped `DeclareAttacker`, so the
/// creature stayed declared as an attacker after the undo — a silent hole.
#[tokio::test]
async fn undo_to_choice_point_reverts_declare_attacker() -> Result<()> {
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Put a creature on the battlefield under P1's control.
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Grizzly Bears", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    assert!(
        !game.combat.is_attacking(creature_id),
        "precondition: creature must not be attacking before declaration"
    );

    // Log a choice point for P1, then declare the attacker (logs DeclareAttacker).
    let prior_log_size = game.logger.log_count();
    game.undo_log.log(
        mtg_engine::undo::GameAction::ChoicePoint {
            player_id: p1_id,
            choice_id: 1,
            choice: None,
        },
        prior_log_size,
    );
    game.declare_attacker_logged(creature_id, p2_id);
    assert!(
        game.combat.is_attacking(creature_id),
        "precondition: creature must be attacking after declaration"
    );
    assert!(
        game.combat.combat_active,
        "combat_active must be set by the declaration"
    );

    // Undo back to the choice point.
    let result = game.undo_to_previous_choice_point(p1_id)?;
    assert!(result.is_some(), "a ChoicePoint for P1 must be found");
    if let Some((_, log_size)) = result {
        game.logger.truncate_to(log_size);
    }

    assert!(
        !game.combat.is_attacking(creature_id),
        "mtg-732: undo_to_previous_choice_point must revert DeclareAttacker (previously silently skipped by the nested `_ => {{}}`)"
    );
    assert!(
        !game.combat.combat_active,
        "mtg-732: combat_active must be restored to its pre-declaration value (false) after undo"
    );

    Ok(())
}
