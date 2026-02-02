//! Incremental mana source index with dirty-bit invalidation
//!
//! This module provides an efficient, incrementally-updateable index of mana-producing
//! cards on the battlefield. Instead of scanning the entire battlefield on every
//! mana query (O(n) per query), this maintains a cached index that is updated
//! incrementally when cards enter/leave the battlefield or tap/untap.
//!
//! # Architecture
//!
//! The index organizes mana producers into **color buckets**:
//! - White producers
//! - Blue producers
//! - Black producers
//! - Red producers
//! - Green producers
//! - Colorless producers
//! - Multi-color producers (dual lands, any-color sources)
//!
//! Each bucket tracks:
//! - List of CardIds that produce that color
//! - A dirty bit indicating whether the bucket needs recalculation
//! - Cached mana capacity (sum of untapped sources)
//!
//! # Invalidation Strategy
//!
//! The index uses **lazy invalidation** with dirty bits:
//!
//! 1. **On card enter battlefield**: If mana producer, add to appropriate bucket(s), set dirty
//! 2. **On card leave battlefield**: If mana producer, remove from bucket(s), set dirty
//! 3. **On tap/untap**: If mana producer, set bucket dirty (capacity changed)
//! 4. **On query**: If any bucket dirty, recalculate capacity for that bucket only
//!
//! This is much more efficient than full rescans when:
//! - Most priority passes don't change battlefield state
//! - Tap events only affect one bucket at a time
//! - Undo operations can simply mark dirty and let next query rebuild
//!
//! # Undo Integration
//!
//! For undo support, we use the simpler "dirty-on-undo" strategy:
//! - Undoing a MoveCard or TapCard marks the entire index as dirty
//! - The next query will rebuild the affected parts
//!
//! This is simpler than maintaining reverse-delta operations and works well
//! because undo typically happens in batches during tree search.

use crate::core::{CardId, ManaColor, ManaProductionKind, PlayerId};
use crate::game::mana_colors::ManaColors;
use crate::game::GameState;
use smallvec::SmallVec;

/// Color bucket categories for mana producers
///
/// This enum is designed for O(1) array indexing without HashMap overhead.
/// The discriminant values are used directly as array indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ManaColorBucket {
    White = 0,
    Blue = 1,
    Black = 2,
    Red = 3,
    Green = 4,
    Colorless = 5,
    /// Multi-color sources (dual lands, any-color like City of Brass)
    Multi = 6,
}

impl ManaColorBucket {
    /// Number of bucket categories
    pub const COUNT: usize = 7;

    /// Convert from ManaColor to bucket
    #[inline]
    pub fn from_color(color: ManaColor) -> Self {
        match color {
            ManaColor::White => ManaColorBucket::White,
            ManaColor::Blue => ManaColorBucket::Blue,
            ManaColor::Black => ManaColorBucket::Black,
            ManaColor::Red => ManaColorBucket::Red,
            ManaColor::Green => ManaColorBucket::Green,
        }
    }

    /// Get the array index for this bucket
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A single bucket of mana producers of a specific color category
#[derive(Debug, Clone, Default)]
pub struct ManaProducerBucket {
    /// CardIds of mana producers in this bucket
    /// SmallVec<[CardId; 8]> handles typical land counts without heap allocation
    pub cards: SmallVec<[CardId; 8]>,

    /// Whether this bucket's cached capacity is stale
    pub dirty: bool,

    /// Cached count of untapped sources in this bucket
    /// Only valid when dirty == false
    pub untapped_count: u8,
}

impl ManaProducerBucket {
    /// Create a new empty bucket
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a card to this bucket, marking dirty
    #[inline]
    pub fn add(&mut self, card_id: CardId) {
        self.cards.push(card_id);
        self.dirty = true;
    }

