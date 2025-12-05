//! Shared display functions for game state visualization
//!
//! This module provides standalone display functions that can be used by both
//! the local GameLoop and network clients to render battlefield state.

use crate::core::{Keyword, PlayerId};
use crate::game::GameState;

/// Print detailed battlefield state for all players
///
/// This displays:
/// - Player life totals and zone sizes
/// - Hand contents for the specified viewer (or active player if None)
/// - Battlefield permanents sorted by type (lands, creatures, others)
///
/// # Arguments
/// * `game` - The game state to display
/// * `viewer` - Optional player ID whose hand contents to show (if None, shows active player's hand)
pub fn print_battlefield_state(game: &GameState, viewer: Option<PlayerId>) {
    // Print state for each player
    for player in game.players.iter() {
        let is_active = player.id == game.turn.active_player;
        let marker = if is_active { " (active)" } else { "" };

        println!("{}{}: ", player.name, marker);
        println!("  Life: {}", player.life);

        // Zone sizes
        if let Some(zones) = game.get_player_zones(player.id) {
            println!(
                "  Hand: {} | Library: {} | Graveyard: {} | Exile: {}",
                zones.hand.len(),
                zones.library.len(),
                zones.graveyard.len(),
                zones.exile.len()
            );

            // Show hand contents for viewer (or active player if not specified)
            let show_hand = viewer.map(|v| v == player.id).unwrap_or(is_active);
            if show_hand && !zones.hand.is_empty() {
                println!("  Hand contents:");
                for &card_id in &zones.hand.cards {
                    if let Ok(card) = game.cards.get(card_id) {
                        println!("    - {} ({})", card.name, card.mana_cost);
                    }
                }
            }
        }

        // Battlefield permanents controlled by this player
        let mut player_permanents: Vec<_> = game
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                game.cards.get(card_id).ok().and_then(|card| {
                    if card.controller == player.id {
                        Some((card_id, card))
                    } else {
                        None
                    }
                })
            })
            .collect();

        // Sort by card type for better readability: lands first, then creatures, then others
        player_permanents.sort_by_key(|(_, card)| {
            if card.is_land() {
                0
            } else if card.is_creature() {
                1
            } else {
                2
            }
        });

        if player_permanents.is_empty() {
            println!("  Battlefield: (empty)");
        } else {
            println!("  Battlefield:");
            for (card_id, card) in player_permanents {
                let tap_status = if card.tapped { " (tapped)" } else { "" };

                // Check for summoning sickness (creatures that entered this turn and don't have haste)
                let has_summoning_sickness = if card.is_creature() {
                    if let Some(entered_turn) = card.turn_entered_battlefield {
                        entered_turn == game.turn.turn_number && !card.has_keyword(Keyword::Haste)
                    } else {
                        false
                    }
                } else {
                    false
                };
                let sickness_status = if has_summoning_sickness {
                    " (summoning sickness)"
                } else {
                    ""
                };

                // Format card display based on type
                if card.is_creature() {
                    // Use get_effective_power/toughness to include all continuous effects
                    // (anthems, equipment, auras, counters) via CR 613 layer system
                    let power = game.get_effective_power(card_id).unwrap_or(card.current_power() as i32);
                    let toughness = game
                        .get_effective_toughness(card_id)
                        .unwrap_or(card.current_toughness() as i32);
                    println!(
                        "    {} ({}) - {}/{}{}{}",
                        card.name, card_id, power, toughness, tap_status, sickness_status
                    );
                } else {
                    println!("    {} ({}){}", card.name, card_id, tap_status);
                }
            }
        }
    }
    println!();
}

/// Print a separator line with optional title
pub fn print_separator(title: Option<&str>) {
    match title {
        Some(t) => {
            println!("════════════════════════════════════════════════════════════════");
            println!("                      {}", t);
            println!("════════════════════════════════════════════════════════════════");
        }
        None => {
            println!("════════════════════════════════════════════════════════════════");
        }
    }
}
