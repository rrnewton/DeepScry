//! Game zones (Library, Hand, Graveyard, Battlefield, etc.)

use crate::core::{CardId, PlayerId};
use serde::{Deserialize, Serialize};

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

/// A zone containing cards (ordered for Library/Graveyard, unordered for others)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardZone {
    /// Zone type
    pub zone_type: Zone,

    /// Owner of this zone (each player has their own zones)
    pub owner: PlayerId,

    /// Cards in this zone (order matters for Library and Graveyard)
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

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Draw from top (for Library)
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
            _ => None,
        }
    }

    pub fn get_zone_mut(&mut self, zone: Zone) -> Option<&mut CardZone> {
        match zone {
            Zone::Library => Some(&mut self.library),
            Zone::Hand => Some(&mut self.hand),
            Zone::Graveyard => Some(&mut self.graveyard),
            Zone::Exile => Some(&mut self.exile),
            _ => None,
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
}
