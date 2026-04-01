//! Shared display functions for game state visualization
//!
//! This module provides standalone display functions that can be used by both
//! the local GameLoop and network clients to render battlefield state.

use crate::core::{Keyword, PlayerId};
use crate::game::controller::GameStateView;
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
            if zones.command.is_empty() {
                println!(
                    "  Hand: {} | Library: {} | Graveyard: {} | Exile: {}",
                    zones.hand.len(),
                    zones.library.len(),
                    zones.graveyard.len(),
                    zones.exile.len()
                );
            } else {
                // Show command zone info for Commander games
                let cmd_names: Vec<String> = zones.command.cards.iter()
                    .filter_map(|&cid| game.cards.try_get(cid).map(|c| c.name.to_string()))
                    .collect();
                println!(
                    "  Hand: {} | Library: {} | Graveyard: {} | Exile: {} | Command: [{}]",
                    zones.hand.len(),
                    zones.library.len(),
                    zones.graveyard.len(),
                    zones.exile.len(),
                    cmd_names.join(", ")
                );
            }

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
                    let power = game
                        .get_effective_power(card_id)
                        .unwrap_or_else(|_| i32::from(card.current_power()));
                    let toughness = game
                        .get_effective_toughness(card_id)
                        .unwrap_or_else(|_| i32::from(card.current_toughness()));
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

/// Format battlefield state as a string for logging
///
/// This is a shared function used by both native and WASM fancy TUI
/// to display the battlefield when the user presses 'b'.
pub fn format_battlefield_for_log(view: &GameStateView) -> String {
    use std::fmt::Write;
    let mut output = String::new();

    writeln!(output, "=== Battlefield ===").unwrap();

    // Get all players
    let player_id = view.player_id();
    let opponent_id = view.opponents().next();

    // Format player's battlefield
    writeln!(output, "\nYou (P1):").unwrap();
    writeln!(output, "  Life: {}", view.player_life(player_id)).unwrap();
    format_player_battlefield(&mut output, view, player_id);

    // Format opponent's battlefield
    if let Some(opp_id) = opponent_id {
        writeln!(output, "\nOpponent (P2):").unwrap();
        writeln!(output, "  Life: {}", view.player_life(opp_id)).unwrap();
        format_player_battlefield(&mut output, view, opp_id);
    }

    output
}

/// Format a single player's battlefield for the log
fn format_player_battlefield(output: &mut String, view: &GameStateView, player_id: PlayerId) {
    use std::fmt::Write;

    // Collect cards controlled by this player
    let bf = view.battlefield();
    let mut lands = Vec::new();
    let mut creatures = Vec::new();
    let mut other = Vec::new();

    for &card_id in bf {
        if let Some(card) = view.get_card(card_id) {
            if card.controller != player_id {
                continue;
            }

            let name = view.card_name(card_id).unwrap_or_else(|| "Unknown".to_string());
            let tapped = if view.is_tapped(card_id) { " (T)" } else { "" };

            if card.is_land() {
                lands.push(format!("    {}{}", name, tapped));
            } else if card.is_creature() {
                // Use effective P/T which includes continuous effects
                let power = view
                    .get_effective_power(card_id)
                    .unwrap_or_else(|| i32::from(card.current_power()));
                let toughness = view
                    .get_effective_toughness(card_id)
                    .unwrap_or_else(|| i32::from(card.current_toughness()));
                let base_power = i32::from(card.base_power().unwrap_or(0));
                let base_toughness = i32::from(card.base_toughness().unwrap_or(0));

                let pt = if power != base_power || toughness != base_toughness {
                    format!(" {}/{} ({}/{})", power, toughness, base_power, base_toughness)
                } else {
                    format!(" {}/{}", power, toughness)
                };
                creatures.push(format!("    {}{}{}", name, pt, tapped));
            } else {
                other.push(format!("    {}{}", name, tapped));
            }
        }
    }

    // Output by category
    let has_permanents = !lands.is_empty() || !creatures.is_empty() || !other.is_empty();

    if !lands.is_empty() {
        writeln!(output, "  Lands:").unwrap();
        for land in &lands {
            writeln!(output, "{}", land).unwrap();
        }
    }

    if !creatures.is_empty() {
        writeln!(output, "  Creatures:").unwrap();
        for creature in &creatures {
            writeln!(output, "{}", creature).unwrap();
        }
    }

    if !other.is_empty() {
        writeln!(output, "  Other permanents:").unwrap();
        for perm in &other {
            writeln!(output, "{}", perm).unwrap();
        }
    }

    if !has_permanents {
        writeln!(output, "  (empty battlefield)").unwrap();
    }
}

/// Format choices with numeric prefixes for action list display
///
/// This is a shared function used by both native and WASM fancy TUI
/// to ensure consistent action list formatting with `[idx] text` prefixes.
pub fn format_choices_with_numbers(choices: &[String], highlighted_idx: usize) -> Vec<(String, bool)> {
    choices
        .iter()
        .enumerate()
        .map(|(idx, text)| {
            let numbered_text = format!("[{}] {}", idx, text);
            (numbered_text, idx == highlighted_idx)
        })
        .collect()
}
