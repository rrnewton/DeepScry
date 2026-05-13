//! Incremental mana source tracking cache
//!
//! This module provides an event-driven cache that tracks mana-producing permanents
//! on the battlefield for a specific player. The cache is maintained incrementally
//! by game events (card enters/leaves battlefield, tap/untap) to avoid expensive
//! O(n) battlefield scans on every mana query.
//!
//! ## Design Principles
//!
//! - **Event-driven**: Events update the cache immediately, queries just read it
//! - **Always current**: The cache is eagerly maintained, never stale
//! - **Encapsulated updates**: All mutations go through well-defined methods
//! - **SmallVec optimization**: Most decks have <20 lands, so we avoid heap allocation
//! - **Lazy rebuild**: On undo/clone, mark dirty and rebuild on next query

use crate::core::{Card, CardId, ManaProductionKind, PlayerId};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Cached mana source tracking for a single player
///
/// Maintains lists of mana-producing permanents categorized by production type,
/// along with pre-computed untapped counts for simple sources.
///
/// This is stored per-player in GameState and updated by event handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManaSourceCache {
    /// Player who owns these mana sources
    player_id: PlayerId,

    // Simple sources by color (single fixed color)
    // SmallVec avoids heap allocation for typical deck sizes (~10-20 lands)
    white_sources: SmallVec<[CardId; 8]>,
    blue_sources: SmallVec<[CardId; 8]>,
    black_sources: SmallVec<[CardId; 8]>,
    red_sources: SmallVec<[CardId; 8]>,
    green_sources: SmallVec<[CardId; 8]>,
    colorless_sources: SmallVec<[CardId; 4]>, // Wastes, Sol Ring, etc.

    // Complex sources (dual lands, any-color, creatures with mana abilities)
    complex_sources: SmallVec<[CardId; 8]>,

    // Precomputed untapped counts (eagerly maintained by tap/untap events)
    untapped_white: u32,
    untapped_blue: u32,
    untapped_black: u32,
    untapped_red: u32,
    untapped_green: u32,
    untapped_colorless: u32,

    /// Dirty flag for exceptional cases (undo, clone)
    /// When true, next query must do full battlefield rebuild
    needs_rebuild: bool,
}

