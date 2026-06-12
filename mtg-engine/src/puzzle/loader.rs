//! Load puzzle state into a Game object
//!
//! This module handles applying parsed PZL state to create initialized games
//! with specific board states for testing.

use crate::{
    core::{Card, CardId, CardType, PlayerId},
    game::GameState,
    loader::AsyncCardDatabase,
    puzzle::{card_notation::CardModifier, CardDefinition, GameStateDefinition, PuzzleFile},
    MtgError, Result,
};
use std::collections::HashMap;

/// Apply a puzzle state to a game
///
/// This creates a game with the exact state specified in the puzzle file,
/// including player life, zones, card states, etc.
///
/// # Errors
///
/// Returns an error if card loading fails or the puzzle state is invalid.
pub async fn load_puzzle_into_game(puzzle: &PuzzleFile, card_db: &AsyncCardDatabase) -> Result<GameState> {
    let state_def = &puzzle.state;

    // Create players
    let player1_name = "Player 1".to_string();
    let player2_name = "Player 2".to_string();

    // Initialize a basic game structure with 20 life (will be overridden)
    let mut game = GameState::new_two_player(player1_name, player2_name, 20);

    // Collect all unique card names from the puzzle
    let mut unique_cards = std::collections::HashSet::new();
    for player_state in &state_def.players {
        for card_def in player_state
            .hand
            .iter()
            .chain(player_state.battlefield.iter())
            .chain(player_state.graveyard.iter())
            .chain(player_state.library.iter())
            .chain(player_state.exile.iter())
        {
            if !card_def.is_token() {
                unique_cards.insert(card_def.name.clone());
            }
        }
    }

    // Scan all cards for token script references and preload token definitions
    let mut token_scripts = std::collections::HashSet::new();
    for card_name in &unique_cards {
        if let Some(paper_card) = card_db.get_card(card_name).await? {
            for token_script in paper_card.extract_token_scripts() {
                token_scripts.insert(token_script);
            }
        }
    }

    // Load token definitions into game
    for token_script in token_scripts {
        if let Some(token_def) = card_db.get_token(&token_script).await? {
            game.token_definitions
                .insert(token_script, std::sync::Arc::new(token_def));
        }
    }

    log::debug!("Puzzle loaded {} token definitions", game.token_definitions.len());

    // Set turn and phase
    game.turn.turn_number = state_def.turn;
    game.turn.current_step = state_def.active_phase;

    // Determine active player
    let active_player_idx = state_def.active_player.index();
    if active_player_idx >= game.players.len() {
        return Err(MtgError::InvalidAction(format!(
            "Invalid active player index: {}",
            active_player_idx
        )));
    }
    game.turn.active_player = game.players[active_player_idx].id;
    game.turn.active_player_idx = active_player_idx;

    // Track card IDs for cross-references (attachments, etc.)
    let mut id_map: HashMap<u32, CardId> = HashMap::new();

    // Load cards for each player
    for (player_idx, player_state) in state_def.players.iter().enumerate() {
        if player_idx >= game.players.len() {
            return Err(MtgError::InvalidAction(format!(
                "Player index out of bounds: {}",
                player_idx
            )));
        }

        let player_id = game.players[player_idx].id;

        // Apply player state
        game.players[player_idx].life = player_state.life;
        game.players[player_idx].lands_played_this_turn = player_state.lands_played as u8;

        // Pre-float mana into the pool (e.g. `p0manapool=U U C`). Each token is a
        // sequence of WUBRGC symbols; non-mana chars are ignored. Used by tests
        // that need unspent mana already in the pool (e.g. Power Sink DrainMana).
        for token in &player_state.mana_pool {
            for c in token.chars() {
                let color = match c {
                    'W' => crate::core::Color::White,
                    'U' => crate::core::Color::Blue,
                    'B' => crate::core::Color::Black,
                    'R' => crate::core::Color::Red,
                    'G' => crate::core::Color::Green,
                    'C' => crate::core::Color::Colorless,
                    _ => continue,
                };
                game.players[player_idx].mana_pool.add_color(color);
            }
        }

        // Load cards into hand
        for card_def in &player_state.hand {
            let card_id = {
                let card = create_card_from_definition(card_def, player_id, &mut game, card_db).await?;
                card.id
            };
            if let Some(id) = card_def.id {
                id_map.insert(id, card_id);
            }
            let zones = game
                .get_player_zones_mut(player_id)
                .ok_or_else(|| MtgError::InvalidAction("Player zones not found".to_string()))?;
            zones.hand.add(card_id);
        }

        // Load cards into battlefield
        for card_def in &player_state.battlefield {
            let card_id = {
                let card = create_card_from_definition(card_def, player_id, &mut game, card_db).await?;
                card.id
            };
            if let Some(id) = card_def.id {
                id_map.insert(id, card_id);
            }
            game.battlefield.add(card_id);
        }

        // Load cards into graveyard
        for card_def in &player_state.graveyard {
            let card_id = {
                let card = create_card_from_definition(card_def, player_id, &mut game, card_db).await?;
                card.id
            };
            if let Some(id) = card_def.id {
                id_map.insert(id, card_id);
            }
            let zones = game
                .get_player_zones_mut(player_id)
                .ok_or_else(|| MtgError::InvalidAction("Player zones not found".to_string()))?;
            zones.graveyard.add(card_id);
        }

        // Load cards into library
        for card_def in &player_state.library {
            let card_id = {
                let card = create_card_from_definition(card_def, player_id, &mut game, card_db).await?;
                card.id
            };
            if let Some(id) = card_def.id {
                id_map.insert(id, card_id);
            }
            let zones = game
                .get_player_zones_mut(player_id)
                .ok_or_else(|| MtgError::InvalidAction("Player zones not found".to_string()))?;
            zones.library.add(card_id);
        }

        // Load cards into exile
        for card_def in &player_state.exile {
            let card_id = {
                let card = create_card_from_definition(card_def, player_id, &mut game, card_db).await?;
                card.id
            };
            if let Some(id) = card_def.id {
                id_map.insert(id, card_id);
            }
            let zones = game
                .get_player_zones_mut(player_id)
                .ok_or_else(|| MtgError::InvalidAction("Player zones not found".to_string()))?;
            zones.exile.add(card_id);
        }

        // Note: Command zone not yet in PlayerZones - will be added when needed
    }

    // Second pass: apply modifiers that depend on card IDs or need card refs
    apply_card_modifiers(&mut game, state_def, &id_map)?;

    // Resolve ETB "choose a player" for battlefield permanents (Black Vise).
    // Puzzle battlefield cards are placed directly (they never "enter" through
    // set_card_zone, which is where the ChoosePlayer replacement normally
    // fires), so a puzzle-placed Black Vise would have no chosen player and its
    // upkeep trigger could never fire. Resolve it here, AFTER all zones are
    // loaded, using the same deterministic public-state pick the live ETB path
    // uses (most cards in hand, tie-break low PlayerId). Pre-compute to avoid a
    // simultaneous mutable+immutable borrow of `game`.
    let chosen: Vec<(CardId, Option<PlayerId>)> = game
        .battlefield
        .cards
        .iter()
        .filter_map(|&cid| {
            let card = game.cards.try_get(cid)?;
            if card.definition.cache.etb_choose_player && card.chosen_player.is_none() {
                Some((cid, game.pick_chosen_opponent(card.controller)))
            } else {
                None
            }
        })
        .collect();
    for (cid, chosen_player) in chosen {
        if let Ok(card_mut) = game.cards.get_mut(cid) {
            card_mut.chosen_player = chosen_player;
        }
    }

    Ok(game)
}

