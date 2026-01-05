// TODO(mtg-0et0f): Remove this file-level allow once wildcards are fixed
#![allow(clippy::wildcard_enum_match_arm)]
//! Game zones (Library, Hand, Graveyard, Battlefield, etc.)
//!
//! ## Library Modes (for networking)
//!
//! Libraries support two modes to enable networked play with hidden information:
//!
//! - **Local**: Normal mode where all card contents are known (server-side)
//! - **Remote**: Client-side mode where library contents are hidden; cards are
//!   revealed via a buffer when drawn or searched
//!
//! In remote mode, the client doesn't know what cards are in the library (to
//! prevent cheating). Instead, when the server reveals a card (draw, tutor),
//! it sends the card info to the client which queues it in a pending reveals
//! buffer. When the client's local game state reaches the draw, it pulls from
//! this buffer.

use crate::core::{CardId, PlayerId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// ═══════════════════════════════════════════════════════════════════════════
// LIBRARY MODE (for networking)
// ═══════════════════════════════════════════════════════════════════════════

/// Library mode determines how a library zone handles its contents.
///
/// In local mode (server-side), all card contents are known and stored directly.
/// In remote mode (client-side), only the size is tracked and cards are revealed
/// via a pending buffer when drawn or searched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LibraryMode {
    /// Normal mode - all cards are known (server-side or local game)
    #[default]
    Local,
    /// Remote mode - contents hidden, cards revealed via buffer
    Remote {
        /// Number of cards in the library (public information per MTG rules)
        size: usize,
        /// Cards revealed by server, waiting to be "drawn" by local simulation
        pending_reveals: VecDeque<CardId>,
    },
}

impl LibraryMode {
    /// Create a new remote library with the given size
    pub fn new_remote(size: usize) -> Self {
        LibraryMode::Remote {
            size,
            pending_reveals: VecDeque::new(),
        }
    }

    /// Check if this is a remote library
    pub fn is_remote(&self) -> bool {
        matches!(self, LibraryMode::Remote { .. })
    }

    /// Get the size for remote libraries (returns None for local)
    pub fn remote_size(&self) -> Option<usize> {
        match self {
            LibraryMode::Remote { size, .. } => Some(*size),
            LibraryMode::Local => None,
        }
    }

    /// Queue a revealed card for later drawing (remote mode only)
    ///
    /// Returns false if this is a local library (no-op).
    pub fn queue_reveal(&mut self, card_id: CardId) -> bool {
        match self {
            LibraryMode::Remote { pending_reveals, .. } => {
                pending_reveals.push_back(card_id);
                true
            }
            LibraryMode::Local => false,
        }
    }

    /// Pop the next revealed card from the pending buffer (remote mode only)
    ///
    /// Returns None if local mode or if no reveals are pending.
    pub fn pop_reveal(&mut self) -> Option<CardId> {
        match self {
            LibraryMode::Remote { size, pending_reveals } => {
                let card = pending_reveals.pop_front();
                if card.is_some() {
                    *size = size.saturating_sub(1);
                }
                card
            }
            LibraryMode::Local => None,
        }
    }

    /// Decrement remote size (e.g., when a card is milled without reveal)
    pub fn decrement_size(&mut self) {
        if let LibraryMode::Remote { size, .. } = self {
            *size = size.saturating_sub(1);
        }
    }

    /// Increment remote size (e.g., when a card is put on top of library)
    pub fn increment_size(&mut self) {
        if let LibraryMode::Remote { size, .. } = self {
            *size += 1;
        }
    }
}

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

/// A zone containing cards (ordered for Library/Graveyard, unordered for others)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardZone {
    /// Zone type
    pub zone_type: Zone,

    /// Owner of this zone (each player has their own zones)
    pub owner: PlayerId,

    /// Cards in this zone (order matters for Library and Graveyard)
    ///
    /// For remote libraries, this vec is empty - use library_mode instead.
    pub cards: Vec<CardId>,

    /// Library mode (only used for Library zones in networked games)
    ///
    /// When Some(Remote), the library contents are hidden and cards
    /// are revealed via the pending_reveals buffer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_mode: Option<LibraryMode>,
}

impl CardZone {
    pub fn new(zone_type: Zone, owner: PlayerId) -> Self {
        CardZone {
            zone_type,
            owner,
            cards: Vec::new(),
            library_mode: None,
        }
    }

