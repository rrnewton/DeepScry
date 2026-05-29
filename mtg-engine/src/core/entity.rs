//! Game entity system with strongly-typed integer IDs

use crate::MtgError;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

/// Strongly-typed integer ID for game entities
///
/// Uses phantom types to distinguish between different kinds of entities
/// (Players, Cards, etc.) at compile time, while keeping the same efficient
/// integer representation at runtime.
///
/// Keeps IDs simple and contiguous for human readability and dense storage.
/// These IDs are stable throughout a game - entities don't get deallocated.
pub struct EntityId<T> {
    id: u32,
    _phantom: PhantomData<T>,
}

// Manual trait implementations that don't require T to have these traits
impl<T> Clone for EntityId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for EntityId<T> {}

impl<T> PartialEq for EntityId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for EntityId<T> {}

impl<T> PartialOrd for EntityId<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for EntityId<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl<T> std::hash::Hash for EntityId<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Sentinel value indicating "reuse the previous target" in SubAbility chains.
/// Used when parsing `Defined$ Targeted` to avoid consuming a new target.
pub const REUSE_PREVIOUS_TARGET: u32 = u32::MAX;

/// Sentinel value indicating "all players" for effects like Wheel of Fortune.
/// Used when parsing `Defined$ Player` to mean "each player".
pub const ALL_PLAYERS_ID: u32 = u32::MAX - 1;

/// Sentinel value indicating "remembered players" for conditional draw effects.
/// Used when parsing `Defined$ Remembered` to mean "draw for each player that was remembered".
pub const REMEMBERED_PLAYERS_ID: u32 = u32::MAX - 2;

/// Sentinel value indicating "the source card itself" for Defined$ Self effects.
pub const SELF_TARGET_ID: u32 = u32::MAX - 3;

/// Sentinel value indicating "the cards in `GameState::remembered_cards`" for
/// `Defined$ Remembered` on a card-targeting effect (e.g. PutCounter chained
/// after a self-exile that used `RememberChanged$ True`, like All Hallow's
/// Eve). At resolution time the engine substitutes this sentinel with each
/// card in `remembered_cards` (in practice usually a single card).
pub const REMEMBERED_CARD_ID: u32 = u32::MAX - 4;

/// Sentinel value indicating "placeholder to be resolved".
/// Used for targets/players that need runtime resolution (e.g., "you", "target creature").
pub const PLACEHOLDER_ID: u32 = 0;

/// Sentinel value indicating "target opponent" — the chosen opponent of the
/// resolving spell/ability. Distinct from `PLACEHOLDER_ID` so the resolver
/// can tell apart "ValidTgts$ Player" (Mind Twist — picks an opponent) from
/// "Defined$ You" (controller). In 2-player games we currently auto-pick the
/// sole opponent without going through the targeting UI; tracked in mtg-564.
pub const TARGET_OPPONENT_ID: u32 = u32::MAX - 5;

/// Sentinel range for encoding a Player as a CardId inside the
/// `valid_targets` slice returned to `Controller::choose_targets`. We do
/// this so controllers can offer Players as targets for `ValidTgts$ Any`
/// effects (Lightning Bolt) without changing the trait signature.
/// `PlayerId(n)` is encoded as `PLAYER_TARGET_BASE - n`.
/// The base is u32::MAX - 1000 to keep a safe gap from other sentinels and
/// avoid colliding with realistic CardId values (which grow upward from 0).
/// See `core::player_as_target_sentinel` / `core::player_target_from_sentinel`
/// and mtg-565 (Lightning Bolt player-target bug).
pub const PLAYER_TARGET_BASE: u32 = u32::MAX - 1000;