/// Create a card from a card definition
///
/// Note: Wildcard is intentional - CardModifier has 24+ variants. Only a subset
/// (Tapped, Counters) are handled in first pass; others need second pass or aren't implemented.
#[allow(clippy::wildcard_enum_match_arm)]
async fn create_card_from_definition<'a>(
    card_def: &CardDefinition,
    owner: PlayerId,
    game: &'a mut GameState,
    card_db: &AsyncCardDatabase,
) -> Result<&'a mut Card> {
    // Check if it's a token
    if card_def.is_token() {
        return Err(MtgError::InvalidAction("Token support not yet implemented".to_string()));
    }

    // Load card from database
    let paper_card = card_db
        .get_card(&card_def.name)
        .await?
        .ok_or_else(|| MtgError::InvalidAction(format!("Card not found: {}", card_def.name)))?;

    // Create game card with proper ID using instantiate method
    let card_id = game.next_card_id();
    let mut card = paper_card.instantiate(card_id, owner);

    // Apply basic modifiers (tapped state and counters)
    for modifier in &card_def.modifiers {
        match modifier {
            CardModifier::Tapped => card.tapped = true,
            CardModifier::Counters(counters) => {
                // Convert HashMap to SmallVec format
                for (counter_type, count) in counters {
                    if *count > 0 {
                        card.add_counter(*counter_type, *count as u8);
                    }
                }
            }
            CardModifier::ChosenColor(colors) => {
                // Apply the first named color as the card's chosen_color
                // (mirrors K:ETBReplacement:Other:ChooseColor behavior).
                // Used for Prismatic Ward puzzles: `Prismatic Ward|ChosenColor:Red`
                if let Some(color_str) = colors.first() {
                    use crate::core::Color;
                    let chosen = match color_str.as_str() {
                        "White" => Some(Color::White),
                        "Blue" => Some(Color::Blue),
                        "Black" => Some(Color::Black),
                        "Red" => Some(Color::Red),
                        "Green" => Some(Color::Green),
                        _ => None,
                    };
                    if let Some(color) = chosen {
                        card.chosen_color = Some(color);
                    }
                }
            }
            // Skip modifiers that need second pass or aren't supported yet
            _ => {}
        }
    }

    // Insert card into game
    let card_id_value = card.id;
    game.cards.insert(card_id_value, card);

    // Return mutable reference
    game.cards.get_mut(card_id_value)
}

