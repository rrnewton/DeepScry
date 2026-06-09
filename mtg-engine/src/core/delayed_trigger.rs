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
    /// Numeric value remembered at registration time (`RememberNumber$ True`).
    ///
    /// Used by cards like Mana Drain whose delayed trigger adds an amount of
    /// mana equal to a value computed when the trigger was created (the
    /// countered spell's mana value). Captured into the trigger so it survives
    /// the `DB$ Cleanup | ClearRemembered$ True` that runs in the same
    /// resolution, and so it is part of serialized game state.
    #[serde(default)]
    pub remembered_amount: Option<u32>,
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
            remembered_amount: None,
        }
    }

    /// Add an expiry condition
    pub fn with_expiry(mut self, expiry: DelayedTriggerExpiry) -> Self {
        self.expiry = Some(expiry);
        self
    }

    /// Attach a remembered numeric value (`RememberNumber$ True`).
    pub fn with_remembered_amount(mut self, amount: Option<u32>) -> Self {
        self.remembered_amount = amount;
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
            DelayedTriggerCondition::Phase { .. }
            | DelayedTriggerCondition::LastCounterRemoved { .. }
            | DelayedTriggerCondition::SpellCast { .. } => false,
        }
    }

    /// Check if this trigger should fire when a spell is cast
    ///
    /// # Parameters
    /// - `caster`: The player who cast the spell
    /// - `spell_types`: Types of the spell being cast (e.g., ["Sorcery", "Lesson"])
    ///
    /// # Returns
    /// `true` if this delayed trigger should fire for this spell cast
    pub fn matches_spell_cast(&self, caster: super::PlayerId, spell_types: &[&str]) -> bool {
        match &self.trigger_condition {
            DelayedTriggerCondition::SpellCast {
                valid_card_type,
                you_only,
            } => {
                // Check if the caster matches
                if *you_only && caster != self.controller {
                    return false;
                }

                // Check if the spell type matches
                match valid_card_type {
                    Some(required_type) => {
                        // Check if any of the spell's types match the required type
                        spell_types.iter().any(|t| t.eq_ignore_ascii_case(required_type))
                    }
                    None => true, // No type restriction
                }
            }
            DelayedTriggerCondition::ZoneChange { .. }
            | DelayedTriggerCondition::Phase { .. }
            | DelayedTriggerCondition::LastCounterRemoved { .. } => false,
        }
    }

    /// Check if this trigger should fire at the beginning of `phase`.
    ///
    /// # Parameters
    /// - `phase`: the phase whose beginning we just reached
    /// - `active_player`: the player whose turn it is right now
    pub fn matches_phase(&self, phase: TriggerPhase, active_player: super::PlayerId) -> bool {
        match &self.trigger_condition {
            DelayedTriggerCondition::Phase { phases, whose_turn } => {
                if !phases.contains(&phase) {
                    return false;
                }
                match whose_turn {
                    TurnOwner::You => active_player == self.controller,
                    TurnOwner::Opponent => active_player != self.controller,
                    TurnOwner::Any => true,
                }
            }
            DelayedTriggerCondition::ZoneChange { .. }
            | DelayedTriggerCondition::SpellCast { .. }
            | DelayedTriggerCondition::LastCounterRemoved { .. } => false,
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

    /// Fire at the beginning of one of a set of phases.
    ///
    /// Example: Mana Drain's "At the beginning of your next main phase"
    /// (`Phase$ Main1,Main2`) fires at whichever main phase comes first.
    Phase {
        /// Phases that fire the trigger (the trigger fires at the first one
        /// reached that also satisfies `whose_turn`). A set rather than a
        /// single phase so `Main1,Main2` = "your next main phase, either one".
        phases: SmallVec<[TriggerPhase; 2]>,
        /// Whose turn? (controller, opponent, or any)
        whose_turn: TurnOwner,
    },

    /// Fire when the last counter of a type is removed
    /// Example: Suspend - when last time counter is removed
    LastCounterRemoved { counter_type: super::CounterType },

    /// Fire when a spell is cast matching certain criteria
    /// Example: Jeong Jeong - "When you next cast a Lesson spell this turn"
    SpellCast {
        /// Valid card types that trigger this (e.g., "Lesson", "Creature", "Noncreature")
        valid_card_type: Option<String>,
        /// Only trigger for spells cast by the trigger's controller
        you_only: bool,
    },
}

/// Which phase triggers the delayed trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerPhase {
    Upkeep,
    Draw,
    /// Pre-combat main phase (Main1).
    Main1,
    BeginCombat,
    EndCombat,
    /// Post-combat main phase (Main2).
    Main2,
    EndStep,
    Cleanup,
}

impl TriggerPhase {
    /// Parse a Forge card-script phase name (`Phase$ ...`) into a [`TriggerPhase`].
    ///
    /// Accepts the script spellings used in `cardsfolder` (`Main1`, `Main2`,
    /// `BeginCombat`, `EndCombat`, `End`, `Cleanup`, `Upkeep`, `Draw`).
    /// Returns `None` for unrecognized names so callers can skip the trigger
    /// rather than silently mapping to the wrong phase.
    pub fn from_script_name(name: &str) -> Option<Self> {
        match name {
            "Upkeep" => Some(Self::Upkeep),
            "Draw" => Some(Self::Draw),
            "Main1" => Some(Self::Main1),
            "BeginCombat" => Some(Self::BeginCombat),
            "EndCombat" => Some(Self::EndCombat),
            "Main2" => Some(Self::Main2),
            "End" | "EndStep" => Some(Self::EndStep),
            "Cleanup" => Some(Self::Cleanup),
            _ => None,
        }
    }
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