impl<T> EntityId<T> {
    #[inline]
    pub fn new(id: u32) -> Self {
        EntityId {
            id,
            _phantom: PhantomData,
        }
    }

    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.id
    }

    /// Check if this ID is a placeholder that needs resolution.
    /// Placeholders (ID 0) are used for targets/players that must be
    /// resolved at runtime (e.g., "you", "target creature", "any target").
    #[inline]
    pub fn is_placeholder(&self) -> bool {
        self.id == PLACEHOLDER_ID
    }

    /// Create a placeholder ID for later resolution.
    #[inline]
    pub fn placeholder() -> Self {
        EntityId::new(PLACEHOLDER_ID)
    }

    /// Check if this ID means "all players" (for effects like Wheel of Fortune).
    #[inline]
    pub fn is_all_players(&self) -> bool {
        self.id == ALL_PLAYERS_ID
    }

    /// Create a sentinel ID meaning "all players".
    #[inline]
    pub fn all_players() -> Self {
        EntityId::new(ALL_PLAYERS_ID)
    }

    /// Check if this ID means "remembered players" (for effects like Raphael's Technique).
    #[inline]
    pub fn is_remembered_players(&self) -> bool {
        self.id == REMEMBERED_PLAYERS_ID
    }

    /// Create a sentinel ID meaning "remembered players".
    #[inline]
    pub fn remembered_players() -> Self {
        EntityId::new(REMEMBERED_PLAYERS_ID)
    }

    /// Check if this ID means "the source card itself" (Defined$ Self).
    #[inline]
    pub fn is_self_target(&self) -> bool {
        self.id == SELF_TARGET_ID
    }

    /// Create a sentinel ID meaning "the source card itself".
    #[inline]
    pub fn self_target() -> Self {
        EntityId::new(SELF_TARGET_ID)
    }

    /// Check if this ID means "the cards in `GameState::remembered_cards`"
    /// (`Defined$ Remembered` on a card-targeting effect).
    ///
    /// Currently used by `PutCounter` chained after a `RememberChanged$ True`
    /// self-exile (e.g. All Hallow's Eve).
    #[inline]
    pub fn is_remembered_card(&self) -> bool {
        self.id == REMEMBERED_CARD_ID
    }

    /// Create a sentinel ID meaning "the cards in `remembered_cards`".
    #[inline]
    pub fn remembered_card() -> Self {
        EntityId::new(REMEMBERED_CARD_ID)
    }

    /// Check if this ID is the "reuse previous target" sentinel.
    /// Used in SubAbility chains where `Defined$ Targeted` means
    /// "use the same target as the parent ability".
    #[inline]
    pub fn is_reuse_previous(&self) -> bool {
        self.id == REUSE_PREVIOUS_TARGET
    }

    /// Create a sentinel ID meaning "reuse the previous target".
    #[inline]
    pub fn reuse_previous() -> Self {
        EntityId::new(REUSE_PREVIOUS_TARGET)
    }

    /// Check if this ID is the "target opponent" sentinel
    /// (Mind Twist-style `ValidTgts$ Player`; tracked in mtg-564).
    #[inline]
    pub fn is_target_opponent(&self) -> bool {
        self.id == TARGET_OPPONENT_ID
    }

    /// Create the "target opponent" sentinel.
    #[inline]
    pub fn target_opponent() -> Self {
        EntityId::new(TARGET_OPPONENT_ID)
    }
}

// Custom Debug implementation to print just the ID number
impl<T> fmt::Debug for EntityId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

impl<T> fmt::Display for EntityId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

// Manual Serialize/Deserialize implementations to handle PhantomData
impl<T> Serialize for EntityId<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.id.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for EntityId<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let id = u32::deserialize(deserializer)?;
        Ok(EntityId::new(id))
    }
}

/// Base trait for all game entities with typed IDs
pub trait GameEntity<T> {
    fn id(&self) -> EntityId<T>;
    fn name(&self) -> &str;
}

/// Central storage for all game entities of a specific type
///
/// Provides O(1) lookup by EntityId using Vec-based indexing.
/// Uses sparse storage to handle IDs from a shared global counter
/// (e.g., Cards may have IDs 2, 3, 4... if 0, 1 were used for Players).
///
/// The type parameter T ensures type safety - `EntityId<T>` can only
/// look up entities of type T.
///
/// ## Performance
///
/// Vec indexing is significantly faster than HashMap lookup because:
/// - No hash computation required
/// - No collision handling
/// - Better cache locality (contiguous memory)
/// - No pointer chasing through HashMap buckets
///
/// Callgrind profiling showed EntityStore lookups consuming ~10-14% of CPU
/// when using HashMap due to hashing overhead in hot paths.
#[derive(Debug, Clone)]
pub struct EntityStore<T>
where
    T: Clone,
{
    /// Sparse storage of entities indexed by EntityId.
    /// Entry at index i corresponds to EntityId(i).
    /// Uses Option<T> to allow gaps for IDs used by other entity types.
    entities: Vec<Option<T>>,
}

