//! Parity tests for the scry/surveil snapshot+apply refactor (Phase B).
//!
//! These pin down the behaviour of the new
//! `scry_snapshot_top_n` / `scry_apply_decision` /
//! `scry_default_heuristic_decision` trio (and the surveil counterparts) so
//! the Phase B architectural split can be proven to be a no-op for games
//! driven by the engine heuristic.
//!
//! Concretely, every test runs the LEGACY wrapper (`scry_cards` /
//! `surveil_cards`) on one game state and the SPLIT pipeline
//! (snapshot → heuristic → apply) on a clone of that state, then asserts the
//! two libraries / graveyards match exactly.

use mtg_forge_rs::core::{Card, CardId, CardType, PlayerId};
use mtg_forge_rs::game::GameState;
use smallvec::SmallVec;

/// Build a fresh two-player game with `library_cards` placed onto P1's
/// library top-down (`library_cards[0]` becomes the top of the library)
/// and `hand_cards` placed into P1's hand. Returns the constructed game.
///
/// All cards are created with `Card::new(...)`; types are set per
/// `library_types[i]`. CardIds start at 100 to avoid colliding with any
/// IDs the engine itself allocates during `new_two_player`.
fn build_game_with_library(library_cards: &[(&str, &[CardType])], hand_lands: usize) -> GameState {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // Add `hand_lands` Forests to P1's hand.
    for i in 0..hand_lands {
        let id = CardId::new(1000 + i as u32);
        let mut c = Card::new(id, "Forest", p1);
        c.set_types(SmallVec::from_vec(vec![CardType::Land]));
        game.cards.insert(id, c);
        if let Some(zones) = game.get_player_zones_mut(p1) {
            zones.hand.cards.push(id);
        }
    }

    // Add library cards. Library is bottom-up so push the LAST element
    // first to make `library_cards[0]` end up on top.
    for (i, (name, types)) in library_cards.iter().enumerate().rev() {
        let id = CardId::new(2000 + i as u32);
        let mut c = Card::new(id, *name, p1);
        c.set_types(SmallVec::from_vec(types.to_vec()));
        game.cards.insert(id, c);
        if let Some(zones) = game.get_player_zones_mut(p1) {
            zones.library.cards.push(id);
        }
    }

    game
}

/// Snapshot the TOP-DOWN library order for P1 (top-of-library first).
fn library_top_down(game: &GameState, p: PlayerId) -> Vec<CardId> {
    let zones = game.get_player_zones(p).expect("zones");
    zones.library.cards.iter().rev().copied().collect()
}

fn graveyard_bottom_up(game: &GameState, p: PlayerId) -> Vec<CardId> {
    let zones = game.get_player_zones(p).expect("zones");
    zones.graveyard.cards.to_vec()
}

#[test]
fn scry_split_matches_legacy_wrapper_no_lands_in_hand() {
    // No lands in hand → the heuristic wants lands → all revealed cards
    // (mix of land + spell) stay on top.
    let library = vec![
        ("Mountain", &[CardType::Land][..]),
        ("Lightning Bolt", &[CardType::Instant][..]),
        ("Goblin Guide", &[CardType::Creature][..]),
    ];

    let mut legacy = build_game_with_library(&library, 0);
    let mut split = legacy.clone();
    let p1 = legacy.players[0].id;

    legacy.scry_cards(p1, 2).expect("legacy scry");

    // Split path: snapshot → heuristic → apply
    let revealed = split.scry_snapshot_top_n(p1, 2);
    let decision = split.scry_default_heuristic_decision(p1, &revealed);
    split.scry_apply_decision(p1, &revealed, &decision).expect("split scry");

    assert_eq!(
        library_top_down(&legacy, p1),
        library_top_down(&split, p1),
        "library order must be byte-identical after legacy vs split scry"
    );
}