    /// Sacrifice a DIFFERENT card than the one the trigger watches.
    ///
    /// Used by Animate Dead: the trigger WATCHES the Aura leaving the
    /// battlefield (`tracked_card` = the Aura, `ZoneChange { from: Battlefield }`)
    /// but the effect SACRIFICES the reanimated creature
    /// (`target`) — "When Animate Dead leaves the battlefield, that creature's
    /// controller sacrifices it" (CR 603.6e leaves-the-battlefield + the
    /// reanimation Aura's drawback). Distinct from `Sacrifice`, which sacrifices
    /// the tracked card itself.
    SacrificeOther {
        /// The card to sacrifice when the trigger fires (the reanimated creature).
        target: CardId,
    },

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

    /// Copy a spell on the stack (triggered by SpellCast condition)
    /// Used by: Jeong Jeong - "copy it and you may choose new targets for the copy"
    ///
    /// This effect copies the spell that triggered the delayed trigger
    /// and puts the copy on the stack (potentially with new targets).
    CopySpellAbility {
        /// Whether the player may choose new targets for the copy
        may_choose_targets: bool,
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

    /// Reverse an `add`: remove the trigger and roll `next_id` back to the
    /// freed id, so a subsequent re-`add` (replay after rewind) assigns the
    /// SAME id the original `add` did. Keeping ids stable across rewind/replay
    /// is required for byte-identical state hashes (snapshot/resume, undo
    /// search). Only valid for the most-recently-added trigger.
    pub fn undo_add(&mut self, id: DelayedTriggerId) -> Option<DelayedTrigger> {
        let removed = self.remove(id);
        if removed.is_some() && id.as_u32() + 1 == self.next_id {
            self.next_id = id.as_u32();
        }
        removed
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

    /// Find triggers that fire at the beginning of `phase` on `active_player`'s turn.
    ///
    /// Used by the turn machinery to fire "at the beginning of your next
    /// [main] phase" delayed triggers (e.g. Mana Drain).
    pub fn find_phase_triggers(&self, phase: TriggerPhase, active_player: PlayerId) -> Vec<DelayedTriggerId> {
        self.triggers
            .iter()
            .filter(|t| t.matches_phase(phase, active_player))
            .map(|t| t.id)
            .collect()
    }

    /// Find triggers that match a spell cast event
    ///
    /// Returns IDs of delayed triggers that fire for this spell cast.
    /// Used by: Jeong Jeong and similar "when you next cast X" effects.
    pub fn get_matching_spellcast_trigger_ids(&self, caster: PlayerId, spell_types: &[&str]) -> Vec<DelayedTriggerId> {
        self.triggers
            .iter()
            .filter(|t| t.matches_spell_cast(caster, spell_types))
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
    fn test_undo_add_restores_next_id() {
        // mtg-519: rewind/replay determinism. undo_add must roll next_id back so
        // a replay re-add assigns the SAME DelayedTriggerId (stable state hash).
        let mut store = DelayedTriggerStore::new();
        let card = CardId::new(1);
        let id1 = store.add(make_test_trigger(card, smallvec::smallvec![Zone::Graveyard]));
        assert_eq!(id1, DelayedTriggerId::new(1));

        // Reverse the add (as undo of RegisterDelayedTrigger does).
        store.undo_add(id1);
        assert_eq!(store.len(), 0);

        // A re-add during replay must reuse id 1, not bump to 2.
        let id1_again = store.add(make_test_trigger(card, smallvec::smallvec![Zone::Graveyard]));
        assert_eq!(
            id1_again,
            DelayedTriggerId::new(1),
            "replay re-add must reuse the freed id"
        );
    }

    #[test]
    fn test_phase_trigger_matches_main_phases() {
        // mtg-519: Mana Drain's `Phase$ Main1,Main2 | ValidPlayer$ You`.
        let controller = crate::core::PlayerId::new(0);
        let opponent = crate::core::PlayerId::new(1);
        let trigger = DelayedTrigger::new(
            DelayedTriggerId::new(0),
            CardId::new(0),
            CardId::new(0),
            controller,
            DelayedTriggerCondition::Phase {
                phases: smallvec::smallvec![TriggerPhase::Main1, TriggerPhase::Main2],
                whose_turn: TurnOwner::You,
            },
            DelayedEffect::Sacrifice,
        );
        // Fires at the controller's main phases (either one)...
        assert!(trigger.matches_phase(TriggerPhase::Main1, controller));
        assert!(trigger.matches_phase(TriggerPhase::Main2, controller));
        // ...but NOT on the opponent's turn (ValidPlayer$ You)...
        assert!(!trigger.matches_phase(TriggerPhase::Main1, opponent));
        // ...nor at a non-main phase.
        assert!(!trigger.matches_phase(TriggerPhase::Upkeep, controller));
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
