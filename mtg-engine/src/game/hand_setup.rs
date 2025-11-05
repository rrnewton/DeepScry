//! Initial hand setup and library shuffling
//!
//! Handles the process of shuffling libraries and drawing opening hands,
//! with support for controlled initial hands for testing scenarios.

use crate::core::PlayerId;
use crate::game::GameState;
use crate::{MtgError, Result};

/// Configuration for a player's initial hand
#[derive(Debug, Clone)]
pub struct HandSetup {
    /// Specific cards to place in hand (by card name)
    pub specific_cards: Vec<String>,
}

impl HandSetup {
    /// Parse hand setup from semicolon-separated card names
    pub fn parse(input: &str) -> Result<Self> {
        let cards: Vec<String> = input
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if cards.is_empty() {
            return Err(MtgError::InvalidAction(
                "Hand setup must contain at least one card".to_string(),
            ));
        }

        if cards.len() > 7 {
            return Err(MtgError::InvalidAction(format!(
                "Hand setup cannot contain more than 7 cards (got {})",
                cards.len()
            )));
        }

        Ok(HandSetup { specific_cards: cards })
    }
}

/// Setup initial hands for all players with optional controlled card selection
pub fn setup_opening_hands(
    game: &mut GameState,
    player_ids: &[PlayerId],
    p1_setup: Option<&HandSetup>,
    p2_setup: Option<&HandSetup>,
) -> Result<()> {
    // Shuffle all libraries first (MTG Rules 103.2)
    for &player_id in player_ids {
        game.shuffle_library(player_id);
    }

    // Draw opening hands (MTG Rules 103.4)
    for (idx, &player_id) in player_ids.iter().enumerate() {
        let setup = match idx {
            0 => p1_setup,
            1 => p2_setup,
            _ => None,
        };

        if let Some(hand_setup) = setup {
            // Controlled hand setup: place specific cards, then draw rest randomly
            setup_controlled_hand(game, player_id, hand_setup)?;
        } else {
            // Normal random draw
            for _ in 0..7 {
                game.draw_card(player_id)?;
            }
        }
    }

    Ok(())
}

/// Setup a controlled hand with specific cards from the library
fn setup_controlled_hand(game: &mut GameState, player_id: PlayerId, hand_setup: &HandSetup) -> Result<()> {
    // Find and move specified cards from library to hand
    for card_name in &hand_setup.specific_cards {
        // Search library for a card with this name (need to search each time to avoid borrow checker issues)
        let card_id = {
            let zones = game
                .get_player_zones(player_id)
                .ok_or_else(|| MtgError::InvalidAction(format!("Player {:?} not found", player_id)))?;

            let matching_card = zones.library.cards.iter().find(|&&cid| {
                game.cards
                    .get(cid)
                    .map(|card| card.name.as_str() == card_name.as_str())
                    .unwrap_or(false)
            });

            match matching_card {
                Some(&id) => id,
                None => {
                    return Err(MtgError::InvalidAction(format!(
                        "Card '{}' not found in player {:?}'s library",
                        card_name, player_id
                    )));
                }
            }
        };

        // Remove from library and add to hand
        let zones = game
            .get_player_zones_mut(player_id)
            .ok_or_else(|| MtgError::InvalidAction(format!("Player {:?} not found", player_id)))?;
        zones.library.remove(card_id);
        zones.hand.add(card_id);
    }

    // Draw remaining cards randomly to reach 7 total
    let cards_in_hand = hand_setup.specific_cards.len();
    let remaining_to_draw = 7usize.saturating_sub(cards_in_hand);

    for _ in 0..remaining_to_draw {
        game.draw_card(player_id)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hand_setup() {
        // Valid single card
        let setup = HandSetup::parse("Lightning Bolt").unwrap();
        assert_eq!(setup.specific_cards.len(), 1);
        assert_eq!(setup.specific_cards[0], "Lightning Bolt");

        // Valid multiple cards
        let setup = HandSetup::parse("Mountain;Lightning Bolt;Island").unwrap();
        assert_eq!(setup.specific_cards.len(), 3);
        assert_eq!(setup.specific_cards[0], "Mountain");
        assert_eq!(setup.specific_cards[1], "Lightning Bolt");
        assert_eq!(setup.specific_cards[2], "Island");

        // Whitespace handling
        let setup = HandSetup::parse(" Mountain ; Lightning Bolt ").unwrap();
        assert_eq!(setup.specific_cards.len(), 2);
        assert_eq!(setup.specific_cards[0], "Mountain");
        assert_eq!(setup.specific_cards[1], "Lightning Bolt");

        // Maximum 7 cards
        let setup = HandSetup::parse("A;B;C;D;E;F;G").unwrap();
        assert_eq!(setup.specific_cards.len(), 7);
    }

    #[test]
    fn test_parse_hand_setup_errors() {
        // Empty string
        assert!(HandSetup::parse("").is_err());

        // Too many cards
        assert!(HandSetup::parse("A;B;C;D;E;F;G;H").is_err());
    }
}
