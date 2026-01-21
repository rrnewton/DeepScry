//! Game zones (Library, Hand, Graveyard, Battlefield, etc.)
//!
//! ## Late-Binding CardID Architecture (mtg-qtqcr)
//!
//! In networked play, CardIDs are shared publicly between server and clients,
//! but the CardID ⟺ CardName binding is deferred until reveal time.
//!
//! All zones use a unified model:
//! - `cards: Vec<CardId>` stores the cards in the zone
//! - Cards in the EntityStore may be "reserved" (slot exists but no Card yet)
//! - When revealed, the Card is inserted via `RevealCard` action
//!
//! This eliminates the old `LibraryMode::Remote` and `pending_reveals` complexity.

use crate::core::{CardId, PlayerId};
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════
// ZONES
// ═══════════════════════════════════════════════════════════════════════════

/// Different zones where cards can exist
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zone {
    Library,
    Hand,
    Battlefield,
    Graveyard,
    Exile,
    Stack,
    Command,
}

impl Zone {
    /// Parse a zone name from a string, handling common variants.
    ///
    /// Accepts capitalized, lowercase, and abbreviated forms:
    /// - "Graveyard", "graveyard", "Grave" → Graveyard
    /// - "Hand", "hand" → Hand
    /// - "Library", "library" → Library
    /// - "Battlefield", "battlefield", "Play" → Battlefield
    /// - "Exile", "exile", "Exiled" → Exile
    /// - "Stack", "stack" → Stack
    /// - "Command", "command" → Command
    ///
    /// Returns None for unrecognized strings.
    #[inline]
    pub fn from_str_lenient(s: &str) -> Option<Zone> {
        match s {
            "Graveyard" | "graveyard" | "Grave" => Some(Zone::Graveyard),
            "Hand" | "hand" => Some(Zone::Hand),
            "Library" | "library" => Some(Zone::Library),
            "Battlefield" | "battlefield" | "Play" => Some(Zone::Battlefield),
            "Exile" | "exile" | "Exiled" => Some(Zone::Exile),
            "Stack" | "stack" => Some(Zone::Stack),
            "Command" | "command" => Some(Zone::Command),
            _ => None,
        }
    }
}

/// A zone containing cards (ordered for Library/Graveyard, unordered for others)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardZone {
    /// Zone type
    pub zone_type: Zone,

    /// Owner of this zone (each player has their own zones)
    pub owner: PlayerId,

    /// Cards in this zone (order matters for Library and Graveyard)
    ///
    /// For network clients, CardIDs are known but card identities may not be.
    /// Use EntityStore::is_revealed() to check if a card's identity is known.
    pub cards: Vec<CardId>,
}

impl CardZone {
    pub fn new(zone_type: Zone, owner: PlayerId) -> Self {
        CardZone {
            zone_type,
            owner,
            cards: Vec::new(),
        }
    }

    /// Create a library zone with pre-allocated CardIDs (for networked clients)
    ///
    /// In the late-binding architecture, CardIDs are known upfront but card
    /// identities are revealed later via RevealCard actions.
    ///
    /// # Arguments
    /// * `owner` - The player who owns this library
    /// * `card_ids` - The CardIDs in the library (in order from bottom to top)
    pub fn new_library_with_cards(owner: PlayerId, card_ids: Vec<CardId>) -> Self {
        CardZone {
            zone_type: Zone::Library,
            owner,
            cards: card_ids,
        }
    }

    pub fn add(&mut self, card_id: CardId) {
        self.cards.push(card_id);
    }