#[test]
fn scry_split_matches_legacy_wrapper_three_lands_in_hand_pushes_lands_to_bottom() {
    // 3 lands in hand → heuristic pushes excess lands to bottom.
    // Reveal 3 cards: [Mountain, Lightning Bolt, Forest]. Both lands
    // should go to bottom; the spell stays on top.
    let library = vec![
        ("Mountain", &[CardType::Land][..]),
        ("Lightning Bolt", &[CardType::Instant][..]),
        ("Forest", &[CardType::Land][..]),
        ("Filler1", &[CardType::Instant][..]),
        ("Filler2", &[CardType::Instant][..]),
    ];

    let mut legacy = build_game_with_library(&library, 3);
    let mut split = legacy.clone();
    let p1 = legacy.players[0].id;

    legacy.scry_cards(p1, 3).expect("legacy scry");

    let revealed = split.scry_snapshot_top_n(p1, 3);
    let decision = split.scry_default_heuristic_decision(p1, &revealed);
    split.scry_apply_decision(p1, &revealed, &decision).expect("split scry");

    assert_eq!(library_top_down(&legacy, p1), library_top_down(&split, p1));
}

#[test]
fn scry_split_handles_empty_library() {
    let mut legacy = build_game_with_library(&[], 0);
    let split = legacy.clone();
    let p1 = legacy.players[0].id;

    legacy.scry_cards(p1, 5).expect("legacy scry");
    let revealed = split.scry_snapshot_top_n(p1, 5);
    assert!(revealed.is_empty(), "snapshot of empty library is empty");
    // No apply call needed; verify both states still match.
    assert_eq!(library_top_down(&legacy, p1), library_top_down(&split, p1));
}

#[test]
fn surveil_split_matches_legacy_wrapper_mill_non_creature_non_land() {
    // Reveal [Lightning Bolt, Goblin Guide, Mountain]. The heuristic mills
    // Lightning Bolt (instant) and keeps Goblin Guide / Mountain on top.
    let library = vec![
        ("Lightning Bolt", &[CardType::Instant][..]),
        ("Goblin Guide", &[CardType::Creature][..]),
        ("Mountain", &[CardType::Land][..]),
        ("Filler", &[CardType::Instant][..]),
    ];

    let mut legacy = build_game_with_library(&library, 0);
    let mut split = legacy.clone();
    let p1 = legacy.players[0].id;

    legacy.surveil_cards(p1, 3).expect("legacy surveil");

    let revealed = split.surveil_snapshot_top_n(p1, 3);
    let decision = split.surveil_default_heuristic_decision(p1, &revealed);
    split
        .surveil_apply_decision(p1, &revealed, &decision)
        .expect("split surveil");

    assert_eq!(
        library_top_down(&legacy, p1),
        library_top_down(&split, p1),
        "library after surveil must match"
    );
    assert_eq!(
        graveyard_bottom_up(&legacy, p1),
        graveyard_bottom_up(&split, p1),
        "graveyard after surveil must match"
    );
}

#[test]
fn surveil_split_keeps_all_creatures_and_lands_on_top() {
    // Pure creature/land reveal → nothing milled, library order preserved
    // exactly as the legacy wrapper would leave it.
    let library = vec![
        ("Goblin Guide", &[CardType::Creature][..]),
        ("Mountain", &[CardType::Land][..]),
        ("Filler", &[CardType::Instant][..]),
    ];

    let mut legacy = build_game_with_library(&library, 0);
    let mut split = legacy.clone();
    let p1 = legacy.players[0].id;

    legacy.surveil_cards(p1, 2).expect("legacy surveil");
    let revealed = split.surveil_snapshot_top_n(p1, 2);
    let decision = split.surveil_default_heuristic_decision(p1, &revealed);
    split
        .surveil_apply_decision(p1, &revealed, &decision)
        .expect("split surveil");

    assert_eq!(library_top_down(&legacy, p1), library_top_down(&split, p1));
    assert_eq!(graveyard_bottom_up(&legacy, p1), graveyard_bottom_up(&split, p1));
    assert!(graveyard_bottom_up(&split, p1).is_empty());
}
