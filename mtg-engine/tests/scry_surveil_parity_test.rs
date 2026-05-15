//! Parity tests for the scry/surveil controller pipeline (Phase B/C).
//!
//! After Phase C the heuristic lives entirely in
//! [`HeuristicController::choose_scry_order`] /
//! [`HeuristicController::choose_surveil`]. These tests pin down the
//! invariant that:
//!
//!   1. `GameState::scry_snapshot_top_n` returns the top N cards
//!      top-down without mutating anything;
//!   2. `HeuristicController::choose_scry_order` produces the same
//!      partition the legacy engine-baked heuristic produced
//!      (lands-to-bottom-when-≥3-in-hand);
//!   3. `GameState::scry_apply_decision` applies the partition with
//!      the same library order the legacy code emitted (preserving
//!      the legacy reordering quirk for byte-identical behaviour).
//!
//! The PRE-Phase-B `scry_cards` wrapper has been removed in Phase C, so
//! we no longer have a "legacy" path to diff against — instead each
//! test asserts the EXPECTED library / graveyard layout produced by the
//! split pipeline, with the expected layout derived from the rules
//! summarised above.

use mtg_forge_rs::core::{Card, CardId, CardType, PlayerId};
use mtg_forge_rs::game::{controller::ChoiceResult, GameState, GameStateView, HeuristicController, PlayerController};
use smallvec::SmallVec;

/// Build a fresh two-player game with `library_cards` placed onto P1's
/// library top-down (`library_cards[0]` becomes the top of the library)
/// and `hand_lands` Forests placed into P1's hand. Returns the
/// constructed game.
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

/// Snapshot the TOP-DOWN library order for `p` (top of library first).
fn library_top_down(game: &GameState, p: PlayerId) -> Vec<CardId> {
    let zones = game.get_player_zones(p).expect("zones");
    zones.library.cards.iter().rev().copied().collect()
}

fn graveyard(game: &GameState, p: PlayerId) -> Vec<CardId> {
    let zones = game.get_player_zones(p).expect("zones");
    zones.graveyard.cards.to_vec()
}

/// Run the full Phase C scry pipeline:
///   snapshot → HeuristicController → apply.
fn run_heuristic_scry(game: &mut GameState, player: PlayerId, count: u8) {
    let revealed = game.scry_snapshot_top_n(player, count);
    if revealed.is_empty() {
        return;
    }
    let mut controller = HeuristicController::new(player);
    let view = GameStateView::new(game, player);
    let decision = match controller.choose_scry_order(&view, &revealed) {
        ChoiceResult::Ok(d) => d,
        ChoiceResult::UndoRequest(_) | ChoiceResult::ExitGame | ChoiceResult::Error(_) | ChoiceResult::NeedInput(_) => {
            panic!("HeuristicController::choose_scry_order returned non-Ok variant in test")
        }
    };
    game.scry_apply_decision(player, &revealed, &decision)
        .expect("apply scry");
}

fn run_heuristic_surveil(game: &mut GameState, player: PlayerId, count: u8) {
    let revealed = game.surveil_snapshot_top_n(player, count);
    if revealed.is_empty() {
        return;
    }
    let mut controller = HeuristicController::new(player);
    let view = GameStateView::new(game, player);
    let decision = match controller.choose_surveil(&view, &revealed) {
        ChoiceResult::Ok(d) => d,
        ChoiceResult::UndoRequest(_) | ChoiceResult::ExitGame | ChoiceResult::Error(_) | ChoiceResult::NeedInput(_) => {
            panic!("HeuristicController::choose_surveil returned non-Ok variant in test")
        }
    };
    game.surveil_apply_decision(player, &revealed, &decision)
        .expect("apply surveil");
}

#[test]
fn scry_no_lands_in_hand_keeps_all_on_top() {
    // Empty hand → heuristic wants more lands → all revealed cards
    // (mix of land + spell) stay on top. The legacy reordering quirk
    // is preserved: the cards end up in revealed-order on top, so the
    // last revealed card becomes the new top of library.
    let library = vec![
        ("Mountain", &[CardType::Land][..]),          // CardId 2000 — initial top
        ("Lightning Bolt", &[CardType::Instant][..]), // CardId 2001
        ("Goblin Guide", &[CardType::Creature][..]),  // CardId 2002 — third from top
    ];

    let mut game = build_game_with_library(&library, 0);
    let p1 = game.players[0].id;

    run_heuristic_scry(&mut game, p1, 2);

    // After the engine's "keep all" path: removed [Mountain, Bolt],
    // pushed Mountain then Bolt back. Library top is now Bolt (CardId 2001),
    // then Mountain (CardId 2000), then untouched Goblin Guide (CardId 2002).
    let top_down = library_top_down(&game, p1);
    assert_eq!(
        &top_down[..3],
        &[CardId::new(2001), CardId::new(2000), CardId::new(2002)],
        "heuristic with 0 lands in hand keeps both revealed on top in revealed order"
    );
}

