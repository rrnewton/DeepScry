//! Delayed trigger infrastructure for effects that fire on future events.
//!
//! Delayed triggers are created by effects and fire when specific conditions are met:
//! - Zone changes (e.g., "when this dies, return it to battlefield")
//! - Phase changes (e.g., "at end of turn, sacrifice this")
//! - Counter removal (e.g., Suspend "when last time counter removed, cast it")
//!
//! This is a general-purpose system used by:
//! - Earthbend: Return to battlefield when dies/exiled
//! - Flicker effects: Return at end of turn
//! - Suspend: Cast when last time counter removed
//! - End-of-turn cleanup: Sacrifice tokens, exile creatures, etc.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use super::{CardId, PlayerId};
use crate::zones::Zone;

/// Unique identifier for a delayed trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DelayedTriggerId(pub u32);

impl DelayedTriggerId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// A delayed trigger waiting to fire when a condition is met.
///
/// Created by effects like Earthbend, flicker, or suspend.
/// Stored in GameState and checked when relevant events occur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayedTrigger {
    /// Unique ID for this delayed trigger
    pub id: DelayedTriggerId,
    /// The card this trigger is watching (e.g., the earthbent land)
    pub tracked_card: CardId,
    /// The source that created this trigger (e.g., Avatar Kyoshi)
    pub source_card: CardId,
    /// Controller of the trigger effect
    pub controller: PlayerId,
    /// What event fires this trigger
    pub trigger_condition: DelayedTriggerCondition,
    /// Effect to execute when triggered
    pub effect: DelayedEffect,
    /// When to remove this trigger (even if not fired)
    pub expiry: Option<DelayedTriggerExpiry>,
}

impl DelayedTrigger {
    /// Create a new delayed trigger
    pub fn new(
        id: DelayedTriggerId,
        tracked_card: CardId,
        source_card: CardId,
        controller: PlayerId,
        trigger_condition: DelayedTriggerCondition,
        effect: DelayedEffect,
    ) -> Self {
        Self {
            id,
            tracked_card,
            source_card,
            controller,
            trigger_condition,
            effect,
            expiry: None,
        }
    }

    /// Add an expiry condition
    pub fn with_expiry(mut self, expiry: DelayedTriggerExpiry) -> Self {
        self.expiry = Some(expiry);
        self
    }

    /// Check if this trigger should fire on a zone change
    pub fn matches_zone_change(&self, card: CardId, from_zone: Zone, to_zone: Zone) -> bool {
        if card != self.tracked_card {
            return false;
        }

        match &self.trigger_condition {
            DelayedTriggerCondition::ZoneChange { from_zones, to_zones } => {
                let from_matches = from_zones.is_empty() || from_zones.contains(&from_zone);
                let to_matches = to_zones.is_empty() || to_zones.contains(&to_zone);
                from_matches && to_matches
            }
            DelayedTriggerCondition::Phase { .. } | DelayedTriggerCondition::LastCounterRemoved { .. } => false,
        }
    }
}

/// What event triggers a delayed trigger
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelayedTriggerCondition {
    /// Fire when tracked card moves between zones
    /// Example: Earthbend - when land goes to Graveyard or Exile
    ZoneChange {
        /// Source zones (empty = any zone)
        from_zones: SmallVec<[Zone; 2]>,
        /// Destination zones that fire the trigger
        to_zones: SmallVec<[Zone; 2]>,
    },

    /// Fire at the beginning of a phase
    /// Example: "At the beginning of the next end step"
    Phase {
        phase: TriggerPhase,
        /// Whose turn? (owner, opponent, or any)
        whose_turn: TurnOwner,
    },

    /// Fire when the last counter of a type is removed
    /// Example: Suspend - when last time counter is removed
    LastCounterRemoved { counter_type: super::CounterType },
}

/// Which phase triggers the delayed trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerPhase {
    Upkeep,
    Draw,
    BeginCombat,
    EndCombat,
    EndStep,
    Cleanup,
}

/// Whose turn the phase trigger fires on
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOwner {
    /// Controller of the trigger
    You,
    /// Opponents of the controller
    Opponent,
    /// Any player's turn
    Any,
}

/// What happens when the delayed trigger fires
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DelayedEffect {
    /// Return tracked card to battlefield under controller's control
    /// Used by: Earthbend, flicker effects
    ReturnToBattlefield {
        /// Should the card enter tapped?
        tapped: bool,
        /// Return to original owner (true) or trigger controller (false)?
        to_owner: bool,
    },

    /// Sacrifice the tracked card
    /// Used by: "Sacrifice at end of turn" effects
    Sacrifice,

    /// Exile the tracked card
    /// Used by: Delayed exile effects
    ExileCard,

    /// Cast the tracked card without paying mana cost
    /// Used by: Suspend
    CastWithoutPaying,

    /// Execute an effect when triggered
    /// Used by: SP$ DelayedTrigger (Fatal Fissure, etc.)
    ///
    /// The effect is stored directly rather than referencing an SVar,
    /// since SVars are resolved at parse time.
    ExecuteEffect {
        /// The effect to execute when the trigger fires
        effect: Box<super::effects::Effect>,
    },
}

/// When to remove the trigger (even if it hasn't fired)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelayedTriggerExpiry {
    /// Remove at end of turn
    EndOfTurn,
    /// Remove at end of next turn
    EndOfNextTurn,
    /// Remove when source leaves battlefield
    SourceLeavesBattlefield,
    /// Never expire (only removed when fired or manually)
    Never,
}

