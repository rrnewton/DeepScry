//! WASM Network Game Initialization (Late-Binding Architecture)
//!
//! Shared helpers for creating network game state in WASM mode.
//! Used by both the FancyTUI (wasm-tui feature) and the headless AI harness.

use crate::core::{CardId, PlayerId};
use crate::game::GameState;
use crate::network::DeckCardIdRanges;

/// Initialize a game with reserved CardID slots for late-binding architecture (mtg-d0jg3)
///
/// This is the WASM equivalent of `GameInitializer::init_game_reserve_only()`.
/// It creates CardID slots upfront without instantiating cards - card identities
/// are revealed later via CardRevealed messages from the server.
///
/// CRITICAL: WASM must use this to ensure behavioral identity with native client.
pub(crate) fn init_game_reserve_only_wasm(
    p1_name: String,
    p2_name: String,
    starting_life: i32,
    ranges: &DeckCardIdRanges,
) -> GameState {
    let total_cards = ranges.total_cards() as usize;
    let mut game = GameState::new_two_player_with_capacity(p1_name, p2_name, starting_life, total_cards);

    // Reserve all CardID slots in EntityStore without instantiating cards
    game.cards
        .reserve_range(CardId::new(ranges.p1_start), ranges.p1_end - ranges.p1_start);
    game.cards
        .reserve_range(CardId::new(ranges.p2_start), ranges.p2_end - ranges.p2_start);

    // Create CardID vectors for each player's library
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    let p1_card_ids: Vec<CardId> = (ranges.p1_start..ranges.p1_end).map(CardId::new).collect();
    let p2_card_ids: Vec<CardId> = (ranges.p2_start..ranges.p2_end).map(CardId::new).collect();

    // Add CardIDs to libraries
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        for card_id in p1_card_ids {
            zones.library.add(card_id);
        }
    }
    if let Some(zones) = game.get_player_zones_mut(p2_id) {
        for card_id in p2_card_ids {
            zones.library.add(card_id);
        }
    }

    // CRITICAL: Match server behavior - set skip_reveals=false so draw_card() logs RevealCard
    // actions to the undo_log. Both server and native clients do this to keep action_count in sync.
    // Without this, the WASM shadow game has fewer actions than the server, causing hash mismatches
    // at every choice point (action_count is included in the state hash).
    game.set_skip_reveals(false);

    log::info!(
        "init_game_reserve_only_wasm: Created game with {} reserved CardID slots (P1: {}, P2: {})",
        total_cards,
        ranges.p1_end - ranges.p1_start,
        ranges.p2_end - ranges.p2_start
    );

    game
}

/// Process a card reveal in WASM network mode
///
/// Wrapper around the shared `process_card_reveal` that uses WASM-specific
/// card definition provider (requires server to provide definitions).
pub(crate) fn process_card_reveal_wasm(
    game: &mut GameState,
    owner: PlayerId,
    card_reveal: crate::network::CardReveal,
    reason: crate::network::RevealReason,
    local_player: Option<PlayerId>,
) {
    use crate::network::{process_card_reveal, WasmCardDefProvider};

    let provider = WasmCardDefProvider;
    process_card_reveal(game, &provider, owner, card_reveal, reason, "WASM", local_player);
}