#[test]
fn scry_three_lands_in_hand_pushes_lands_to_bottom() {
    // 3 lands in hand → heuristic doesn't want more lands → revealed
    // lands go to bottom; revealed spell stays on top.
    let library = vec![
        ("Mountain", &[CardType::Land][..]),          // 2000
        ("Lightning Bolt", &[CardType::Instant][..]), // 2001
        ("Forest", &[CardType::Land][..]),            // 2002
        ("Filler1", &[CardType::Instant][..]),        // 2003
        ("Filler2", &[CardType::Instant][..]),        // 2004
    ];

    let mut game = build_game_with_library(&library, 3);
    let p1 = game.players[0].id;

    run_heuristic_scry(&mut game, p1, 3);

    // Both Mountain (2000) and Forest (2002) go to bottom.
    // Lightning Bolt (2001) stays on top.
    // Filler1 (2003) and Filler2 (2004) untouched in the middle.
    //
    // Library after (top-down): [Bolt, Filler1, Filler2, …, Mountain, Forest]
    // (bottom: Mountain at deepest, then Forest above it).
    let top_down = library_top_down(&game, p1);

    // Top of library is Lightning Bolt.
    assert_eq!(top_down[0], CardId::new(2001), "Bolt remains on top");
    // Bottom of library: Mountain at absolute bottom, Forest just above.
    let last = top_down.len() - 1;
    assert_eq!(top_down[last], CardId::new(2000), "Mountain at absolute bottom");
    assert_eq!(top_down[last - 1], CardId::new(2002), "Forest just above bottom");
}

#[test]
fn scry_handles_empty_library() {
    let mut game = build_game_with_library(&[], 0);
    let p1 = game.players[0].id;

    let revealed = game.scry_snapshot_top_n(p1, 5);
    assert!(revealed.is_empty(), "snapshot of empty library is empty");

    // Apply path is a no-op for empty revealed slice.
    run_heuristic_scry(&mut game, p1, 5);
    assert!(library_top_down(&game, p1).is_empty());
}

#[test]
fn surveil_mills_non_creature_non_land() {
    // Reveal [Lightning Bolt, Goblin Guide, Mountain]. The heuristic
    // mills Lightning Bolt (instant) and keeps Goblin Guide / Mountain
    // on top.
    let library = vec![
        ("Lightning Bolt", &[CardType::Instant][..]), // 2000
        ("Goblin Guide", &[CardType::Creature][..]),  // 2001
        ("Mountain", &[CardType::Land][..]),          // 2002
        ("Filler", &[CardType::Instant][..]),         // 2003 — untouched
    ];

    let mut game = build_game_with_library(&library, 0);
    let p1 = game.players[0].id;

    run_heuristic_surveil(&mut game, p1, 3);

    // Graveyard: only Lightning Bolt.
    assert_eq!(graveyard(&game, p1), vec![CardId::new(2000)]);

    // Library top-down after surveil:
    //   - Filler (2003) was untouched at the bottom of the original top-3 layer
    //   - Goblin Guide (2001) and Mountain (2002) re-pushed in revealed order;
    //     Mountain ends up on top per the legacy ordering quirk.
    let top_down = library_top_down(&game, p1);
    assert_eq!(
        &top_down[..],
        &[CardId::new(2002), CardId::new(2001), CardId::new(2003)]
    );
}

#[test]
fn surveil_keeps_all_creatures_and_lands_on_top() {
    // Pure creature/land reveal → nothing milled.
    let library = vec![
        ("Goblin Guide", &[CardType::Creature][..]), // 2000
        ("Mountain", &[CardType::Land][..]),         // 2001
        ("Filler", &[CardType::Instant][..]),        // 2002 — untouched
    ];

    let mut game = build_game_with_library(&library, 0);
    let p1 = game.players[0].id;

    run_heuristic_surveil(&mut game, p1, 2);

    assert!(graveyard(&game, p1).is_empty());

    // Goblin Guide + Mountain re-pushed in revealed order; Mountain ends
    // up on top (legacy quirk preserved).
    let top_down = library_top_down(&game, p1);
    assert_eq!(
        &top_down[..],
        &[CardId::new(2001), CardId::new(2000), CardId::new(2002)]
    );
}