/// Storage for delayed triggers
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DelayedTriggerStore {
    triggers: Vec<DelayedTrigger>,
    next_id: u32,
}

impl DelayedTriggerStore {
    pub fn new() -> Self {
        Self {
            triggers: Vec::new(),
            next_id: 1,
        }
    }

    /// Add a new delayed trigger, returns its ID
    pub fn add(&mut self, mut trigger: DelayedTrigger) -> DelayedTriggerId {
        let id = DelayedTriggerId::new(self.next_id);
        self.next_id += 1;
        trigger.id = id;
        self.triggers.push(trigger);
        id
    }

    /// Remove a trigger by ID (e.g., when it fires)
    pub fn remove(&mut self, id: DelayedTriggerId) -> Option<DelayedTrigger> {
        if let Some(pos) = self.triggers.iter().position(|t| t.id == id) {
            Some(self.triggers.remove(pos))
        } else {
            None
        }
    }

    /// Find triggers that match a zone change
    pub fn find_zone_change_triggers(&self, card: CardId, from_zone: Zone, to_zone: Zone) -> Vec<DelayedTriggerId> {
        self.triggers
            .iter()
            .filter(|t| t.matches_zone_change(card, from_zone, to_zone))
            .map(|t| t.id)
            .collect()
    }

    /// Get a trigger by ID
    pub fn get(&self, id: DelayedTriggerId) -> Option<&DelayedTrigger> {
        self.triggers.iter().find(|t| t.id == id)
    }

    /// Get all triggers (for serialization/debugging)
    pub fn all(&self) -> &[DelayedTrigger] {
        &self.triggers
    }

    /// Remove triggers that have expired at end of turn
    pub fn cleanup_end_of_turn(&mut self) -> Vec<DelayedTrigger> {
        let mut removed = Vec::new();
        self.triggers.retain(|t| {
            if matches!(t.expiry, Some(DelayedTriggerExpiry::EndOfTurn)) {
                removed.push(t.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// Remove triggers whose source has left the battlefield
    pub fn cleanup_source_left(&mut self, source: CardId) -> Vec<DelayedTrigger> {
        let mut removed = Vec::new();
        self.triggers.retain(|t| {
            if t.source_card == source && matches!(t.expiry, Some(DelayedTriggerExpiry::SourceLeavesBattlefield)) {
                removed.push(t.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// Restore a trigger (for undo)
    pub fn restore(&mut self, trigger: DelayedTrigger) {
        // Ensure we don't exceed the next_id
        if trigger.id.as_u32() >= self.next_id {
            self.next_id = trigger.id.as_u32() + 1;
        }
        self.triggers.push(trigger);
    }

    /// Get count of active triggers
    pub fn len(&self) -> usize {
        self.triggers.len()
    }

    /// Check if store is empty
    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CardId;

    fn make_test_trigger(tracked: CardId, to_zones: SmallVec<[Zone; 2]>) -> DelayedTrigger {
        DelayedTrigger::new(
            DelayedTriggerId::new(0),
            tracked,
            CardId::new(100),
            crate::core::PlayerId::new(0),
            DelayedTriggerCondition::ZoneChange {
                from_zones: SmallVec::new(),
                to_zones,
            },
            DelayedEffect::ReturnToBattlefield {
                tapped: true,
                to_owner: true,
            },
        )
    }

    #[test]
    fn test_zone_change_trigger_matches() {
        let card = CardId::new(1);
        let trigger = make_test_trigger(card, smallvec::smallvec![Zone::Graveyard, Zone::Exile]);

        // Should match: card goes to graveyard
        assert!(trigger.matches_zone_change(card, Zone::Battlefield, Zone::Graveyard));

        // Should match: card goes to exile
        assert!(trigger.matches_zone_change(card, Zone::Battlefield, Zone::Exile));

        // Should NOT match: wrong card
        assert!(!trigger.matches_zone_change(CardId::new(2), Zone::Battlefield, Zone::Graveyard));

        // Should NOT match: wrong destination
        assert!(!trigger.matches_zone_change(card, Zone::Battlefield, Zone::Hand));
    }

    #[test]
    fn test_delayed_trigger_store_add_remove() {
        let mut store = DelayedTriggerStore::new();
        let card = CardId::new(1);

        let trigger = make_test_trigger(card, smallvec::smallvec![Zone::Graveyard]);
        let id = store.add(trigger);

        assert_eq!(store.len(), 1);
        assert!(store.get(id).is_some());

        let removed = store.remove(id);
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_find_zone_change_triggers() {
        let mut store = DelayedTriggerStore::new();
        let card1 = CardId::new(1);
        let card2 = CardId::new(2);

        let trigger1 = make_test_trigger(card1, smallvec::smallvec![Zone::Graveyard]);
        let trigger2 = make_test_trigger(card2, smallvec::smallvec![Zone::Exile]);

        let id1 = store.add(trigger1);
        let _id2 = store.add(trigger2);

        // Find triggers for card1 going to graveyard
        let matches = store.find_zone_change_triggers(card1, Zone::Battlefield, Zone::Graveyard);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], id1);

        // No matches for card1 going to exile (wrong destination)
        let no_matches = store.find_zone_change_triggers(card1, Zone::Battlefield, Zone::Exile);
        assert!(no_matches.is_empty());
    }
}
