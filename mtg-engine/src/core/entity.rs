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

impl<T> EntityId<T> {
    pub fn new(id: u32) -> Self {
        EntityId {
            id,
            _phantom: PhantomData,
        }
    }

    pub fn as_u32(&self) -> u32 {
        self.id
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
    #[inline]
    pub fn get(&self, id: EntityId<T>) -> Result<&T> {
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

    /// Remove an entity (not supported - entities are never removed)
    ///
    /// This method exists only for API compatibility but will panic if called.
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

        store.insert(id1, entity1.clone());
        store.insert(id2, entity2.clone());

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

        store.insert(id2, entity1.clone());
        store.insert(id5, entity2.clone());

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
}