    /// Create a new remote library zone (for networked clients)
    ///
    /// The library starts with the given size but no known contents.
    /// Cards will be revealed via queue_reveal() as they are drawn/searched.
    pub fn new_remote_library(owner: PlayerId, size: usize) -> Self {
        CardZone {
            zone_type: Zone::Library,
            owner,
            cards: Vec::new(),
            library_mode: Some(LibraryMode::new_remote(size)),
        }
    }

    /// Check if this is a remote library
    pub fn is_remote_library(&self) -> bool {
        self.library_mode.as_ref().is_some_and(|m| m.is_remote())
    }

    /// Queue a revealed card for drawing (remote library only)
    ///
    /// Call this when the server reveals a card that will be drawn.
    /// Returns false if this is not a remote library.
    pub fn queue_reveal(&mut self, card_id: CardId) -> bool {
        if let Some(ref mut mode) = self.library_mode {
            mode.queue_reveal(card_id)
        } else {
            false
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
    ///
    /// For remote libraries, returns the tracked size (not cards.len()).
    pub fn len(&self) -> usize {
        if let Some(LibraryMode::Remote { size, .. }) = &self.library_mode {
            *size
        } else {
            self.cards.len()
        }
    }

    /// Check if this zone is empty
    ///
    /// For remote libraries, checks the tracked size.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Draw from top (for Library)
    ///
    /// For remote libraries, pops from the pending reveals buffer.
    /// Returns None if empty or if no reveal is pending (remote mode).
    pub fn draw_top(&mut self) -> Option<CardId> {
        if let Some(ref mut mode) = self.library_mode {
            mode.pop_reveal()
        } else {
            self.cards.pop()
        }
    }

    /// Look at top card without removing it
    ///
    /// For remote libraries, peeks at the first pending reveal (if any).
    /// Note: In remote mode, this only sees revealed cards, not the actual top.
    pub fn peek_top(&self) -> Option<CardId> {
        if let Some(LibraryMode::Remote { pending_reveals, .. }) = &self.library_mode {
            pending_reveals.front().copied()
        } else {
            self.cards.last().copied()
        }
    }

    /// Add to bottom (for Library)
    ///
    /// For remote libraries, only increments the size counter.
    /// The actual card placement is handled by the server.
    pub fn add_to_bottom(&mut self, card_id: CardId) {
        if let Some(ref mut mode) = self.library_mode {
            mode.increment_size();
            // Don't add to cards - remote library contents are hidden
        } else {
            self.cards.insert(0, card_id);
        }
    }

    /// Add to top (for Library - e.g., for effects that put cards on top)
    ///
    /// For remote libraries, only increments the size counter.
    pub fn add_to_top(&mut self, card_id: CardId) {
        if let Some(ref mut mode) = self.library_mode {
            mode.increment_size();
            // Don't add to cards - remote library contents are hidden
        } else {
            self.cards.push(card_id);
        }
    }

    /// Shuffle the zone (for Library)
    ///
    /// For remote libraries, this is a no-op - the server handles shuffling.
    pub fn shuffle(&mut self, rng: &mut impl rand::Rng) {
        if self.library_mode.is_none() {
            use rand::seq::SliceRandom;
            self.cards.shuffle(rng);
        }
        // Remote libraries: no-op, server handles shuffling
    }

    /// Clear all cards
    ///
    /// For remote libraries, also resets the size to 0.
    pub fn clear(&mut self) {
        self.cards.clear();
        if let Some(LibraryMode::Remote { size, pending_reveals }) = &mut self.library_mode {
            *size = 0;
            pending_reveals.clear();
        }
    }

    /// Get the number of pending reveals (remote library only)
    ///
    /// Returns 0 for local libraries.
    pub fn pending_reveals_count(&self) -> usize {
        if let Some(LibraryMode::Remote { pending_reveals, .. }) = &self.library_mode {
            pending_reveals.len()
        } else {
            0
        }
    }

    /// Decrement the remote library size by 1 (for cards already drawn server-side)
    ///
    /// Used for opening hand reveals where the server has already drawn the cards
    /// and we just need to update our tracked size. No-op for local libraries.
    pub fn decrement_size(&mut self) {
        if let Some(ref mut mode) = self.library_mode {
            mode.decrement_size();
        }
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

    // ═══════════════════════════════════════════════════════════════════════
    // REMOTE LIBRARY TESTS
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_remote_library_creation() {
        let player_id = PlayerId::new(1);
        let library = CardZone::new_remote_library(player_id, 60);

        assert!(library.is_remote_library());
        assert_eq!(library.len(), 60);
        assert!(!library.is_empty());
        assert_eq!(library.zone_type, Zone::Library);
        assert!(library.cards.is_empty()); // Cards vec is empty for remote
    }

    #[test]
    fn test_remote_library_draw_with_reveals() {
        let player_id = PlayerId::new(1);
        let mut library = CardZone::new_remote_library(player_id, 60);

        // Initially no reveals pending
        assert_eq!(library.pending_reveals_count(), 0);
        assert_eq!(library.draw_top(), None); // No reveal available

        // Queue some reveals (simulating server sending card info)
        let card1 = CardId::new(100);
        let card2 = CardId::new(101);
        let card3 = CardId::new(102);

        assert!(library.queue_reveal(card1));
        assert!(library.queue_reveal(card2));
        assert!(library.queue_reveal(card3));

        assert_eq!(library.pending_reveals_count(), 3);
        assert_eq!(library.len(), 60); // Size unchanged until draw

        // Draw the revealed cards (FIFO order)
        assert_eq!(library.draw_top(), Some(card1));
        assert_eq!(library.len(), 59);

        assert_eq!(library.draw_top(), Some(card2));
        assert_eq!(library.len(), 58);

        assert_eq!(library.draw_top(), Some(card3));
        assert_eq!(library.len(), 57);

        // No more reveals
        assert_eq!(library.draw_top(), None);
        assert_eq!(library.pending_reveals_count(), 0);
    }

    #[test]
    fn test_remote_library_peek() {
        let player_id = PlayerId::new(1);
        let mut library = CardZone::new_remote_library(player_id, 60);

        // No reveals - peek returns None
        assert_eq!(library.peek_top(), None);

        // Queue a reveal
        let card1 = CardId::new(100);
        library.queue_reveal(card1);

        // Peek returns the first pending reveal
        assert_eq!(library.peek_top(), Some(card1));
        assert_eq!(library.len(), 60); // Peek doesn't change size

        // Peek again - still same card
        assert_eq!(library.peek_top(), Some(card1));
    }

    #[test]
    fn test_remote_library_add_to_top_and_bottom() {
        let player_id = PlayerId::new(1);
        let mut library = CardZone::new_remote_library(player_id, 60);

        let card = CardId::new(100);

        // Add to top - just increments size
        library.add_to_top(card);
        assert_eq!(library.len(), 61);
        assert!(library.cards.is_empty()); // Still no cards in vec

        // Add to bottom - just increments size
        library.add_to_bottom(card);
        assert_eq!(library.len(), 62);
        assert!(library.cards.is_empty()); // Still no cards in vec
    }

    #[test]
    fn test_remote_library_clear() {
        let player_id = PlayerId::new(1);
        let mut library = CardZone::new_remote_library(player_id, 60);

        let card1 = CardId::new(100);
        library.queue_reveal(card1);

        assert_eq!(library.len(), 60);
        assert_eq!(library.pending_reveals_count(), 1);

        library.clear();

        assert_eq!(library.len(), 0);
        assert!(library.is_empty());
        assert_eq!(library.pending_reveals_count(), 0);
    }

    #[test]
    fn test_local_library_queue_reveal_is_noop() {
        let player_id = PlayerId::new(1);
        let mut library = CardZone::new(Zone::Library, player_id);

        let card = CardId::new(100);
        library.add(card);

        // Queue reveal on local library returns false (no-op)
        assert!(!library.queue_reveal(CardId::new(200)));
        assert!(!library.is_remote_library());
    }

    #[test]
    fn test_library_mode_serialization() {
        // Test that LibraryMode serializes correctly
        let local = LibraryMode::Local;
        let json = serde_json::to_string(&local).unwrap();
        assert!(json.contains("local"));

        let remote = LibraryMode::new_remote(60);
        let json = serde_json::to_string(&remote).unwrap();
        assert!(json.contains("remote"));
        assert!(json.contains("60"));

        // Roundtrip
        let roundtrip: LibraryMode = serde_json::from_str(&json).unwrap();
        assert!(roundtrip.is_remote());
        assert_eq!(roundtrip.remote_size(), Some(60));
    }
}
