//! Game entity system with strongly-typed integer IDs

use crate::MtgError;
use crate::Result;
use rustc_hash::FxHashMap;
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
/// Provides fast lookup by EntityId and manages entity lifecycle.
/// Uses FxHashMap for fast hashing of integer keys.
/// The type parameter T ensures type safety - `EntityId<T>` can only
/// look up entities of type T.
#[derive(Debug, Clone)]
pub struct EntityStore<T>
where
    T: Clone,
{
    entities: FxHashMap<EntityId<T>, T>,
    next_id: u32,
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
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("EntityStore", 2)?;
        state.serialize_field("entities", &self.entities)?;
        state.serialize_field("next_id", &self.next_id)?;
        state.end()
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
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Entities,
            NextId,
        }

        struct EntityStoreVisitor<T> {
            marker: PhantomData<T>,
        }

        impl<'de, T> serde::de::Visitor<'de> for EntityStoreVisitor<T>
        where
            T: Deserialize<'de> + Clone,
        {
            type Value = EntityStore<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct EntityStore")
            }

            fn visit_map<V>(self, mut map: V) -> std::result::Result<EntityStore<T>, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut entities = None;
                let mut next_id = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Entities => {
                            if entities.is_some() {
                                return Err(serde::de::Error::duplicate_field("entities"));
                            }
                            entities = Some(map.next_value()?);
                        }
                        Field::NextId => {
                            if next_id.is_some() {
                                return Err(serde::de::Error::duplicate_field("next_id"));
                            }
                            next_id = Some(map.next_value()?);
                        }
                    }
                }
                let entities = entities.ok_or_else(|| serde::de::Error::missing_field("entities"))?;
                let next_id = next_id.ok_or_else(|| serde::de::Error::missing_field("next_id"))?;
                Ok(EntityStore { entities, next_id })
            }
        }

        const FIELDS: &[&str] = &["entities", "next_id"];
        deserializer.deserialize_struct("EntityStore", FIELDS, EntityStoreVisitor { marker: PhantomData })
    }
}

impl<T> EntityStore<T>
where
    T: Clone,
{
    pub fn new() -> Self {
        EntityStore {
            entities: FxHashMap::default(),
            next_id: 0,
        }
    }

    /// Generate a new unique EntityId
    pub fn next_id(&mut self) -> EntityId<T> {
        let id = EntityId::new(self.next_id);
        self.next_id += 1;
        id
    }

    /// Insert an entity with a specific ID
    pub fn insert(&mut self, id: EntityId<T>, entity: T) {
        self.entities.insert(id, entity);
    }

    /// Get an entity by ID
    pub fn get(&self, id: EntityId<T>) -> Result<&T> {
        self.entities.get(&id).ok_or(MtgError::EntityNotFound(id.as_u32()))
    }

    /// Get a mutable reference to an entity
    pub fn get_mut(&mut self, id: EntityId<T>) -> Result<&mut T> {
        self.entities.get_mut(&id).ok_or(MtgError::EntityNotFound(id.as_u32()))
    }

    /// Check if an entity exists
    pub fn contains(&self, id: EntityId<T>) -> bool {
        self.entities.contains_key(&id)
    }

    /// Remove an entity (rarely used - entities typically persist)
    pub fn remove(&mut self, id: EntityId<T>) -> Option<T> {
        self.entities.remove(&id)
    }

    /// Iterate over all entities
    pub fn iter(&self) -> impl Iterator<Item = (&EntityId<T>, &T)> {
        self.entities.iter()
    }

    /// Get count of entities
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
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
    fn test_entity_store() {
        let mut store: EntityStore<TestEntity> = EntityStore::new();
        let id1 = store.next_id();
        let id2 = store.next_id();

        assert_eq!(id1.as_u32(), 0);
        assert_eq!(id2.as_u32(), 1);

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
}
