//! Game zones (Library, Hand, Graveyard, Battlefield, etc.)

use crate::core::{CardId, PlayerId};
use serde::{Deserialize, Serialize};
use std::alloc::{Allocator, Global};

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
#[derive(Debug, Clone)]
pub struct CardZone<A: Allocator + Clone = Global> {
    /// Zone type
    pub zone_type: Zone,

    /// Owner of this zone (each player has their own zones)
    pub owner: PlayerId,

    /// Cards in this zone (order matters for Library and Graveyard)
    pub cards: Vec<CardId, A>,
}

// Manual Serialize implementation - serialize only the data, not the allocator
impl<A: Allocator + Clone> Serialize for CardZone<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("CardZone", 3)?;
        state.serialize_field("zone_type", &self.zone_type)?;
        state.serialize_field("owner", &self.owner)?;
        state.serialize_field("cards", &self.cards.as_slice())?;
        state.end()
    }
}

// Manual Deserialize implementation - reconstruct with Global allocator
impl<'de> Deserialize<'de> for CardZone {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            ZoneType,
            Owner,
            Cards,
        }

        struct CardZoneVisitor;

        impl<'de> serde::de::Visitor<'de> for CardZoneVisitor {
            type Value = CardZone;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct CardZone")
            }

            fn visit_map<V>(self, mut map: V) -> Result<CardZone, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut zone_type = None;
                let mut owner = None;
                let mut cards = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::ZoneType => {
                            zone_type = Some(map.next_value()?);
                        }
                        Field::Owner => {
                            owner = Some(map.next_value()?);
                        }
                        Field::Cards => {
                            cards = Some(map.next_value::<Vec<CardId>>()?);
                        }
                    }
                }

                let zone_type = zone_type.ok_or_else(|| serde::de::Error::missing_field("zone_type"))?;
                let owner = owner.ok_or_else(|| serde::de::Error::missing_field("owner"))?;
                let cards = cards.ok_or_else(|| serde::de::Error::missing_field("cards"))?;

                Ok(CardZone {
                    zone_type,
                    owner,
                    cards,
                })
            }
        }

        const FIELDS: &[&str] = &["zone_type", "owner", "cards"];
        deserializer.deserialize_struct("CardZone", FIELDS, CardZoneVisitor)
    }
}

impl<A: Allocator + Clone> CardZone<A> {
    pub fn new_in(zone_type: Zone, owner: PlayerId, alloc: A) -> Self {
        CardZone {
            zone_type,
            owner,
            cards: Vec::new_in(alloc),
        }
    }
}

impl CardZone {
    pub fn new(zone_type: Zone, owner: PlayerId) -> Self {
        CardZone {
            zone_type,
            owner,
            cards: Vec::new(),
        }
    }
}

impl<A: Allocator + Clone> CardZone<A> {

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
#[derive(Debug, Clone)]
pub struct PlayerZones<A: Allocator + Clone = Global> {
    pub library: CardZone<A>,
    pub hand: CardZone<A>,
    pub graveyard: CardZone<A>,
    pub exile: CardZone<A>,
}

// Manual Serialize implementation
impl<A: Allocator + Clone> Serialize for PlayerZones<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("PlayerZones", 4)?;
        state.serialize_field("library", &self.library)?;
        state.serialize_field("hand", &self.hand)?;
        state.serialize_field("graveyard", &self.graveyard)?;
        state.serialize_field("exile", &self.exile)?;
        state.end()
    }
}

// Manual Deserialize implementation
impl<'de> Deserialize<'de> for PlayerZones {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PlayerZonesData {
            library: CardZone,
            hand: CardZone,
            graveyard: CardZone,
            exile: CardZone,
        }

        let data = PlayerZonesData::deserialize(deserializer)?;
        Ok(PlayerZones {
            library: data.library,
            hand: data.hand,
            graveyard: data.graveyard,
            exile: data.exile,
        })
    }
}

impl<A: Allocator + Clone> PlayerZones<A> {
    pub fn new_in(player_id: PlayerId, alloc: A) -> Self {
        PlayerZones {
            library: CardZone::new_in(Zone::Library, player_id, alloc.clone()),
            hand: CardZone::new_in(Zone::Hand, player_id, alloc.clone()),
            graveyard: CardZone::new_in(Zone::Graveyard, player_id, alloc.clone()),
            exile: CardZone::new_in(Zone::Exile, player_id, alloc),
        }
    }

    pub fn get_zone(&self, zone: Zone) -> Option<&CardZone<A>> {
        match zone {
            Zone::Library => Some(&self.library),
            Zone::Hand => Some(&self.hand),
            Zone::Graveyard => Some(&self.graveyard),
            Zone::Exile => Some(&self.exile),
            _ => None,
        }
    }

    pub fn get_zone_mut(&mut self, zone: Zone) -> Option<&mut CardZone<A>> {
        match zone {
            Zone::Library => Some(&mut self.library),
            Zone::Hand => Some(&mut self.hand),
            Zone::Graveyard => Some(&mut self.graveyard),
            Zone::Exile => Some(&mut self.exile),
            _ => None,
        }
    }
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
