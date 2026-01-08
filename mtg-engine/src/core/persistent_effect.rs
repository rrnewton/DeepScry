//! Persistent effects system for tracking long-lived game effects.
//!
//! # Design Rationale: Why NOT the Command Zone
//!
//! Java Forge stores persistent effects as "virtual cards" in the command zone.
//! This approach conflates two concepts:
//!   1. The command zone (a real game zone for commanders, emblems, etc.)
//!   2. Implementation bookkeeping for persistent effects
//!
//! We explicitly reject this design. Instead, we use dedicated storage for
//! persistent effects, keeping game zones purely for game objects.
//!
//! # What is a Persistent Effect?
//!
//! A persistent effect is a game effect that:
//!   - Lasts beyond the resolution of a single spell/ability
//!   - Is NOT a static ability on a permanent (those are on the Card itself)
//!   - Is NOT a continuous effect layer (those are in continuous_effects.rs)
//!   - Needs cleanup when certain game events occur
//!
//! Examples:
//!   - Airbend: "While exiled, you may cast it for {2}" - tracks permission on exiled card
//!   - Delay: "At the beginning of your upkeep, exile this spell with N time counters"
//!   - Suspend: Track time counters and when to cast
//!   - Imprint: Remember which card was exiled with this permanent
//!
//! # Cleanup
//!
//! Persistent effects are automatically cleaned up when:
//!   - The tracked card changes zones (leaves exile, is cast, etc.)
//!   - The source permanent leaves the battlefield
//!   - The effect's duration expires (end of turn, etc.)

use crate::core::{CardId, ManaCost, PlayerId};
use serde::{Deserialize, Serialize};

/// Unique identifier for persistent effects
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersistentEffectId(u32);

impl PersistentEffectId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// A persistent effect that lasts beyond a single spell resolution.
///
/// Unlike Java Forge's approach of storing these as "virtual cards" in the
/// command zone, we use dedicated typed storage. This makes the effect's
/// semantics explicit and avoids polluting game zones with implementation
/// details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentEffect {
    /// Unique identifier for this effect
    pub id: PersistentEffectId,

    /// The card that created this effect (may have left the battlefield)
    pub source_card: CardId,

    /// Player who created/controls this effect
    pub controller: PlayerId,

    /// The specific type of persistent effect
    pub kind: PersistentEffectKind,

    /// When this effect should be automatically removed
    pub cleanup_condition: CleanupCondition,
}

/// The kind of persistent effect and its specific data.
///
/// Each variant contains only the data needed for that effect type,
/// making the semantics explicit rather than buried in key-value pairs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistentEffectKind {
    /// Airbend: Grants permission to cast an exiled card for an alternative cost.
    ///
    /// Created by: Airbend ability (Avatar set mechanic)
    /// Effect: "While exiled, its owner may cast it for {2} rather than its mana cost."
    ///
    /// The tracked_card is the exiled card. When the owner has priority and the
    /// card is still in exile, they may cast it by paying {2} instead of its
    /// mana cost.
    MayPlayFromExile {
        /// The exiled card that can be cast
        tracked_card: CardId,

        /// The alternative cost to cast (typically {2} for Airbend)
        alternative_cost: ManaCost,

        /// The card's owner (who may cast it)
        owner: PlayerId,
    },

    /// Imprint: Remembers a card exiled with this permanent.
    ///
    /// Created by: Chrome Mox, Isochron Scepter, etc.
    /// Effect: Varies, but the permanent "remembers" the exiled card.
    ///
    /// When the source permanent leaves the battlefield, the imprinted card
    /// typically stays in exile but loses any special permissions.
    Imprint {
        /// The imprinted card in exile
        imprinted_card: CardId,
    },

    /// Suspend: Card exiled with time counters, cast when counters reach zero.
    ///
    /// Created by: Suspend keyword
    /// Effect: At upkeep, remove a counter. When last counter removed, cast.
    Suspend {
        /// The suspended card in exile
        suspended_card: CardId,

        /// Current number of time counters
        time_counters: u8,
    },

    /// CantBeBlocked: Creature can't be blocked this turn.
    ///
    /// Created by: AB$ Effect with StaticAbilities$ that grant unblockable
    /// Examples: Deserter's Disciple, various evasion effects
    ///
    /// Typically cleaned up at end of turn.
    CantBeBlocked {
        /// The creature that can't be blocked
        creature: CardId,
    },

    /// MayPlayOneWithoutManaCost: Play ONE of multiple exiled cards without paying mana cost.
    ///
    /// Created by: Fire Lord Ozai's {6} ability, similar "exile and may play" effects
    /// Effect: "Until end of turn, you may play one of those cards without paying its mana cost."
    ///
    /// Key differences from MayPlayFromExile:
    /// - Tracks MULTIPLE cards (one from each opponent's library)
    /// - No mana cost (play for free)
    /// - Can only play ONE of them (effect removed after first play)
    ///
    /// Cleanup: End of turn, OR when any tracked card is played
    MayPlayOneWithoutManaCost {
        /// The exiled cards that can be played (one from each opponent)
        tracked_cards: Vec<CardId>,

        /// The player who may play one of these cards
        beneficiary: PlayerId,
    },
    // Future: Add more persistent effect types as needed
    // - Delay (cast spell at next upkeep)
    // - Cascade (exile until you hit a cheaper spell)
    // - Hideaway (look at cards, exile one face-down)
    // - Adventure (in exile, can cast creature half)
}