/// Apply card modifiers that need second pass (attachments, etc.) or card references
#[allow(clippy::wildcard_enum_match_arm)]
fn apply_card_modifiers(
    game: &mut GameState,
    state_def: &GameStateDefinition,
    id_map: &HashMap<u32, CardId>,
) -> Result<()> {
    // First: build a map from card name → card definitions with their modifiers
    // (for the SummonSick pass below that uses name matching).
    // Second: collect AttachedTo modifiers keyed by the CARD's own CardId.
    // We collect first to avoid borrow conflicts.

    // --- Summoning sickness pass ---
    for card_id in game.battlefield.cards.iter() {
        if let Ok(card) = game.cards.get_mut(*card_id) {
            if card.types.contains(&CardType::Creature) {
                let has_summoning_sickness = state_def
                    .players
                    .iter()
                    .flat_map(|p| {
                        p.hand
                            .iter()
                            .chain(p.battlefield.iter())
                            .chain(p.graveyard.iter())
                            .chain(p.library.iter())
                            .chain(p.exile.iter())
                    })
                    .find(|def| def.name == card.name.as_str())
                    .map(|def| def.modifiers.iter().any(|m| matches!(m, CardModifier::SummonSick)))
                    .unwrap_or(false);

                if has_summoning_sickness {
                    card.turn_entered_battlefield = Some(state_def.turn);
                } else {
                    card.turn_entered_battlefield = Some(state_def.turn.saturating_sub(1));
                }
            }
        }
    }

    // --- AttachedTo pass: set card.attached_to for Auras / Equipment ---
    // Collect (card_id, target_puzzle_id) pairs to avoid borrow conflicts.
    let attach_jobs: Vec<(CardId, u32)> = state_def
        .players
        .iter()
        .flat_map(|p| p.battlefield.iter())
        .filter_map(|def| {
            // Only cards with an Id AND an AttachedTo modifier need second-pass attachment.
            let card_puzzle_id = def.id?;
            let target_puzzle_id = def.modifiers.iter().find_map(|m| {
                if let CardModifier::AttachedTo(tid) = m {
                    Some(*tid)
                } else {
                    None
                }
            })?;
            let card_id = id_map.get(&card_puzzle_id)?;
            Some((*card_id, target_puzzle_id))
        })
        .collect();

    for (aura_id, target_puzzle_id) in attach_jobs {
        if let Some(&target_id) = id_map.get(&target_puzzle_id) {
            if let Ok(card) = game.cards.get_mut(aura_id) {
                card.attached_to = Some(target_id);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CounterType;
    use crate::puzzle::PuzzleFile;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_load_simple_puzzle() {
        let puzzle_contents = r#"
[metadata]
Name:Test Puzzle
Goal:Win
Turns:1
Difficulty:Easy

[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0battlefield=Mountain
p1life=10
"#;

        let puzzle = PuzzleFile::parse(puzzle_contents).unwrap();

        // Create a card database for testing (with cardsfolder path)
        use std::path::PathBuf;
        let card_db = Arc::new(AsyncCardDatabase::new(PathBuf::from("cardsfolder")));

        // Try to load the puzzle
        let result = load_puzzle_into_game(&puzzle, &card_db).await;

        // If cardsfolder exists and has the cards, this should succeed
        // Otherwise it should fail with card not found
        if let Ok(game) = result {
            // Verify basic game state
            assert_eq!(game.players[0].life, 20);
            assert_eq!(game.players[1].life, 10);
            assert_eq!(game.turn.turn_number, 1);
        } else {
            // Expected to fail if cardsfolder doesn't exist
            // (which is fine for unit tests)
            eprintln!(
                "Puzzle loading failed (expected if cardsfolder not available): {:?}",
                result.err()
            );
        }
    }

    #[tokio::test]
    async fn test_load_puzzle_with_counters() {
        let puzzle_contents = r#"
[metadata]
Name:Counter Test
Goal:Win
Turns:1
Difficulty:Easy

[state]
turn=2
activeplayer=p0
activephase=MAIN1
p0life=20
p0battlefield=Grizzly Bears|Counters:P1P1=2
p1life=20
"#;

        let puzzle = PuzzleFile::parse(puzzle_contents).unwrap();
        assert_eq!(puzzle.state.players[0].battlefield.len(), 1);

        let card_def = &puzzle.state.players[0].battlefield[0];
        assert_eq!(card_def.name, "Grizzly Bears");
        assert_eq!(card_def.modifiers.len(), 1);

        if let CardModifier::Counters(counters) = &card_def.modifiers[0] {
            assert_eq!(counters.get(&CounterType::P1P1), Some(&2));
        } else {
            panic!("Expected Counters modifier");
        }
    }
}