    /// Remove a card from this bucket, marking dirty
    ///
    /// Returns true if the card was found and removed
    #[inline]
    pub fn remove(&mut self, card_id: CardId) -> bool {
        if let Some(pos) = self.cards.iter().position(|&id| id == card_id) {
            self.cards.swap_remove(pos);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Check if this bucket contains a card
    #[inline]
    pub fn contains(&self, card_id: CardId) -> bool {
        self.cards.contains(&card_id)
    }

    /// Mark this bucket as dirty (needs recalculation)
    #[inline]
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clear the bucket entirely
    pub fn clear(&mut self) {
        self.cards.clear();
        self.dirty = true;
        self.untapped_count = 0;
    }
}

/// Incremental index of mana-producing cards organized by color bucket
///
/// This provides O(1) lookup of whether a card is a mana producer and which
/// bucket(s) it belongs to, plus O(bucket_size) recalculation when dirty.
#[derive(Debug, Clone)]
pub struct ManaProducerIndex {
    /// Array of color buckets, indexed by ManaColorBucket discriminant
    /// Using array instead of HashMap for O(1) access without hashing
    buckets: [ManaProducerBucket; ManaColorBucket::COUNT],

    /// Global dirty flag - if true, the entire index needs rebuild
    /// This is set on undo operations for simplicity
    globally_dirty: bool,

    /// The player this index is for
    player_id: PlayerId,

    /// Maps CardId -> which bucket(s) it belongs to
    /// SmallVec because most cards belong to 1 bucket (basic lands),
    /// dual lands belong to 2, City of Brass to all 5 colors
    card_buckets: Vec<(CardId, SmallVec<[ManaColorBucket; 2]>)>,
}

impl ManaProducerIndex {
    /// Create a new empty index for a player
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            buckets: Default::default(),
            globally_dirty: true, // Start dirty so first query builds index
            player_id,
            card_buckets: Vec::new(),
        }
    }

    /// Mark the entire index as dirty (used for undo)
    #[inline]
    pub fn mark_globally_dirty(&mut self) {
        self.globally_dirty = true;
    }

    /// Check if the index needs a full rebuild
    #[inline]
    pub fn is_globally_dirty(&self) -> bool {
        self.globally_dirty
    }

    /// Get a bucket by color category
    #[inline]
    pub fn bucket(&self, bucket: ManaColorBucket) -> &ManaProducerBucket {
        &self.buckets[bucket.index()]
    }

    /// Get a mutable bucket by color category
    #[inline]
    pub fn bucket_mut(&mut self, bucket: ManaColorBucket) -> &mut ManaProducerBucket {
        &mut self.buckets[bucket.index()]
    }

    /// Notify the index that a card entered the battlefield
    ///
    /// If the card is a mana producer, it will be added to the appropriate bucket(s).
    /// Returns true if the card was added as a mana producer.
    pub fn on_card_entered(&mut self, card_id: CardId, game: &GameState) -> bool {
        // Check if this card is a mana producer for our player
        let Some(card) = game.cards.try_get(card_id) else {
            return false;
        };

        if card.owner != self.player_id {
            return false;
        }

        // Determine what mana this card produces
        let buckets = self.classify_mana_producer(card);
        if buckets.is_empty() {
            return false;
        }

        // Add to each relevant bucket
        for &bucket in &buckets {
            self.buckets[bucket.index()].add(card_id);
        }

        // Track which buckets this card is in
        self.card_buckets.push((card_id, buckets));

        true
    }

    /// Notify the index that a card left the battlefield
    ///
    /// If the card was a mana producer, it will be removed from all buckets.
    /// Returns true if the card was removed as a mana producer.
    pub fn on_card_left(&mut self, card_id: CardId) -> bool {
        // Find and remove from card_buckets tracking
        let Some(pos) = self.card_buckets.iter().position(|(id, _)| *id == card_id) else {
            return false;
        };

        let (_, buckets) = self.card_buckets.swap_remove(pos);

        // Remove from each bucket
        for bucket in buckets {
            self.buckets[bucket.index()].remove(card_id);
        }

        true
    }