    pub fn remove(&mut self, card_id: CardId) -> bool {
        if let Some(pos) = self.cards.iter().position(|&id| id == card_id) {
            // Note: We use remove() instead of swap_remove() even for semantically unordered zones
            // (Hand, Battlefield, etc.) because iteration order matters for deterministic gameplay.
            // Controllers iterate over cards in a consistent order, so changing iteration order
            // would break determinism tests.
            self.cards.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn contains(&self, card_id: CardId) -> bool {
        self.cards.contains(&card_id)
    }

    /// Get the number of cards in this zone
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Check if this zone is empty
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Draw from top (for Library)
    ///
    /// Returns None if the library is empty.
    pub fn draw_top(&mut self) -> Option<CardId> {
        self.cards.pop()
    }

    /// Look at top card without removing it
    pub fn peek_top(&self) -> Option<CardId> {
        self.cards.last().copied()
    }

    /// Add to bottom (for Library)
    pub fn add_to_bottom(&mut self, card_id: CardId) {
        self.cards.insert(0, card_id);
    }

    /// Add to top (for Library - e.g., for effects that put cards on top)
    pub fn add_to_top(&mut self, card_id: CardId) {
        self.cards.push(card_id);
    }

    /// Shuffle the zone (for Library)
    pub fn shuffle(&mut self, rng: &mut impl rand::Rng) {
        use rand::seq::SliceRandom;
        self.cards.shuffle(rng);
    }

    /// Clear all cards
    pub fn clear(&mut self) {
        self.cards.clear();
    }
}

/// Collection of all zones for a player
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerZones {
    pub library: CardZone,
    pub hand: CardZone,
    pub graveyard: CardZone,
    pub exile: CardZone,
}

impl PlayerZones {
    pub fn new(player_id: PlayerId) -> Self {
        PlayerZones {
            library: CardZone::new(Zone::Library, player_id),
            hand: CardZone::new(Zone::Hand, player_id),
            graveyard: CardZone::new(Zone::Graveyard, player_id),
            exile: CardZone::new(Zone::Exile, player_id),
        }
    }

    pub fn get_zone(&self, zone: Zone) -> Option<&CardZone> {
        match zone {
            Zone::Library => Some(&self.library),
            Zone::Hand => Some(&self.hand),
            Zone::Graveyard => Some(&self.graveyard),
            Zone::Exile => Some(&self.exile),
            // Battlefield, Stack, and Command are shared zones on GameState, not per-player
            Zone::Battlefield | Zone::Stack | Zone::Command => None,
        }
    }

    pub fn get_zone_mut(&mut self, zone: Zone) -> Option<&mut CardZone> {
        match zone {
            Zone::Library => Some(&mut self.library),
            Zone::Hand => Some(&mut self.hand),
            Zone::Graveyard => Some(&mut self.graveyard),
            Zone::Exile => Some(&mut self.exile),
            // Battlefield, Stack, and Command are shared zones on GameState, not per-player
            Zone::Battlefield | Zone::Stack | Zone::Command => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_card_zone() {
        let player_id = PlayerId::new(1);
        let mut zone = CardZone::new(Zone::Hand, player_id);

        assert_eq!(zone.len(), 0);
        assert!(zone.is_empty());

        let card1 = CardId::new(10);
        let card2 = CardId::new(11);

        zone.add(card1);
        zone.add(card2);

        assert_eq!(zone.len(), 2);
        assert!(zone.contains(card1));
        assert!(zone.contains(card2));

        assert!(zone.remove(card1));
        assert_eq!(zone.len(), 1);
        assert!(!zone.contains(card1));
    }

    #[test]
    fn test_library_operations() {
        let player_id = PlayerId::new(1);
        let mut library = CardZone::new(Zone::Library, player_id);

        let card1 = CardId::new(10);
        let card2 = CardId::new(11);
        let card3 = CardId::new(12);

        library.add(card1); // Bottom
        library.add(card2);
        library.add(card3); // Top

        assert_eq!(library.peek_top(), Some(card3));
        assert_eq!(library.draw_top(), Some(card3));
        assert_eq!(library.len(), 2);
        assert_eq!(library.draw_top(), Some(card2));
        assert_eq!(library.draw_top(), Some(card1));
        assert!(library.is_empty());
        assert_eq!(library.draw_top(), None);
    }

    #[test]
    fn test_player_zones() {
        let player_id = PlayerId::new(1);
        let zones = PlayerZones::new(player_id);

        assert_eq!(zones.library.zone_type, Zone::Library);
        assert_eq!(zones.hand.zone_type, Zone::Hand);
        assert_eq!(zones.graveyard.zone_type, Zone::Graveyard);
        assert_eq!(zones.exile.zone_type, Zone::Exile);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // LIBRARY WITH CARDS TESTS (for late-binding architecture)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_library_with_cards() {
        let player_id = PlayerId::new(1);
        let card_ids = vec![CardId::new(0), CardId::new(1), CardId::new(2)];
        let mut library = CardZone::new_library_with_cards(player_id, card_ids);

        assert_eq!(library.zone_type, Zone::Library);
        assert_eq!(library.len(), 3);
        assert!(!library.is_empty());

        // Draw from top (last element is top)
        assert_eq!(library.draw_top(), Some(CardId::new(2)));
        assert_eq!(library.len(), 2);

        assert_eq!(library.draw_top(), Some(CardId::new(1)));
        assert_eq!(library.draw_top(), Some(CardId::new(0)));
        assert!(library.is_empty());
    }

    #[test]
    fn test_library_shuffle() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha12Rng;

        let player_id = PlayerId::new(1);
        let mut library = CardZone::new(Zone::Library, player_id);

        for i in 0..10 {
            library.add(CardId::new(i));
        }

        let original: Vec<CardId> = library.cards.clone();
        let mut rng = ChaCha12Rng::seed_from_u64(12345);
        library.shuffle(&mut rng);

        // After shuffle with seeded RNG, order should change
        // (With 10 cards and a random seed, very unlikely to stay same)
        assert_ne!(library.cards, original);
        assert_eq!(library.len(), 10);
    }

    #[test]
    fn test_zone_from_str_lenient() {
        // Test capitalized forms
        assert_eq!(Zone::from_str_lenient("Graveyard"), Some(Zone::Graveyard));
        assert_eq!(Zone::from_str_lenient("Hand"), Some(Zone::Hand));
        assert_eq!(Zone::from_str_lenient("Library"), Some(Zone::Library));
        assert_eq!(Zone::from_str_lenient("Battlefield"), Some(Zone::Battlefield));
        assert_eq!(Zone::from_str_lenient("Exile"), Some(Zone::Exile));
        assert_eq!(Zone::from_str_lenient("Stack"), Some(Zone::Stack));
        assert_eq!(Zone::from_str_lenient("Command"), Some(Zone::Command));

        // Test lowercase forms
        assert_eq!(Zone::from_str_lenient("graveyard"), Some(Zone::Graveyard));
        assert_eq!(Zone::from_str_lenient("hand"), Some(Zone::Hand));

        // Test abbreviated forms
        assert_eq!(Zone::from_str_lenient("Grave"), Some(Zone::Graveyard));
        assert_eq!(Zone::from_str_lenient("Play"), Some(Zone::Battlefield));
        assert_eq!(Zone::from_str_lenient("Exiled"), Some(Zone::Exile));

        // Test invalid strings
        assert_eq!(Zone::from_str_lenient("invalid"), None);
        assert_eq!(Zone::from_str_lenient(""), None);
    }
}
