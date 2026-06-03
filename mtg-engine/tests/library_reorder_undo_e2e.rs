//! mtg-ba6uq #2: library-reorder operations (scry / surveil / Dig "rest to
//! bottom") must round-trip through the per-action undo path.
//!
//! These effects mutate the library (and, for surveil, the graveyard) Vec with
//! RAW `remove`/`insert`/`push` ops rather than logged `MoveCard`s, and were
//! previously NOT undo-logged at all. A mid-turn rewind therefore left the
//! library in its reordered state; a forward replay then re-derived a different
//! order, diverging the rewind-verifier hash (and the per-action MCTS undo).
//! The NETWORK hash excludes library order (hidden info), so this is NOT a
//! cross-machine desync — but it IS an undo-log completeness hole.
//!
//! The fix logs a `GameAction::ReorderLibrary { previous_order,
//! previous_graveyard }` snapshot (via the shared `GameState::log_library_reorder`
//! helper) before each raw reorder; its undo restores the captured order(s).
//!
//! NEGATIVE-TEST GUARD: with the `log_library_reorder` call removed from
//! `scry_apply_decision` / `surveil_apply_decision`, the post-undo order
//! assertions and the UndoTest hash assertions FAIL; with the fix they PASS.

use mtg_engine::{
    core::{Card, CardType},
    game::{compute_undo_test_hash, GameState, ScryDecision, SurveilDecision},
    Result,
};

/// Build `n` distinct cards in P1's library and return their ids (in insertion
/// order; the last inserted is on top of the library).
fn seed_library(game: &mut GameState, n: usize) -> Vec<mtg_engine::core::CardId> {
    let p1 = game.players[0].id;
    let mut ids = Vec::new();
    for i in 0..n {
        let id = game.next_card_id();
        let mut card = Card::new(id, format!("Lib Card {i}").as_str(), p1);
        card.add_type(CardType::Land);
        game.cards.insert(id, card);
        game.get_player_zones_mut(p1).unwrap().library.add(id);
        ids.push(id);
    }
    ids
}

#[tokio::test]
async fn scry_reorder_undo_round_trip() -> Result<()> {
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    game.seed_rng(7);
    let p1 = game.players[0].id;
    seed_library(&mut game, 5);

    // Top two cards (top-down): library back is the top.
    let order_before: Vec<_> = game.get_player_zones(p1).unwrap().library.cards.clone();
    let revealed = vec![
        order_before[order_before.len() - 1],
        order_before[order_before.len() - 2],
    ];

    let hash_before = compute_undo_test_hash(&game);
    let log_len_before = game.undo_log.len();

    // Scry 2: put the current top card on the bottom, keep the other on top.
    // top/bottom are bottom-up.
    let decision = ScryDecision {
        top: [revealed[1]].into_iter().collect(),
        bottom: [revealed[0]].into_iter().collect(),
    };
    game.scry_apply_decision(p1, &revealed, &decision)?;

    let order_after: Vec<_> = game.get_player_zones(p1).unwrap().library.cards.clone();
    assert_ne!(
        order_before, order_after,
        "precondition: the scry must actually reorder the library"
    );
    assert!(
        game.undo_log.len() > log_len_before,
        "scry must log a ReorderLibrary action"
    );

    while game.undo_log.len() > log_len_before {
        game.undo()?;
    }

    assert_eq!(
        game.get_player_zones(p1).unwrap().library.cards,
        order_before,
        "mtg-ba6uq #2: undo must restore the exact pre-scry library order"
    );
    assert_eq!(
        compute_undo_test_hash(&game),
        hash_before,
        "mtg-ba6uq #2: UndoTest hash must round-trip after undoing a scry reorder"
    );

    Ok(())
}

#[tokio::test]
async fn surveil_reorder_undo_round_trip() -> Result<()> {
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    game.seed_rng(7);
    let p1 = game.players[0].id;
    seed_library(&mut game, 5);

    let revealed = game.surveil_snapshot_top_n(p1, 2).to_vec();
    assert_eq!(revealed.len(), 2, "need two cards on top to surveil");

    let lib_before: Vec<_> = game.get_player_zones(p1).unwrap().library.cards.clone();
    let gy_before: Vec<_> = game.get_player_zones(p1).unwrap().graveyard.cards.clone();
    let hash_before = compute_undo_test_hash(&game);
    let log_len_before = game.undo_log.len();

    // Surveil 2: mill the current top card to the graveyard, keep the other on
    // top. top is bottom-up; graveyard is placement order.
    let decision = SurveilDecision {
        top: [revealed[1]].into_iter().collect(),
        graveyard: [revealed[0]].into_iter().collect(),
    };
    game.surveil_apply_decision(p1, &revealed, &decision)?;

    assert!(
        game.get_player_zones(p1)
            .unwrap()
            .graveyard
            .cards
            .contains(&revealed[0]),
        "precondition: surveil must mill the chosen card to the graveyard"
    );
    assert!(
        game.undo_log.len() > log_len_before,
        "surveil must log a ReorderLibrary action"
    );

    while game.undo_log.len() > log_len_before {
        game.undo()?;
    }

    assert_eq!(
        game.get_player_zones(p1).unwrap().library.cards,
        lib_before,
        "mtg-ba6uq #2: undo must restore the exact pre-surveil library order"
    );
    assert_eq!(
        game.get_player_zones(p1).unwrap().graveyard.cards,
        gy_before,
        "mtg-ba6uq #2: undo must restore the exact pre-surveil graveyard (the milled card returns to library)"
    );
    assert_eq!(
        compute_undo_test_hash(&game),
        hash_before,
        "mtg-ba6uq #2: UndoTest hash must round-trip after undoing a surveil"
    );

    Ok(())
}