// Manual Serialize/Deserialize implementations
impl<T> Serialize for EntityStore<T>
where
    T: Serialize + Clone,
{
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a Vec - the index IS the EntityId
        self.entities.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for EntityStore<T>
where
    T: Deserialize<'de> + Clone,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Deserialize from Vec - the index IS the EntityId
        let entities = Vec::deserialize(deserializer)?;
        Ok(EntityStore { entities })
    }
}

impl<T> EntityStore<T>
where
    T: Clone,
{
    pub fn new() -> Self {
        EntityStore { entities: Vec::new() }
    }

    /// Create a new EntityStore with pre-allocated capacity
    ///
    /// This avoids Vec resizes during initial entity loading.
    /// Use this when you know the approximate number of entities upfront.
    pub fn with_capacity(capacity: usize) -> Self {
        EntityStore {
            entities: Vec::with_capacity(capacity),
        }
    }

    /// Generate a new unique EntityId
    ///
    /// Note: This returns the next sequential ID for this store, but since
    /// IDs may come from a global counter shared with other entity types,
    /// prefer using the global ID generator (GameState::next_id) instead.
    pub fn next_id(&mut self) -> EntityId<T> {
        EntityId::new(self.entities.len() as u32)
    }

    /// Insert an entity with a specific ID (write-once)
    ///
    /// IDs can be sparse (not sequential) since they come from a global counter
    /// shared with other entity types. The Vec is extended with None entries
    /// as needed to accommodate the ID.
    ///
    /// # Panics
    ///
    /// Panics if the slot is already occupied. EntityStore is write-once:
    /// entities are created and never replaced. Use `contains()` to check
    /// if a slot is occupied before inserting if needed.
    pub fn insert(&mut self, id: EntityId<T>, entity: T) {
        let idx = id.as_u32() as usize;

        // Extend the Vec if needed
        if idx >= self.entities.len() {
            self.entities.resize_with(idx + 1, || None);
        }

        // Enforce write-once: slot must be empty
        if self.entities[idx].is_some() {
            panic!(
                "EntityStore::insert(): slot {} is already occupied - EntityStore is write-once",
                idx
            );
        }

        self.entities[idx] = Some(entity);
    }

    /// Insert an entity only if the slot is vacant
    ///
    /// Returns true if the entity was inserted, false if the slot was already occupied.
    /// This is useful for network clients that may receive duplicate reveals.
    pub fn insert_if_vacant(&mut self, id: EntityId<T>, entity: T) -> bool {
        let idx = id.as_u32() as usize;

        // Extend the Vec if needed
        if idx >= self.entities.len() {
            self.entities.resize_with(idx + 1, || None);
        }

        if self.entities[idx].is_some() {
            false
        } else {
            self.entities[idx] = Some(entity);
            true
        }
    }

    /// Get an entity by ID
    ///
    /// # Errors
    ///
    /// Returns `MtgError::EntityNotFound` if the entity ID does not exist.
    #[inline]
    pub fn get(&self, id: EntityId<T>) -> Result<&T> {
        // Guard against sentinel values (REUSE_PREVIOUS_TARGET=u32::MAX, ALL_PLAYERS=u32::MAX-1)
        // These are control-flow markers, not real entity IDs.
        // Note: is_placeholder() (id==0) is NOT guarded here because 0 IS a valid entity index.
        if id.is_reuse_previous() || id.as_u32() == ALL_PLAYERS_ID || id.is_self_target() {
            return Err(MtgError::EntityNotFound(id.as_u32()));
        }
        self.entities
            .get(id.as_u32() as usize)
            .and_then(|opt| opt.as_ref())
            .ok_or(MtgError::EntityNotFound(id.as_u32()))
    }

    /// Get an entity by ID, returning Option instead of Result
    ///
    /// This is more efficient than `get()` for hot paths where we don't need
    /// detailed error information. Avoids the overhead of Result<_, MtgError>
    /// construction and drop, which can be significant in tight loops.
    #[inline]
    pub fn try_get(&self, id: EntityId<T>) -> Option<&T> {
        self.entities.get(id.as_u32() as usize).and_then(|opt| opt.as_ref())
    }

    /// Get a mutable reference to an entity, returning Option instead of Result
    ///
    /// See `try_get()` for rationale on why this is more efficient for hot paths.
    #[inline]
    pub fn try_get_mut(&mut self, id: EntityId<T>) -> Option<&mut T> {
        self.entities.get_mut(id.as_u32() as usize).and_then(|opt| opt.as_mut())
    }

    /// Get a mutable reference to an entity
    ///
    /// # Errors
    ///
    /// Returns `MtgError::EntityNotFound` if the entity ID does not exist.
    #[inline]
    pub fn get_mut(&mut self, id: EntityId<T>) -> Result<&mut T> {
        self.entities
            .get_mut(id.as_u32() as usize)
            .and_then(|opt| opt.as_mut())
            .ok_or(MtgError::EntityNotFound(id.as_u32()))
    }

    /// Check if an entity exists
    #[inline]
    pub fn contains(&self, id: EntityId<T>) -> bool {
        self.entities
            .get(id.as_u32() as usize)
            .map(|opt| opt.is_some())
            .unwrap_or(false)
    }

    // =========================================================================
    // LATE-BINDING CARDID SUPPORT (mtg-218)
    // =========================================================================
    //
    // These methods support the late-binding CardID<=>CardName architecture where
    // CardIDs are pre-allocated at game start, but the actual card entity is only
    // inserted when the card is revealed to the player.

    /// Reserve a slot for an entity that will be revealed later
    ///
    /// Called during game initialization to pre-allocate CardIDs. The slot
    /// remains None until insert() is called with the revealed entity.
    /// This ensures the Vec has sufficient capacity without inserting entities.
    ///
    /// # Example
    /// ```ignore
    /// // Pre-allocate slots for a 40-card deck
    /// for i in 0..40 {
    ///     store.reserve(EntityId::new(i));
    /// }
    /// // Later, when card is revealed:
    /// store.insert(EntityId::new(5), revealed_card);
    /// ```
    pub fn reserve(&mut self, id: EntityId<T>) {
        let idx = id.as_u32() as usize;
        if idx >= self.entities.len() {
            self.entities.resize_with(idx + 1, || None);
        }
        // Note: We don't check if occupied - reserve is just ensuring capacity.
        // The slot may already have an entity (if revealed) or be None (unrevealed).
    }

    /// Reserve a contiguous range of slots [start, end)
    ///
    /// More efficient than calling reserve() in a loop.
    /// Used during game initialization to pre-allocate CardIDs for both decks.
    pub fn reserve_range(&mut self, start: EntityId<T>, count: u32) {
        let end_idx = (start.as_u32() + count) as usize;
        if end_idx > self.entities.len() {
            self.entities.resize_with(end_idx, || None);
        }
    }

    /// Check if a slot is revealed (has an entity) vs unrevealed (None)
    ///
    /// Returns true if the slot contains Some(entity), false if None or out of bounds.
    /// This is semantically equivalent to contains() but named for clarity in the
    /// late-binding context where "revealed" means "identity is known".
    #[inline]
    pub fn is_revealed(&self, id: EntityId<T>) -> bool {
        self.contains(id)
    }

    /// Clear a slot back to None (for undo of RevealCard)
    ///
    /// Returns the removed entity if present, None if slot was empty or out of bounds.
    /// This is used when undoing a RevealCard action to "unreveal" a card.
    ///
    /// # Note
    /// Unlike the old remove() which panicked, this gracefully handles the operation.
    /// The slot remains in the Vec as None, preserving the sparse storage structure.
    pub fn clear(&mut self, id: EntityId<T>) -> Option<T> {
        let idx = id.as_u32() as usize;
        if idx < self.entities.len() {
            self.entities[idx].take()
        } else {
            None
        }
    }

    /// Remove an entity (not supported - entities are never removed)
    ///
    /// This method exists only for API compatibility but will panic if called.
    ///
    /// # Panics
    ///
    /// Always panics. Entities are never removed in MTG games.
    #[allow(unused_variables)]
    pub fn remove(&mut self, id: EntityId<T>) -> Option<T> {
        panic!("EntityStore::remove() is not supported - entities are never removed in MTG games")
    }

    /// Iterate over all entities with their IDs (skips None entries)
    pub fn iter(&self) -> impl Iterator<Item = (EntityId<T>, &T)> {
        self.entities
            .iter()
            .enumerate()
            .filter_map(|(idx, opt)| opt.as_ref().map(|entity| (EntityId::new(idx as u32), entity)))
    }

    /// Iterate over all entities without IDs (more efficient, skips None entries)
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entities.iter().filter_map(|opt| opt.as_ref())
    }

    /// Get count of actual entities (not including None gaps)
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.iter().filter(|opt| opt.is_some()).count()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        !self.entities.iter().any(|opt| opt.is_some())
    }
}