    /// Notify the index that a card's tap state changed
    ///
    /// This marks the relevant bucket(s) as dirty so capacity will be recalculated.
    pub fn on_tap_changed(&mut self, card_id: CardId) {
        // Find which buckets this card is in
        if let Some((_, buckets)) = self.card_buckets.iter().find(|(id, _)| *id == card_id) {
            for &bucket in buckets {
                self.buckets[bucket.index()].mark_dirty();
            }
        }
    }

    /// Classify a card into mana producer bucket(s)
    ///
    /// Returns empty SmallVec if the card is not a mana producer.
    fn classify_mana_producer(&self, card: &crate::core::Card) -> SmallVec<[ManaColorBucket; 2]> {
        use crate::core::CardType;

        let mut buckets = SmallVec::new();

        // Check if it's a land or has mana ability
        let is_land = card.types.contains(&CardType::Land);
        let has_mana_ability = card.types.contains(&CardType::Creature) && card.definition.cache.mana_production.produces_mana();

        if !is_land && !has_mana_ability {
            return buckets;
        }

        // Check for basic lands first (most common case)
        if is_land {
            match card.name.as_str() {
                "Plains" => {
                    buckets.push(ManaColorBucket::White);
                    return buckets;
                }
                "Island" => {
                    buckets.push(ManaColorBucket::Blue);
                    return buckets;
                }
                "Swamp" => {
                    buckets.push(ManaColorBucket::Black);
                    return buckets;
                }
                "Mountain" => {
                    buckets.push(ManaColorBucket::Red);
                    return buckets;
                }
                "Forest" => {
                    buckets.push(ManaColorBucket::Green);
                    return buckets;
                }
                "Wastes" => {
                    buckets.push(ManaColorBucket::Colorless);
                    return buckets;
                }
                _ => {}
            }
        }

        // Check cached mana production (covers creatures and complex lands)
        if card.definition.cache.mana_production.produces_mana() {
            match &card.definition.cache.mana_production.kind {
                ManaProductionKind::Fixed(color) => {
                    buckets.push(ManaColorBucket::from_color(*color));
                }
                ManaProductionKind::Choice(_colors) => {
                    // Dual land - goes into Multi bucket
                    buckets.push(ManaColorBucket::Multi);
                }
                ManaProductionKind::AnyColor => {
                    // City of Brass style - goes into Multi bucket
                    buckets.push(ManaColorBucket::Multi);
                }
                ManaProductionKind::Colorless => {
                    buckets.push(ManaColorBucket::Colorless);
                }
            }
            return buckets;
        }

        // Check for dual lands by subtype (e.g., Taiga with Plains+Mountain subtypes)
        if is_land {
            let mut colors = ManaColors::new();
            for subtype in &card.subtypes {
                match subtype.as_str() {
                    "Plains" => colors.insert(ManaColor::White),
                    "Island" => colors.insert(ManaColor::Blue),
                    "Swamp" => colors.insert(ManaColor::Black),
                    "Mountain" => colors.insert(ManaColor::Red),
                    "Forest" => colors.insert(ManaColor::Green),
                    _ => {}
                }
            }
            if colors.len() >= 2 {
                buckets.push(ManaColorBucket::Multi);
            }
        }

        buckets
    }

    /// Rebuild the entire index from scratch by scanning the battlefield
    ///
    /// This is called when globally_dirty is true.
    pub fn rebuild(&mut self, game: &GameState) {
        // Clear all buckets
        for bucket in &mut self.buckets {
            bucket.clear();
        }
        self.card_buckets.clear();

        // Scan battlefield for mana producers
        for &card_id in &game.battlefield.cards {
            self.on_card_entered(card_id, game);
        }

        // Mark all buckets as dirty so capacity gets recalculated
        for bucket in &mut self.buckets {
            bucket.dirty = true;
        }

        self.globally_dirty = false;
    }

