//! Game initialization from decks
//!
//! Creates games from deck lists and card database

use crate::core::PlayerId;
use crate::game::GameState;
use crate::loader::{AsyncCardDatabase as CardDatabase, DeckList};
use crate::{MtgError, Result};

/// Game builder for initializing games from decks
pub struct GameInitializer<'a> {
    card_db: &'a CardDatabase,
}

impl<'a> GameInitializer<'a> {
    /// Create a new game initializer with a card database
    pub fn new(card_db: &'a CardDatabase) -> Self {
        GameInitializer { card_db }
    }

    /// Initialize a two-player game with reserve-only CardIDs (for network clients)
    ///
    /// This creates the game structure without instantiating any cards. Instead, it:
    /// 1. Reserves CardID slots based on DeckCardIdRanges
    /// 2. Sets up libraries with CardIDs (but no Card instances yet)
    /// 3. Card identities will be revealed later via RevealCard actions
    ///
    /// This is used by network clients in the late-binding CardID architecture
    /// (mtg-qtqcr) where CardIDs are known upfront but card identities are hidden.
    pub fn init_game_reserve_only(
        &self,
        player1_name: String,
        player2_name: String,
        starting_life: i32,
        ranges: &crate::network::DeckCardIdRanges,
    ) -> GameState {
        use crate::core::CardId;

        let total_cards = ranges.total_cards() as usize;
        let mut game = GameState::new_two_player_with_capacity(player1_name, player2_name, starting_life, total_cards);

        // Reserve all CardID slots in EntityStore without instantiating cards
        // This uses the Phase 1 EntityStore::reserve_range() method
        game.cards
            .reserve_range(CardId::new(ranges.p1_start), ranges.p1_end - ranges.p1_start);
        game.cards
            .reserve_range(CardId::new(ranges.p2_start), ranges.p2_end - ranges.p2_start);

        // Create CardID vectors for each player's library
        // CardIDs are known, but card identities will be revealed later
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let p1_card_ids: Vec<CardId> = (ranges.p1_start..ranges.p1_end).map(CardId::new).collect();
        let p2_card_ids: Vec<CardId> = (ranges.p2_start..ranges.p2_end).map(CardId::new).collect();

        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.library = crate::zones::CardZone::new_library_with_cards(p1_id, p1_card_ids);
        }
        if let Some(zones) = game.get_player_zones_mut(p2_id) {
            zones.library = crate::zones::CardZone::new_library_with_cards(p2_id, p2_card_ids);
        }

        // Set next_entity_id past the reserved range
        // Cards will use IDs from 0..total_cards, so next_entity_id should start after
        game.set_next_entity_id(ranges.p2_end);

        log::debug!(
            "Reserve-only game initialized: {} total CardIDs reserved, libraries have CardIDs",
            total_cards
        );