impl<T> Default for EntityStore<T>
where
    T: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestEntity {
        id: EntityId<TestEntity>,
        name: String,
    }

    impl GameEntity<TestEntity> for TestEntity {
        fn id(&self) -> EntityId<TestEntity> {
            self.id
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_entity_store_sequential() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        let id1 = EntityId::new(0);
        let id2 = EntityId::new(1);

        let entity1 = TestEntity {
            id: id1,
            name: "Test1".to_string(),
        };
        let entity2 = TestEntity {
            id: id2,
            name: "Test2".to_string(),
        };

        store.insert(id1, entity1);
        store.insert(id2, entity2);

        assert_eq!(store.len(), 2);
        assert_eq!(store.get(id1).unwrap().name, "Test1");
        assert_eq!(store.get(id2).unwrap().name, "Test2");
        assert!(store.get(EntityId::new(999)).is_err());
    }

    #[test]
    fn test_entity_store_sparse() {
        // Test sparse IDs (simulating global counter where IDs 0,1 were used by another type)
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        let id2 = EntityId::new(2);
        let id5 = EntityId::new(5);

        let entity1 = TestEntity {
            id: id2,
            name: "Test2".to_string(),
        };
        let entity2 = TestEntity {
            id: id5,
            name: "Test5".to_string(),
        };

        store.insert(id2, entity1);
        store.insert(id5, entity2);

        assert_eq!(store.len(), 2); // Only 2 actual entities
        assert_eq!(store.get(id2).unwrap().name, "Test2");
        assert_eq!(store.get(id5).unwrap().name, "Test5");
        assert!(store.get(EntityId::new(0)).is_err()); // Gap
        assert!(store.get(EntityId::new(1)).is_err()); // Gap
        assert!(store.get(EntityId::new(3)).is_err()); // Gap
        assert!(store.get(EntityId::new(4)).is_err()); // Gap
        assert!(store.get(EntityId::new(999)).is_err()); // Out of bounds
    }

    #[test]
    fn test_entity_store_iter_skips_none() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        // Insert only at positions 2 and 5 (sparse)
        store.insert(
            EntityId::new(2),
            TestEntity {
                id: EntityId::new(2),
                name: "A".to_string(),
            },
        );
        store.insert(
            EntityId::new(5),
            TestEntity {
                id: EntityId::new(5),
                name: "B".to_string(),
            },
        );

        // iter() should return exactly 2 items, skipping None entries
        let items: Vec<_> = store.iter().collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0.as_u32(), 2);
        assert_eq!(items[0].1.name, "A");
        assert_eq!(items[1].0.as_u32(), 5);
        assert_eq!(items[1].1.name, "B");
    }

    #[test]
    #[should_panic(expected = "EntityStore::insert(): slot 0 is already occupied")]
    fn test_entity_store_write_once_panics_on_double_insert() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        let id = EntityId::new(0);
        let entity1 = TestEntity {
            id,
            name: "First".to_string(),
        };
        let entity2 = TestEntity {
            id,
            name: "Second".to_string(),
        };

        store.insert(id, entity1);
        // This should panic because slot is already occupied
        store.insert(id, entity2);
    }

    #[test]
    fn test_entity_store_insert_if_vacant() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        let id = EntityId::new(0);
        let entity1 = TestEntity {
            id,
            name: "First".to_string(),
        };
        let entity2 = TestEntity {
            id,
            name: "Second".to_string(),
        };

        // First insert should succeed
        assert!(store.insert_if_vacant(id, entity1));
        assert_eq!(store.get(id).unwrap().name, "First");

        // Second insert should fail but not panic
        assert!(!store.insert_if_vacant(id, entity2));
        // Original value should still be there
        assert_eq!(store.get(id).unwrap().name, "First");
    }

    // =========================================================================
    // LATE-BINDING CARDID TESTS (mtg-218)
    // =========================================================================

    #[test]
    fn test_entity_store_reserve() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        // Reserve slots for a "40-card deck"
        for i in 0..40 {
            store.reserve(EntityId::new(i));
        }

        // All slots should exist but be None (unrevealed)
        for i in 0..40 {
            assert!(!store.is_revealed(EntityId::new(i)));
            assert!(!store.contains(EntityId::new(i)));
            assert!(store.get(EntityId::new(i)).is_err());
        }

        // len() should still be 0 (no actual entities)
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());

        // Now "reveal" card 5
        let id5 = EntityId::new(5);
        let entity = TestEntity {
            id: id5,
            name: "Lightning Bolt".to_string(),
        };
        store.insert(id5, entity);

        // Card 5 should now be revealed
        assert!(store.is_revealed(id5));
        assert!(store.contains(id5));
        assert_eq!(store.get(id5).unwrap().name, "Lightning Bolt");

        // Other cards still unrevealed
        assert!(!store.is_revealed(EntityId::new(0)));
        assert!(!store.is_revealed(EntityId::new(39)));

        // len() should be 1
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_entity_store_reserve_range() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        // Reserve slots for two decks: 0..40 and 40..80
        store.reserve_range(EntityId::new(0), 40);
        store.reserve_range(EntityId::new(40), 40);

        // All 80 slots should be reserved but unrevealed
        for i in 0..80 {
            assert!(!store.is_revealed(EntityId::new(i)));
        }

        // Insert at slot 50 (second deck)
        let id50 = EntityId::new(50);
        store.insert(
            id50,
            TestEntity {
                id: id50,
                name: "Mountain".to_string(),
            },
        );

        assert!(store.is_revealed(id50));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_entity_store_clear() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        // Insert an entity
        let id = EntityId::new(5);
        let entity = TestEntity {
            id,
            name: "Serra Angel".to_string(),
        };
        store.insert(id, entity);

        assert!(store.is_revealed(id));
        assert_eq!(store.len(), 1);

        // Clear (unreveal) the entity
        let removed = store.clear(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "Serra Angel");

        // Slot should now be unrevealed
        assert!(!store.is_revealed(id));
        assert!(!store.contains(id));
        assert_eq!(store.len(), 0);

        // Can insert again (re-reveal)
        store.insert(
            id,
            TestEntity {
                id,
                name: "Serra Angel Again".to_string(),
            },
        );
        assert!(store.is_revealed(id));
        assert_eq!(store.get(id).unwrap().name, "Serra Angel Again");
    }

    #[test]
    fn test_entity_store_clear_empty_slot() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();

        // Reserve but don't insert
        store.reserve(EntityId::new(5));

        // Clear should return None (no entity to remove)
        let removed = store.clear(EntityId::new(5));
        assert!(removed.is_none());

        // Clear out-of-bounds should also return None
        let removed = store.clear(EntityId::new(999));
        assert!(removed.is_none());
    }

    #[test]
    fn test_entity_store_late_binding_workflow() {
        // Simulates the full late-binding workflow:
        // 1. Pre-allocate CardIDs at game start
        // 2. Cards start unrevealed
        // 3. Reveal cards as they become known
        // 4. Undo reveal (clear)

        let mut store: EntityStore<TestEntity> = EntityStore::new();

        // Game start: allocate IDs for two 40-card decks
        store.reserve_range(EntityId::new(0), 40); // P1's deck
        store.reserve_range(EntityId::new(40), 40); // P2's deck

        // Initial state: all unrevealed
        assert_eq!(store.len(), 0);

        // P1 draws card 0 and it's revealed to them
        let card0 = EntityId::new(0);
        store.insert(
            card0,
            TestEntity {
                id: card0,
                name: "Lightning Bolt".to_string(),
            },
        );

        // P1 knows card 0
        assert!(store.is_revealed(card0));
        // P2's card 40 is still unknown
        assert!(!store.is_revealed(EntityId::new(40)));

        // P1 plays card 0 onto stack - P2 now sees it
        // (In P2's store, they would now insert it)

        // Undo the reveal (for undo functionality)
        store.clear(card0);
        assert!(!store.is_revealed(card0));

        // Re-reveal (redo)
        store.insert(
            card0,
            TestEntity {
                id: card0,
                name: "Lightning Bolt".to_string(),
            },
        );
        assert!(store.is_revealed(card0));
    }
}