    /// Recalculate untapped counts for dirty buckets
    ///
    /// This updates the cached untapped_count for any bucket marked dirty.
    pub fn recalculate_dirty_buckets(&mut self, game: &GameState) {
        for bucket in &mut self.buckets {
            if bucket.dirty {
                bucket.untapped_count = 0;
                for &card_id in &bucket.cards {
                    if let Some(card) = game.cards.try_get(card_id) {
                        if !card.tapped {
                            // Check summoning sickness for creatures
                            let has_summoning_sickness = if card.is_creature() {
                                if let Some(entered_turn) = card.turn_entered_battlefield {
                                    entered_turn == game.turn.turn_number
                                        && !card.has_keyword(crate::core::Keyword::Haste)
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if !has_summoning_sickness {
                                bucket.untapped_count += 1;
                            }
                        }
                    }
                }
                bucket.dirty = false;
            }
        }
    }

    /// Get the total untapped count across all single-color buckets
    ///
    /// This represents the lower bound on mana production from simple sources.
    pub fn simple_untapped_total(&self) -> u8 {
        self.buckets[ManaColorBucket::White.index()].untapped_count
            + self.buckets[ManaColorBucket::Blue.index()].untapped_count
            + self.buckets[ManaColorBucket::Black.index()].untapped_count
            + self.buckets[ManaColorBucket::Red.index()].untapped_count
            + self.buckets[ManaColorBucket::Green.index()].untapped_count
            + self.buckets[ManaColorBucket::Colorless.index()].untapped_count
    }

    /// Check if there are any multi-color sources (requires greedy resolver)
    pub fn has_multi_sources(&self) -> bool {
        !self.buckets[ManaColorBucket::Multi.index()].cards.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, CardType, EntityId};
    use crate::game::GameState;

    #[test]
    fn test_bucket_add_remove() {
        let mut bucket = ManaProducerBucket::new();
        let card_id = EntityId::new(42);

        bucket.add(card_id);
        assert!(bucket.contains(card_id));
        assert!(bucket.dirty);

        bucket.dirty = false;
        assert!(bucket.remove(card_id));
        assert!(!bucket.contains(card_id));
        assert!(bucket.dirty);
    }

    #[test]
    fn test_index_basic_lands() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Mountain
        let mountain_id = game.next_card_id();
        let mut mountain = Card::new(mountain_id, "Mountain".to_string(), p1_id);
        mountain.add_type(CardType::Land);
        mountain.controller = p1_id;
        game.cards.insert(mountain_id, mountain);
        game.battlefield.add(mountain_id);

        // Create index and rebuild
        let mut index = ManaProducerIndex::new(p1_id);
        index.rebuild(&game);

        // Should have mountain in red bucket
        assert!(index.bucket(ManaColorBucket::Red).contains(mountain_id));
        assert!(!index.bucket(ManaColorBucket::Blue).contains(mountain_id));
    }

    #[test]
    fn test_index_incremental_update() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let mut index = ManaProducerIndex::new(p1_id);
        index.rebuild(&game);

        // Add a Forest incrementally
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.add_type(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        // Notify index
        assert!(index.on_card_entered(forest_id, &game));
        assert!(index.bucket(ManaColorBucket::Green).contains(forest_id));

        // Remove incrementally
        assert!(index.on_card_left(forest_id));
        assert!(!index.bucket(ManaColorBucket::Green).contains(forest_id));
    }

    #[test]
    fn test_index_tap_notification() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add an Island
        let island_id = game.next_card_id();
        let mut island = Card::new(island_id, "Island".to_string(), p1_id);
        island.add_type(CardType::Land);
        island.controller = p1_id;
        game.cards.insert(island_id, island);
        game.battlefield.add(island_id);

        let mut index = ManaProducerIndex::new(p1_id);
        index.rebuild(&game);
        index.recalculate_dirty_buckets(&game);

        // Initially untapped
        assert_eq!(index.bucket(ManaColorBucket::Blue).untapped_count, 1);

        // Tap the land
        game.cards.get_mut(island_id).unwrap().tapped = true;
        index.on_tap_changed(island_id);
        index.recalculate_dirty_buckets(&game);

        assert_eq!(index.bucket(ManaColorBucket::Blue).untapped_count, 0);
    }
}
