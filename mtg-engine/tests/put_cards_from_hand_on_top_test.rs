//! Regression tests for [`Effect::PutCardsFromHandOnTopOfLibrary`] (B13 fix).
//!
//! Brainstorm's sub-ability ("put two cards from your hand on top of your
//! library in any order") was previously silently dropped — the effect was
//! never executed, so cards never moved Hand→Library.
//!
//! These tests verify the public `execute_effect` dispatch path via
//! [`Effect::PutCardsFromHandOnTopOfLibrary`], confirming that:
//!
//! 1. Cards are removed from the player's hand.
//! 2. Cards appear in the player's library.
//! 3. The correct number of cards is moved.
//! 4. An empty hand is handled gracefully (moves zero cards, no panic).
//!
//! MTG rules reference: CR 401.4 ("…the owner may arrange them in any order
//! — that library's owner doesn't reveal the order chosen").

use mtg_engine::{
    core::{Card, CardId, CardType, Effect, ManaCost, PlayerId},
    game::GameState,
    Result,
};

/// Build a minimal two-player game with `hand_size` cards in P1's hand and
/// `library_size` land cards in P1's library.
///
/// Hand cards get CMC equal to their index (index 0 → CMC 0, etc.) so the
/// heuristic's lowest-CMC sort is predictable.
/// Returns `(game, p1, hand_ids)` where `hand_ids[i]` has CMC `i`.
fn setup_game(hand_size: usize, library_size: usize) -> (GameState, PlayerId, Vec<CardId>) {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    game.seed_rng(42);
    let p1 = game.players[0].id;

    // Library: plain lands (non-empty baseline).
    for i in 0..library_size {
        let id = game.next_card_id();
        let mut c = Card::new(id, format!("Lib Land {i}").as_str(), p1);
        c.add_type(CardType::Land);
        game.cards.insert(id, c);
        game.get_player_zones_mut(p1).unwrap().library.add(id);
    }

    // Hand: instants with generic CMC equal to index i.
    let mut hand_ids = Vec::with_capacity(hand_size);
    for i in 0..hand_size {
        let id = game.next_card_id();
        let mut c = Card::new(id, format!("Hand Card {i} (CMC={i})").as_str(), p1);
        c.add_type(CardType::Instant);
        c.mana_cost = ManaCost {
            generic: i as u8,
            ..ManaCost::new()
        };
        game.cards.insert(id, c);
        game.get_player_zones_mut(p1).unwrap().hand.add(id);
        hand_ids.push(id);
    }

    (game, p1, hand_ids)
}

// ─── Test 1: 2 cards move from hand to library ───────────────────────────────

#[tokio::test]
async fn two_cards_move_from_hand_to_library() -> Result<()> {
    let (mut game, p1, _hand_ids) = setup_game(4, 2);

    let hand_before = game.get_player_zones(p1).unwrap().hand.cards.len();
    let lib_before = game.get_player_zones(p1).unwrap().library.cards.len();

    game.execute_effect(&Effect::PutCardsFromHandOnTopOfLibrary { player: p1, count: 2 })?;

    let hand_after = game.get_player_zones(p1).unwrap().hand.cards.len();
    let lib_after = game.get_player_zones(p1).unwrap().library.cards.len();

    assert_eq!(hand_after, hand_before - 2, "exactly 2 cards must leave the hand");
    assert_eq!(lib_after, lib_before + 2, "exactly 2 cards must arrive in the library");

    Ok(())
}

// ─── Test 2: heuristic picks lowest-CMC cards (observable via library) ───────

#[tokio::test]
async fn lowest_cmc_cards_are_placed_on_library() -> Result<()> {
    // 5 hand cards with CMC 0..=4; 2 should go back.
    // The heuristic chooses CMC=0 and CMC=1 (deterministic).
    let (mut game, p1, hand_ids) = setup_game(5, 2);

    game.execute_effect(&Effect::PutCardsFromHandOnTopOfLibrary { player: p1, count: 2 })?;

    let lib_cards = game.get_player_zones(p1).unwrap().library.cards.clone();

    // Both CMC=0 and CMC=1 cards must be in the library.
    assert!(
        lib_cards.contains(&hand_ids[0]),
        "CMC=0 card must be put on top of library"
    );
    assert!(
        lib_cards.contains(&hand_ids[1]),
        "CMC=1 card must be put on top of library"
    );

    // CMC=2..=4 cards should remain in hand.
    let hand_cards = game.get_player_zones(p1).unwrap().hand.cards.clone();
    for &high_cmc in &hand_ids[2..] {
        assert!(hand_cards.contains(&high_cmc), "high-CMC card must stay in hand");
    }

    Ok(())
}

// ─── Test 3: empty hand — no panic, zero cards moved ─────────────────────────

#[tokio::test]
async fn empty_hand_no_panic() -> Result<()> {
    let (mut game, p1, _) = setup_game(0, 3); // no hand cards

    let lib_before = game.get_player_zones(p1).unwrap().library.cards.len();

    // Should not panic; just be a no-op because there are no cards to put back.
    game.execute_effect(&Effect::PutCardsFromHandOnTopOfLibrary { player: p1, count: 2 })?;

    let lib_after = game.get_player_zones(p1).unwrap().library.cards.len();
    assert_eq!(
        lib_after, lib_before,
        "library size must be unchanged when hand is empty"
    );

    Ok(())
}

// ─── Test 4: count=1 puts exactly one card back ──────────────────────────────

#[tokio::test]
async fn count_one_puts_exactly_one_card_back() -> Result<()> {
    let (mut game, p1, _) = setup_game(3, 2);

    let hand_before = game.get_player_zones(p1).unwrap().hand.cards.len();
    let lib_before = game.get_player_zones(p1).unwrap().library.cards.len();

    game.execute_effect(&Effect::PutCardsFromHandOnTopOfLibrary { player: p1, count: 1 })?;

    let hand_after = game.get_player_zones(p1).unwrap().hand.cards.len();
    let lib_after = game.get_player_zones(p1).unwrap().library.cards.len();

    assert_eq!(hand_after, hand_before - 1, "exactly 1 card must leave the hand");
    assert_eq!(lib_after, lib_before + 1, "exactly 1 card must arrive in the library");

    Ok(())
}