        game
    }

    /// Initialize a two-player game from two decks
    pub async fn init_game(
        &self,
        player1_name: String,
        player1_deck: &DeckList,
        player2_name: String,
        player2_deck: &DeckList,
        starting_life: i32,
    ) -> Result<GameState> {
        // Calculate total cards for pre-sizing EntityStore
        let total_cards: usize = player1_deck.main_deck.iter().map(|e| e.count as usize).sum::<usize>()
            + player2_deck.main_deck.iter().map(|e| e.count as usize).sum::<usize>();

        let mut game = GameState::new_two_player_with_capacity(player1_name, player2_name, starting_life, total_cards);

        // Get player IDs
        let player1_id = game.players[0].id;
        let player2_id = game.players[1].id;

        // Pre-load all unique cards from both decks to ensure deterministic CardID allocation
        // This populates the card database cache before we start allocating CardIDs
        let mut unique_cards = std::collections::HashSet::new();
        for entry in &player1_deck.main_deck {
            unique_cards.insert(entry.card_name.clone());
        }
        for entry in &player2_deck.main_deck {
            unique_cards.insert(entry.card_name.clone());
        }

        // Load all cards in parallel (into cache)
        // Sort to ensure deterministic ordering across runs
        let mut card_names: Vec<String> = unique_cards.into_iter().collect();
        card_names.sort();
        if !card_names.is_empty() {
            self.card_db.load_cards(&card_names).await?;
        }

        // Scan all loaded cards for token script references
        // This ensures we preload any tokens that cards might create
        let mut token_scripts = std::collections::HashSet::new();
        for card_name in &card_names {
            if let Some(card_def) = self.card_db.get_card(card_name).await? {
                for token_script in card_def.extract_token_scripts() {
                    token_scripts.insert(token_script);
                }
            }
        }

        // Load all token definitions from tokenscripts/ directory
        if !token_scripts.is_empty() {
            for token_script in token_scripts {
                // Token scripts are in forge-java/forge-gui/res/tokenscripts/
                // Format: c_a_food_sac.txt
                if let Some(token_def) = self.card_db.get_token(&token_script).await? {
                    game.token_definitions
                        .insert(token_script, std::sync::Arc::new(token_def));
                }
            }
        }

        // Now load decks sequentially - cards will come from cache, ensuring deterministic order
        // Deck 1: card1, card2, card3, ...
        // Deck 2: card1, card2, card3, ...
        self.load_deck_into_game(&mut game, player1_id, player1_deck).await?;

        self.load_deck_into_game(&mut game, player2_id, player2_deck).await?;

        Ok(game)
    }

    /// Load a deck into a player's library
    async fn load_deck_into_game(&self, game: &mut GameState, player_id: PlayerId, deck: &DeckList) -> Result<()> {
        for entry in &deck.main_deck {
            // Look up the card definition
            let card_def = self.card_db.get_card(&entry.card_name).await?.ok_or_else(|| {
                MtgError::InvalidCardFormat(format!("Card not found in database: {}", entry.card_name))
            })?;

            // Create the requested number of copies
            for _ in 0..entry.count {
                let card_id = game.next_card_id();
                let card = card_def.instantiate(card_id, player_id);

                // Add to game's card store
                game.cards.insert(card_id, card);

                // Add to player's library
                if let Some(zones) = game.get_player_zones_mut(player_id) {
                    zones.library.add(card_id);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{DeckEntry, DeckLoader};
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_init_simple_game() {
        // Only run if cardsfolder exists
        let cardsfolder = PathBuf::from("cardsfolder");
        if !cardsfolder.exists() {
            return;
        }

        // Load card database
        let db = CardDatabase::new(cardsfolder);
        db.eager_load().await.unwrap();

        // Create simple decks (all Lightning Bolts and Mountains)
        let deck_content = r#"
[Main]
20 Mountain
40 Lightning Bolt
"#;

        let deck = DeckLoader::parse(deck_content).unwrap();

        // Initialize game
        let initializer = GameInitializer::new(&db);
        let game = initializer
            .init_game("Player1".to_string(), &deck, "Player2".to_string(), &deck, 20)
            .await
            .unwrap();

        // Verify game state
        assert_eq!(game.players.len(), 2);

        // Check each player has 60 cards in library
        for player in &game.players {
            if let Some(zones) = game.get_player_zones(player.id) {
                assert_eq!(zones.library.cards.len(), 60);
            }
        }

        // Total of 120 cards in the game (60 per player)
        assert_eq!(game.cards.len(), 120);
    }

    #[tokio::test]
    async fn test_missing_card_error() {
        use std::path::PathBuf;

        let db = CardDatabase::new(PathBuf::from("cardsfolder")); // Empty database (no eager load)
        let deck = DeckList {
            main_deck: vec![DeckEntry {
                card_name: "Nonexistent Card".to_string(),
                count: 1,
            }],
            sideboard: vec![],
        };

        let initializer = GameInitializer::new(&db);
        let result = initializer
            .init_game("Player1".to_string(), &deck, "Player2".to_string(), &deck, 20)
            .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_init_game_reserve_only() {
        use crate::core::CardId;
        use crate::network::DeckCardIdRanges;

        // Create ranges for two 40-card decks
        let ranges = DeckCardIdRanges::from_deck_sizes(40, 40);

        // Create an empty card database (not used for reserve-only mode)
        let db = CardDatabase::new(PathBuf::from("cardsfolder"));
        let initializer = GameInitializer::new(&db);

        // Initialize in reserve-only mode
        let game = initializer.init_game_reserve_only("Player1".to_string(), "Player2".to_string(), 20, &ranges);

        // Verify game state
        assert_eq!(game.players.len(), 2);

        // Both libraries should have CardIDs but no Card instances yet
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.len(), 40);
            // Library contains CardIDs
            assert_eq!(zones.library.cards.len(), 40);
        } else {
            panic!("P1 zones not found");
        }

        if let Some(zones) = game.get_player_zones(p2_id) {
            assert_eq!(zones.library.len(), 40);
            assert_eq!(zones.library.cards.len(), 40);
        } else {
            panic!("P2 zones not found");
        }

        // No cards should be instantiated (len() counts Some entries)
        assert_eq!(game.cards.len(), 0);

        // But all CardID slots should be reserved (can check by attempting insert)
        // Verify slots are reserved by checking is_revealed returns false
        assert!(!game.cards.is_revealed(CardId::new(0)));
        assert!(!game.cards.is_revealed(CardId::new(39)));
        assert!(!game.cards.is_revealed(CardId::new(40)));
        assert!(!game.cards.is_revealed(CardId::new(79)));
    }

    #[test]
    fn test_reserve_only_card_ranges() {
        use crate::core::CardId;
        use crate::network::DeckCardIdRanges;

        // Create asymmetric deck sizes
        let ranges = DeckCardIdRanges::from_deck_sizes(60, 40);

        let db = CardDatabase::new(PathBuf::from("cardsfolder"));
        let initializer = GameInitializer::new(&db);

        let game = initializer.init_game_reserve_only("P1".to_string(), "P2".to_string(), 20, &ranges);

        // Verify ranges
        assert_eq!(ranges.p1_start, 0);
        assert_eq!(ranges.p1_end, 60);
        assert_eq!(ranges.p2_start, 60);
        assert_eq!(ranges.p2_end, 100);

        // Check library sizes match
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.len(), 60);
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert_eq!(zones.library.len(), 40);
        }

        // Verify CardID slots are reserved (not revealed but reservable)
        // All slots from 0..99 should be unrevealed
        for id in 0..100 {
            assert!(!game.cards.is_revealed(CardId::new(id)));
        }
    }
}