impl ManaSourceCache {
    /// Create a new empty cache for a player
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            white_sources: SmallVec::new(),
            blue_sources: SmallVec::new(),
            black_sources: SmallVec::new(),
            red_sources: SmallVec::new(),
            green_sources: SmallVec::new(),
            colorless_sources: SmallVec::new(),
            complex_sources: SmallVec::new(),
            untapped_white: 0,
            untapped_blue: 0,
            untapped_black: 0,
            untapped_red: 0,
            untapped_green: 0,
            untapped_colorless: 0,
            needs_rebuild: false,
        }
    }

    /// Check if cache needs full rebuild
    pub fn needs_rebuild(&self) -> bool {
        self.needs_rebuild
    }

    /// Mark cache as needing rebuild (called after undo/rewind)
    pub fn mark_dirty(&mut self) {
        self.needs_rebuild = true;
    }

    /// Check if cache is empty (no mana sources tracked)
    ///
    /// This can happen when tests create cards and add them to battlefield
    /// without triggering the event system that populates the cache.
    pub fn is_empty(&self) -> bool {
        self.white_sources.is_empty()
            && self.blue_sources.is_empty()
            && self.black_sources.is_empty()
            && self.red_sources.is_empty()
            && self.green_sources.is_empty()
            && self.colorless_sources.is_empty()
            && self.complex_sources.is_empty()
    }

    /// Get reference to white mana sources
    pub fn white_sources(&self) -> &[CardId] {
        &self.white_sources
    }

    /// Get reference to blue mana sources
    pub fn blue_sources(&self) -> &[CardId] {
        &self.blue_sources
    }

    /// Get reference to black mana sources
    pub fn black_sources(&self) -> &[CardId] {
        &self.black_sources
    }

    /// Get reference to red mana sources
    pub fn red_sources(&self) -> &[CardId] {
        &self.red_sources
    }

    /// Get reference to green mana sources
    pub fn green_sources(&self) -> &[CardId] {
        &self.green_sources
    }

    /// Get reference to colorless mana sources
    pub fn colorless_sources(&self) -> &[CardId] {
        &self.colorless_sources
    }

    /// Get reference to complex mana sources
    pub fn complex_sources(&self) -> &[CardId] {
        &self.complex_sources
    }

    /// Get untapped count for white sources
    pub fn untapped_white(&self) -> u32 {
        self.untapped_white
    }

    /// Get untapped count for blue sources
    pub fn untapped_blue(&self) -> u32 {
        self.untapped_blue
    }

    /// Get untapped count for black sources
    pub fn untapped_black(&self) -> u32 {
        self.untapped_black
    }

    /// Get untapped count for red sources
    pub fn untapped_red(&self) -> u32 {
        self.untapped_red
    }

    /// Get untapped count for green sources
    pub fn untapped_green(&self) -> u32 {
        self.untapped_green
    }

    /// Get untapped count for colorless sources
    pub fn untapped_colorless(&self) -> u32 {
        self.untapped_colorless
    }

    /// Event handler: A card entered the battlefield
    ///
    /// Classifies the card and adds it to appropriate source list(s).
    /// Only processes if the card is owned by this player and produces mana.
    pub fn on_card_entered(&mut self, card_id: CardId, card: &Card) {
        // Quick filter: only track cards owned by this player
        if card.owner != self.player_id {
            return;
        }

        // Quick filter: only track mana sources (O(1) cache check)
        if !card.definition.cache.is_mana_source {
            return;
        }

        // Creatures with mana abilities are always complex sources
        // (due to summoning sickness and other creature-specific rules)
        if card.is_creature() {
            self.complex_sources.push(card_id);
            return;
        }

        // Cards with chosen_color (like Thriving lands) are complex sources
        // since they can produce multiple colors
        if card.chosen_color.is_some() {
            self.complex_sources.push(card_id);
            return;
        }

        // Classify card based on mana production type. The `amount` field on the
        // production captures multi-mana sources (Sol Ring → 2 colorless, etc.)
        // and is added to the per-colour untapped totals so that bounds checks
        // and capacity queries see the full mana available.
        let production = &card.definition.cache.mana_production;
        let n = u32::from(production.amount.max(1));

        match &production.kind {
            ManaProductionKind::Fixed(color) => {
                // Simple source - add to appropriate color list
                use crate::core::ManaColor;
                match color {
                    ManaColor::White => {
                        self.white_sources.push(card_id);
                        if !card.tapped {
                            self.untapped_white += n;
                        }
                    }
                    ManaColor::Blue => {
                        self.blue_sources.push(card_id);
                        if !card.tapped {
                            self.untapped_blue += n;
                        }
                    }
                    ManaColor::Black => {
                        self.black_sources.push(card_id);
                        if !card.tapped {
                            self.untapped_black += n;
                        }
                    }
                    ManaColor::Red => {
                        self.red_sources.push(card_id);
                        if !card.tapped {
                            self.untapped_red += n;
                        }
                    }
                    ManaColor::Green => {
                        self.green_sources.push(card_id);
                        if !card.tapped {
                            self.untapped_green += n;
                        }
                    }
                }
            }
            ManaProductionKind::Colorless => {
                // Colorless source (Wastes, Sol Ring, etc.). Sol Ring's amount=2
                // means each untapped Sol Ring contributes 2 to the colorless capacity.
                self.colorless_sources.push(card_id);
                if !card.tapped {
                    self.untapped_colorless += n;
                }
            }
            ManaProductionKind::Choice(_) | ManaProductionKind::AnyColor => {
                // Complex source - dual land or any-color
                // Don't track untapped counts for complex sources (need full evaluation)
                self.complex_sources.push(card_id);
            }
        }
    }

    /// Event handler: A card left the battlefield
    ///
    /// Removes the card from all source lists and updates untapped counts.
    pub fn on_card_left(&mut self, card_id: CardId, card: &Card) {
        // Quick filter: only relevant if owned by this player
        if card.owner != self.player_id {
            return;
        }

        // Quick filter: only mana sources
        if !card.definition.cache.is_mana_source {
            return;
        }

        // Remove from all lists (card should only be in one, but safe to check all).
        // Multi-mana sources (Sol Ring → 2, Black Lotus → 3) deduct their full
        // per-activation `amount` from the corresponding untapped total.
        let was_untapped = !card.tapped;
        let n = u32::from(card.definition.cache.mana_production.amount.max(1));

        if let Some(pos) = self.white_sources.iter().position(|&id| id == card_id) {
            self.white_sources.swap_remove(pos);
            if was_untapped {
                self.untapped_white = self.untapped_white.saturating_sub(n);
            }
        }
        if let Some(pos) = self.blue_sources.iter().position(|&id| id == card_id) {
            self.blue_sources.swap_remove(pos);
            if was_untapped {
                self.untapped_blue = self.untapped_blue.saturating_sub(n);
            }
        }
        if let Some(pos) = self.black_sources.iter().position(|&id| id == card_id) {
            self.black_sources.swap_remove(pos);
            if was_untapped {
                self.untapped_black = self.untapped_black.saturating_sub(n);
            }
        }
        if let Some(pos) = self.red_sources.iter().position(|&id| id == card_id) {
            self.red_sources.swap_remove(pos);
            if was_untapped {
                self.untapped_red = self.untapped_red.saturating_sub(n);
            }
        }
        if let Some(pos) = self.green_sources.iter().position(|&id| id == card_id) {
            self.green_sources.swap_remove(pos);
            if was_untapped {
                self.untapped_green = self.untapped_green.saturating_sub(n);
            }
        }
        if let Some(pos) = self.colorless_sources.iter().position(|&id| id == card_id) {
            self.colorless_sources.swap_remove(pos);
            if was_untapped {
                self.untapped_colorless = self.untapped_colorless.saturating_sub(n);
            }
        }
        if let Some(pos) = self.complex_sources.iter().position(|&id| id == card_id) {
            self.complex_sources.swap_remove(pos);
            // Complex sources don't track untapped counts
        }
    }

    /// Event handler: A permanent was tapped
    ///
    /// Updates untapped counts if this is a mana source.
    pub fn on_tap(&mut self, card_id: CardId, card: &Card) {
        // Quick filter: only relevant if owned by this player
        if card.owner != self.player_id {
            return;
        }

        // Quick filter: only mana sources
        if !card.definition.cache.is_mana_source {
            return;
        }

        // Decrement untapped count for the appropriate color. Multi-mana sources
        // (Sol Ring → 2, etc.) deduct their full `amount` so capacity stays in sync.
        // Only simple sources track untapped counts.
        let n = u32::from(card.definition.cache.mana_production.amount.max(1));
        if self.white_sources.contains(&card_id) {
            self.untapped_white = self.untapped_white.saturating_sub(n);
        } else if self.blue_sources.contains(&card_id) {
            self.untapped_blue = self.untapped_blue.saturating_sub(n);
        } else if self.black_sources.contains(&card_id) {
            self.untapped_black = self.untapped_black.saturating_sub(n);
        } else if self.red_sources.contains(&card_id) {
            self.untapped_red = self.untapped_red.saturating_sub(n);
        } else if self.green_sources.contains(&card_id) {
            self.untapped_green = self.untapped_green.saturating_sub(n);
        } else if self.colorless_sources.contains(&card_id) {
            self.untapped_colorless = self.untapped_colorless.saturating_sub(n);
        }
        // Complex sources don't track untapped counts
    }

    /// Event handler: A permanent was untapped
    ///
    /// Updates untapped counts if this is a mana source.
    pub fn on_untap(&mut self, card_id: CardId, card: &Card) {
        // Quick filter: only relevant if owned by this player
        if card.owner != self.player_id {
            return;
        }

        // Quick filter: only mana sources
        if !card.definition.cache.is_mana_source {
            return;
        }

        // Increment untapped count for the appropriate color. Multi-mana sources
        // restore their full `amount` to capacity (Sol Ring → +2, etc.).
        // Only simple sources track untapped counts.
        let n = u32::from(card.definition.cache.mana_production.amount.max(1));
        if self.white_sources.contains(&card_id) {
            self.untapped_white += n;
        } else if self.blue_sources.contains(&card_id) {
            self.untapped_blue += n;
        } else if self.black_sources.contains(&card_id) {
            self.untapped_black += n;
        } else if self.red_sources.contains(&card_id) {
            self.untapped_red += n;
        } else if self.green_sources.contains(&card_id) {
            self.untapped_green += n;
        } else if self.colorless_sources.contains(&card_id) {
            self.untapped_colorless += n;
        }
        // Complex sources don't track untapped counts
    }

    /// Rebuild cache from battlefield (called when needs_rebuild == true)
    ///
    /// Scans the battlefield and rebuilds all internal state.
    /// This is expensive (O(n)) but only happens after undo/rewind.
    pub fn rebuild_from_battlefield(&mut self, game: &crate::game::GameState) {
        // Clear all state
        self.white_sources.clear();
        self.blue_sources.clear();
        self.black_sources.clear();
        self.red_sources.clear();
        self.green_sources.clear();
        self.colorless_sources.clear();
        self.complex_sources.clear();
        self.untapped_white = 0;
        self.untapped_blue = 0;
        self.untapped_black = 0;
        self.untapped_red = 0;
        self.untapped_green = 0;
        self.untapped_colorless = 0;

        // Scan battlefield and rebuild
        for &card_id in &game.battlefield.cards {
            if let Some(card) = game.cards.try_get(card_id) {
                self.on_card_entered(card_id, card);
            }
        }

        self.needs_rebuild = false;
    }

    /// Clear the cache (called when game state is reset)
    pub fn clear(&mut self) {
        self.white_sources.clear();
        self.blue_sources.clear();
        self.black_sources.clear();
        self.red_sources.clear();
        self.green_sources.clear();
        self.colorless_sources.clear();
        self.complex_sources.clear();
        self.untapped_white = 0;
        self.untapped_blue = 0;
        self.untapped_black = 0;
        self.untapped_red = 0;
        self.untapped_green = 0;
        self.untapped_colorless = 0;
        self.needs_rebuild = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::PlayerId;

    #[test]
    fn test_cache_creation() {
        let player_id = PlayerId::new(1);
        let cache = ManaSourceCache::new(player_id);

        assert_eq!(cache.untapped_white(), 0);
        assert_eq!(cache.white_sources().len(), 0);
        assert!(!cache.needs_rebuild());
    }

    #[test]
    fn test_mark_dirty() {
        let player_id = PlayerId::new(1);
        let mut cache = ManaSourceCache::new(player_id);

        assert!(!cache.needs_rebuild());
        cache.mark_dirty();
        assert!(cache.needs_rebuild());
    }

    #[test]
    fn test_simple_source_tracking() {
        let player_id = PlayerId::new(1);
        let mut cache = ManaSourceCache::new(player_id);

        // Create a white mana source (Plains)
        let card_id = CardId::new(1);
        let mut card = Card::new(card_id, "Plains", player_id);
        card.set_text("{T}: Add {W}.".to_string());

        // Add card
        cache.on_card_entered(card_id, &card);

        assert_eq!(cache.white_sources().len(), 1);
        assert_eq!(cache.untapped_white(), 1);

        // Tap card
        card.tapped = true;
        cache.on_tap(card_id, &card);
        assert_eq!(cache.untapped_white(), 0);

        // Untap card
        card.tapped = false;
        cache.on_untap(card_id, &card);
        assert_eq!(cache.untapped_white(), 1);

        // Remove card
        cache.on_card_left(card_id, &card);
        assert_eq!(cache.white_sources().len(), 0);
        assert_eq!(cache.untapped_white(), 0);
    }
}