/// Condition that triggers automatic cleanup of a persistent effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupCondition {
    /// Remove when the tracked card leaves its current zone (e.g., exile).
    /// Typical for Airbend, Suspend, etc.
    TrackedCardLeavesZone { card: CardId, zone: crate::zones::Zone },

    /// Remove when the source permanent leaves the battlefield.
    /// Typical for Imprint effects tied to a permanent.
    SourceLeavesBattlefield { source: CardId },

    /// Remove when the tracked card is cast (even if it stays in zone briefly).
    /// For effects that grant one-time cast permission.
    TrackedCardIsCast { card: CardId },

    /// Remove at the end of the current turn.
    EndOfTurn,

    /// Never automatically remove (manual cleanup only).
    Never,

    /// Multiple conditions: remove when ANY condition is met.
    Any(Vec<CleanupCondition>),
}

/// Storage for all active persistent effects in a game.
///
/// This is the authoritative location for persistent effect data.
/// NOT stored in the command zone, NOT stored as virtual cards.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistentEffectStore {
    /// All active persistent effects
    effects: Vec<PersistentEffect>,

    /// Next ID to assign
    next_id: u32,
}

impl PersistentEffectStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
            next_id: 0,
        }
    }

    /// Add a new persistent effect and return its ID.
    pub fn add(
        &mut self,
        kind: PersistentEffectKind,
        source_card: CardId,
        controller: PlayerId,
        cleanup_condition: CleanupCondition,
    ) -> PersistentEffectId {
        let id = PersistentEffectId::new(self.next_id);
        self.next_id += 1;

        self.effects.push(PersistentEffect {
            id,
            source_card,
            controller,
            kind,
            cleanup_condition,
        });

        id
    }

    /// Remove a persistent effect by ID.
    pub fn remove(&mut self, id: PersistentEffectId) -> Option<PersistentEffect> {
        if let Some(pos) = self.effects.iter().position(|e| e.id == id) {
            Some(self.effects.remove(pos))
        } else {
            None
        }
    }

    /// Get all active persistent effects.
    pub fn all(&self) -> &[PersistentEffect] {
        &self.effects
    }

    /// Get a specific effect by ID.
    pub fn get(&self, id: PersistentEffectId) -> Option<&PersistentEffect> {
        self.effects.iter().find(|e| e.id == id)
    }

    /// Find all MayPlayFromExile effects for a specific exiled card.
    ///
    /// Used when determining if a player can cast a card from exile.
    pub fn find_may_play_from_exile(&self, card_id: CardId) -> impl Iterator<Item = &PersistentEffect> {
        self.effects.iter().filter(move |e| {
            matches!(
                &e.kind,
                PersistentEffectKind::MayPlayFromExile { tracked_card, .. }
                if *tracked_card == card_id
            )
        })
    }

    /// Check if a creature has a CantBeBlocked effect.
    ///
    /// Used during combat when determining if blockers can be declared.
    /// Returns true if ANY active effect makes this creature unblockable.
    pub fn is_creature_unblockable(&self, creature_id: CardId) -> bool {
        self.effects.iter().any(|e| {
            matches!(
                &e.kind,
                PersistentEffectKind::CantBeBlocked { creature }
                if *creature == creature_id
            )
        })
    }

    /// Find MayPlayOneWithoutManaCost effects that allow playing a specific card.
    ///
    /// Used when determining if a player can cast a card from exile for free.
    /// Returns the effect ID and beneficiary if the card is in a may-play effect.
    pub fn find_may_play_without_cost(&self, card_id: CardId) -> Option<(PersistentEffectId, PlayerId)> {
        self.effects.iter().find_map(|e| {
            if let PersistentEffectKind::MayPlayOneWithoutManaCost { tracked_cards, beneficiary } = &e.kind {
                if tracked_cards.contains(&card_id) {
                    return Some((e.id, *beneficiary));
                }
            }
            None
        })
    }

    /// Check if a player can play a specific exiled card without paying mana.
    ///
    /// Used in casting logic to offer free plays from exile.
    pub fn can_play_without_mana_cost(&self, card_id: CardId, player: PlayerId) -> bool {
        self.effects.iter().any(|e| {
            matches!(
                &e.kind,
                PersistentEffectKind::MayPlayOneWithoutManaCost { tracked_cards, beneficiary }
                if tracked_cards.contains(&card_id) && *beneficiary == player
            )
        })
    }

    /// Find all effects that should be cleaned up because a card left a zone.
    ///
    /// Returns the IDs of effects to remove.
    pub fn find_effects_to_cleanup_on_zone_change(
        &self,
        card_id: CardId,
        from_zone: crate::zones::Zone,
    ) -> Vec<PersistentEffectId> {
        self.effects
            .iter()
            .filter(|e| Self::should_cleanup_on_zone_change(&e.cleanup_condition, card_id, from_zone))
            .map(|e| e.id)
            .collect()
    }

    /// Check if a cleanup condition is triggered by a zone change.
    fn should_cleanup_on_zone_change(
        condition: &CleanupCondition,
        card_id: CardId,
        from_zone: crate::zones::Zone,
    ) -> bool {
        match condition {
            CleanupCondition::TrackedCardLeavesZone { card, zone } => *card == card_id && *zone == from_zone,
            CleanupCondition::SourceLeavesBattlefield { source } => {
                *source == card_id && from_zone == crate::zones::Zone::Battlefield
            }
            CleanupCondition::Any(conditions) => conditions
                .iter()
                .any(|c| Self::should_cleanup_on_zone_change(c, card_id, from_zone)),
            CleanupCondition::TrackedCardIsCast { .. } | CleanupCondition::EndOfTurn | CleanupCondition::Never => false,
        }
    }

    /// Find all effects that should be cleaned up because a card was cast.
    ///
    /// Returns the IDs of effects to remove.
    pub fn find_effects_to_cleanup_on_cast(&self, card_id: CardId) -> Vec<PersistentEffectId> {
        self.effects
            .iter()
            .filter(|e| Self::should_cleanup_on_cast(&e.cleanup_condition, card_id))
            .map(|e| e.id)
            .collect()
    }

    /// Check if a cleanup condition is triggered by a card being cast.
    fn should_cleanup_on_cast(condition: &CleanupCondition, card_id: CardId) -> bool {
        match condition {
            CleanupCondition::TrackedCardIsCast { card } => *card == card_id,
            CleanupCondition::Any(conditions) => conditions.iter().any(|c| Self::should_cleanup_on_cast(c, card_id)),
            CleanupCondition::TrackedCardLeavesZone { .. }
            | CleanupCondition::SourceLeavesBattlefield { .. }
            | CleanupCondition::EndOfTurn
            | CleanupCondition::Never => false,
        }
    }

    /// Find all effects that should be cleaned up at end of turn.
    ///
    /// Returns the IDs of effects to remove.
    pub fn find_effects_to_cleanup_at_eot(&self) -> Vec<PersistentEffectId> {
        self.effects
            .iter()
            .filter(|e| Self::should_cleanup_at_eot(&e.cleanup_condition))
            .map(|e| e.id)
            .collect()
    }

    /// Check if a cleanup condition is triggered at end of turn.
    fn should_cleanup_at_eot(condition: &CleanupCondition) -> bool {
        match condition {
            CleanupCondition::EndOfTurn => true,
            CleanupCondition::Any(conditions) => conditions.iter().any(Self::should_cleanup_at_eot),
            CleanupCondition::TrackedCardLeavesZone { .. }
            | CleanupCondition::SourceLeavesBattlefield { .. }
            | CleanupCondition::TrackedCardIsCast { .. }
            | CleanupCondition::Never => false,
        }
    }

    /// Remove multiple effects by their IDs.
    ///
    /// Returns the removed effects.
    pub fn remove_many(&mut self, ids: &[PersistentEffectId]) -> Vec<PersistentEffect> {
        let mut removed = Vec::new();
        for id in ids {
            if let Some(effect) = self.remove(*id) {
                removed.push(effect);
            }
        }
        removed
    }

    /// Get count of active effects.
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Check if there are no active effects.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ManaCost;
    use crate::zones::Zone;

    #[test]
    fn test_add_and_remove_effect() {
        let mut store = PersistentEffectStore::new();

        let source = CardId::new(1);
        let tracked = CardId::new(2);
        let controller = PlayerId::new(0);

        let id = store.add(
            PersistentEffectKind::MayPlayFromExile {
                tracked_card: tracked,
                alternative_cost: ManaCost::from_string("2"),
                owner: controller,
            },
            source,
            controller,
            CleanupCondition::TrackedCardLeavesZone {
                card: tracked,
                zone: Zone::Exile,
            },
        );

        assert_eq!(store.len(), 1);

        let effect = store.get(id).unwrap();
        assert_eq!(effect.source_card, source);

        let removed = store.remove(id);
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_find_may_play_from_exile() {
        let mut store = PersistentEffectStore::new();

        let source = CardId::new(1);
        let tracked1 = CardId::new(2);
        let tracked2 = CardId::new(3);
        let controller = PlayerId::new(0);

        // Add two effects for different cards
        store.add(
            PersistentEffectKind::MayPlayFromExile {
                tracked_card: tracked1,
                alternative_cost: ManaCost::from_string("2"),
                owner: controller,
            },
            source,
            controller,
            CleanupCondition::TrackedCardLeavesZone {
                card: tracked1,
                zone: Zone::Exile,
            },
        );

        store.add(
            PersistentEffectKind::MayPlayFromExile {
                tracked_card: tracked2,
                alternative_cost: ManaCost::from_string("2"),
                owner: controller,
            },
            source,
            controller,
            CleanupCondition::TrackedCardLeavesZone {
                card: tracked2,
                zone: Zone::Exile,
            },
        );

        // Should find only the effect for tracked1
        let effects: Vec<_> = store.find_may_play_from_exile(tracked1).collect();
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_cleanup_on_zone_change() {
        let mut store = PersistentEffectStore::new();

        let source = CardId::new(1);
        let tracked = CardId::new(2);
        let controller = PlayerId::new(0);

        store.add(
            PersistentEffectKind::MayPlayFromExile {
                tracked_card: tracked,
                alternative_cost: ManaCost::from_string("2"),
                owner: controller,
            },
            source,
            controller,
            CleanupCondition::TrackedCardLeavesZone {
                card: tracked,
                zone: Zone::Exile,
            },
        );

        // Should find effect to cleanup when tracked card leaves exile
        let to_cleanup = store.find_effects_to_cleanup_on_zone_change(tracked, Zone::Exile);
        assert_eq!(to_cleanup.len(), 1);

        // Should NOT find effect when a different card leaves exile
        let other_card = CardId::new(99);
        let to_cleanup = store.find_effects_to_cleanup_on_zone_change(other_card, Zone::Exile);
        assert_eq!(to_cleanup.len(), 0);

        // Should NOT find effect when tracked card leaves a different zone
        let to_cleanup = store.find_effects_to_cleanup_on_zone_change(tracked, Zone::Graveyard);
        assert_eq!(to_cleanup.len(), 0);
    }
}
