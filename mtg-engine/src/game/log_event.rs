//! Structured game-log events — the log-of-record for puzzle assertions.
//!
//! # Design
//!
//! The game engine has always maintained a **string** log (human-readable
//! output). The puzzle assertion DSL (Phase 1) queries only *final state*
//! (life totals, zone contents, turn count). Phase 2 needs **event-level**
//! assertions: "trigger X fired", "creature Y died", "spell Z was cast".
//! Querying the string buffer for these would violate the no-hacky-string-
//! operations rule and would be fragile.
//!
//! This module provides a **structured** parallel log: an `Vec<LogEvent>` that
//! records exactly the game events we need for assertions. It lives alongside
//! the string buffer in [`GameLogger`] and is populated by the same call sites.
//!
//! ## Zero-overhead disable
//!
//! When event-logging is off ([`GameLogger::enable_event_log`] not called), the
//! `event_log` vec is empty and every `push_event` call is a cheap `is_empty`
//! guard — no allocation. The game uses `VerbosityLevel::Silent` for MCTS and
//! fuzz runs; those paths also set `enable_event_log = false` by default.
//!
//! ## Rewind / determinism
//!
//! The `event_log` is NOT serialized (it mirrors the string `log_buffer` in
//! this regard). The `GameLogger` custom `Serialize` impl skips it. On rewind
//! the event log is truncated in the same way as the string buffer: callers
//! store the `prior_event_log_size` alongside `prior_log_size` in the undo log
//! and call [`GameLogger::truncate_events_to`] on undo.
//!
//! ## Unboxed flat enum
//!
//! Follows the `UndoLog` / `GameAction` pattern exactly: flat enum variants,
//! no `Box`, no per-entry heap allocation beyond owned `String` fields.

use crate::core::{CardId, PlayerId};
use crate::game::phase::Step;

/// A structured game event recorded for assertion queries.
///
/// This is the log-of-record for event-level puzzle assertions (Phase 2).
/// It is populated by the same call sites that emit string log entries,
/// but is separate from the string buffer and lives only in memory.
///
/// **Flat enum, no Box.** Follows the UndoLog/GameAction pattern:
/// every variant is `Copy`-capable or uses owned `String`s but no heap boxes.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// A spell was cast (moved onto the stack).
    SpellCast {
        /// The card being cast.
        card_id: CardId,
        /// Name at cast time (owned so it's available after zone changes).
        card_name: String,
        /// The player casting the spell.
        caster: PlayerId,
    },

    /// A triggered ability fired (was placed on the stack from a trigger).
    TriggerFired {
        /// The permanent that carries the trigger.
        source_id: CardId,
        /// Source permanent's name at firing time.
        source_name: String,
        /// The controller of the trigger source.
        controller: PlayerId,
        /// Human-readable description of what triggered (e.g. "Creature dies").
        description: String,
    },

    /// A creature died (moved from battlefield to graveyard via state-based
    /// effect or effect resolving).
    CreatureDied {
        card_id: CardId,
        card_name: String,
        /// Controller at the moment of death.
        controller: PlayerId,
    },

    /// A card changed zones.
    ZoneChange {
        card_id: CardId,
        card_name: String,
        owner: PlayerId,
        from: ZoneTag,
        to: ZoneTag,
    },

    /// Damage was dealt (to player or permanent).
    DamageDealt {
        /// The source dealing damage.
        source_id: CardId,
        source_name: String,
        /// The amount of damage.
        amount: i32,
        /// The target.
        target: DamageTarget,
    },

    /// A player's life total changed.
    LifeChanged {
        player: PlayerId,
        delta: i32,
        new_total: i32,
    },

    /// A new turn started.
    TurnStarted { turn_number: u32, active_player: PlayerId },

    /// A new step/phase started within a turn.
    StepStarted { step: Step, active_player: PlayerId },
}

/// Zone identifier for `ZoneChange` events.
///
/// Mirrors `crate::zones::Zone` but without needing to import the zones crate
/// from this module. This is a display-layer view, not the full zone type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneTag {
    Library,
    Hand,
    Battlefield,
    Graveyard,
    Stack,
    Exile,
    Command,
    Unknown,
}

impl ZoneTag {
    pub fn name(self) -> &'static str {
        match self {
            ZoneTag::Library => "library",
            ZoneTag::Hand => "hand",
            ZoneTag::Battlefield => "battlefield",
            ZoneTag::Graveyard => "graveyard",
            ZoneTag::Stack => "stack",
            ZoneTag::Exile => "exile",
            ZoneTag::Command => "command",
            ZoneTag::Unknown => "unknown",
        }
    }
}

impl From<crate::zones::Zone> for ZoneTag {
    fn from(z: crate::zones::Zone) -> Self {
        use crate::zones::Zone;
        match z {
            Zone::Library => ZoneTag::Library,
            Zone::Hand => ZoneTag::Hand,
            Zone::Battlefield => ZoneTag::Battlefield,
            Zone::Graveyard => ZoneTag::Graveyard,
            Zone::Stack => ZoneTag::Stack,
            Zone::Exile => ZoneTag::Exile,
            Zone::Command => ZoneTag::Command,
        }
    }
}

/// Target of a damage event.
#[derive(Debug, Clone)]
pub enum DamageTarget {
    Player { player_id: PlayerId, life_after: i32 },
    Permanent { card_id: CardId, card_name: String },
}

/// A read-only query view over the event log.
///
/// Callers get this by calling [`GameLogger::events()`]. It borrows the
/// logger's event_log `Vec` without copying.
pub struct EventLogView<'a> {
    pub(crate) events: &'a [LogEvent],
}

impl<'a> EventLogView<'a> {
    /// Iterate over all recorded events.
    pub fn iter(&self) -> std::slice::Iter<'_, LogEvent> {
        self.events.iter()
    }

    /// Number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Is the log empty?
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Did any trigger fire from the named source?
    pub fn any_trigger_fired_from(&self, source_name: &str) -> bool {
        self.events.iter().any(|e| {
            if let LogEvent::TriggerFired { source_name: n, .. } = e {
                n.eq_ignore_ascii_case(source_name)
            } else {
                false
            }
        })
    }

    /// Did a creature with the given name die?
    pub fn any_creature_died_named(&self, name: &str) -> bool {
        self.events.iter().any(|e| {
            if let LogEvent::CreatureDied { card_name: n, .. } = e {
                n.eq_ignore_ascii_case(name)
            } else {
                false
            }
        })
    }

    /// Was a spell with the given name cast?
    pub fn any_spell_cast_named(&self, name: &str) -> bool {
        self.events.iter().any(|e| {
            if let LogEvent::SpellCast { card_name: n, .. } = e {
                n.eq_ignore_ascii_case(name)
            } else {
                false
            }
        })
    }
}
