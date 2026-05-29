//! Turn phases and steps

use serde::{Deserialize, Serialize};

/// Major phases of a turn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Beginning,
    PreCombatMain,
    Combat,
    PostCombatMain,
    Ending,
}

/// Specific steps within phases
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Step {
    // Beginning Phase
    Untap,
    Upkeep,
    Draw,

    // Pre-Combat Main Phase
    Main1,

    // Combat Phase
    BeginCombat,
    DeclareAttackers,
    DeclareBlockers,
    CombatDamage,
    EndCombat,

    // Post-Combat Main Phase
    Main2,

    // Ending Phase
    End,
    Cleanup,
}

impl Step {
    /// Get a fixed platform-independent u32 discriminant for hashing.
    ///
    /// CRITICAL: Use this instead of `std::mem::discriminant` for network hash computation.
    /// `Discriminant<Step>` wraps `isize` internally (32-bit on WASM32, 64-bit on x86-64),
    /// causing `write_isize` to emit different byte counts, producing different hashes.
    /// This explicit match returns a fixed `u32` that is identical on all platforms.
    pub fn as_hash_u32(self) -> u32 {
        match self {
            Step::Untap => 0,
            Step::Upkeep => 1,
            Step::Draw => 2,
            Step::Main1 => 3,
            Step::BeginCombat => 4,
            Step::DeclareAttackers => 5,
            Step::DeclareBlockers => 6,
            Step::CombatDamage => 7,
            Step::EndCombat => 8,
            Step::Main2 => 9,
            Step::End => 10,
            Step::Cleanup => 11,
        }
    }

    /// Get the phase this step belongs to
    pub fn phase(&self) -> Phase {
        match self {
            Step::Untap | Step::Upkeep | Step::Draw => Phase::Beginning,
            Step::Main1 => Phase::PreCombatMain,
            Step::BeginCombat
            | Step::DeclareAttackers
            | Step::DeclareBlockers
            | Step::CombatDamage
            | Step::EndCombat => Phase::Combat,
            Step::Main2 => Phase::PostCombatMain,
            Step::End | Step::Cleanup => Phase::Ending,
        }
    }

    /// Get a short abbreviation for gamelog tagging
    ///
    /// Returns a 2-character abbreviation:
    /// - UK: Untap
    /// - UP: Upkeep
    /// - DR: Draw
    /// - M1: Main Phase 1
    /// - BC: Begin Combat
    /// - DA: Declare Attackers
    /// - DB: Declare Blockers
    /// - CD: Combat Damage
    /// - EC: End Combat
    /// - M2: Main Phase 2
    /// - ET: End Turn (End step)
    /// - CL: Cleanup
    pub fn abbreviation(&self) -> &'static str {
        match self {
            Step::Untap => "UK",
            Step::Upkeep => "UP",
            Step::Draw => "DR",
            Step::Main1 => "M1",
            Step::BeginCombat => "BC",
            Step::DeclareAttackers => "DA",
            Step::DeclareBlockers => "DB",
            Step::CombatDamage => "CD",
            Step::EndCombat => "EC",
            Step::Main2 => "M2",
            Step::End => "ET",
            Step::Cleanup => "CL",
        }
    }

    /// Get the next step in turn order
    pub fn next(&self) -> Option<Step> {
        match self {
            Step::Untap => Some(Step::Upkeep),
            Step::Upkeep => Some(Step::Draw),
            Step::Draw => Some(Step::Main1),
            Step::Main1 => Some(Step::BeginCombat),
            Step::BeginCombat => Some(Step::DeclareAttackers),
            Step::DeclareAttackers => Some(Step::DeclareBlockers),
            Step::DeclareBlockers => Some(Step::CombatDamage),
            Step::CombatDamage => Some(Step::EndCombat),
            Step::EndCombat => Some(Step::Main2),
            Step::Main2 => Some(Step::End),
            Step::End => Some(Step::Cleanup),
            Step::Cleanup => None, // End of turn
        }
    }

    /// Can a player play a sorcery in this step?
    pub fn is_sorcery_speed(&self) -> bool {
        matches!(self, Step::Main1 | Step::Main2)
    }

    /// Can a player play lands in this step?
    pub fn can_play_lands(&self) -> bool {
        matches!(self, Step::Main1 | Step::Main2)
    }
}

/// Represents the current turn structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStructure {
    /// Current turn number (starts at 1)
    ///
    /// INVARIANT: Turn numbers are 1-based (turn 1, 2, 3, ...), NEVER 0.
    /// This matches MTG rules where the first turn is "Turn 1".
    /// Any code that sees turn_number == 0 should treat it as a critical bug.
    pub turn_number: u32,

    /// Current step
    pub current_step: Step,

    /// Active player (whose turn it is)
    pub active_player: crate::core::PlayerId,

    /// Active player's index in GameState::players Vec (for O(1) next player lookup)
    pub active_player_idx: usize,

    /// Priority player (who currently has priority)
    pub priority_player: Option<crate::core::PlayerId>,

    /// Consecutive passes in current priority round (0, 1, or 2)
    /// When both players pass consecutively (reaches 2), the stack resolves or phase advances.
    /// This must persist across NeedInput returns for WASM non-blocking controllers.
    pub consecutive_passes: u8,

    /// Queue of extra turns to take (CR 500.7)
    /// Each entry is the player who gets the extra turn.
    /// When the current turn ends, if this queue is non-empty, the next turn
    /// is given to the player at the front of the queue instead of the normal rotation.
    /// Most recent AddTurn effects add to the back (FIFO order).
    #[serde(default)]
    pub extra_turns: Vec<crate::core::PlayerId>,
    // NOTE (mtg-j4128/mtg-610): nine #[serde(skip)] per-turn re-entry guard fields
    // (draw_step_executed_turn, turn_state_reset_turn, attackers_declared_turn,
    // blockers_declared_turn, combat_first_strike_damage_dealt_turn,
    // combat_first_strike_priority_done_turn, combat_damage_dealt_turn,
    // upkeep_triggers_checked_turn, end_step_triggers_checked_turn) used to live here.
    // They suppressed double-application when the WASM AI harness re-entered a step
    // after a NeedInput block WITHOUT rewinding. The harness now uses the shared
    // undo-log rewind/replay (rewind to turn start, replay forward, suppress only
    // external effects), which reverts then re-applies the step prologue — so these
    // WASM-specific guards are no longer needed and have been deleted.
}

impl TurnStructure {
    pub fn new(starting_player: crate::core::PlayerId) -> Self {
        TurnStructure {
            turn_number: 1,
            current_step: Step::Untap,
            active_player: starting_player,
            active_player_idx: 0, // Default to first player, should be set by GameState
            priority_player: None,
            consecutive_passes: 0,
            extra_turns: Vec::new(),
        }
    }

    pub fn new_with_idx(starting_player: crate::core::PlayerId, starting_idx: usize) -> Self {
        TurnStructure {
            turn_number: 1,
            current_step: Step::Untap,
            active_player: starting_player,
            active_player_idx: starting_idx,
            priority_player: None,
            consecutive_passes: 0,
            extra_turns: Vec::new(),
        }
    }

    pub fn current_phase(&self) -> Phase {
        self.current_step.phase()
    }

    /// Advance to the next step
    pub fn advance_step(&mut self) -> bool {
        if let Some(next_step) = self.current_step.next() {
            self.current_step = next_step;
            true
        } else {
            false // End of turn
        }
    }

    /// Start a new turn
    pub fn next_turn(&mut self, next_player: crate::core::PlayerId) {
        self.turn_number += 1;
        self.current_step = Step::Untap;
        self.active_player = next_player;
        self.priority_player = None;
        self.consecutive_passes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::PlayerId;

    #[test]
    fn test_step_phases() {
        assert_eq!(Step::Untap.phase(), Phase::Beginning);
        assert_eq!(Step::Main1.phase(), Phase::PreCombatMain);
        assert_eq!(Step::DeclareAttackers.phase(), Phase::Combat);
        assert_eq!(Step::Main2.phase(), Phase::PostCombatMain);
        assert_eq!(Step::Cleanup.phase(), Phase::Ending);
    }

    #[test]
    fn test_step_progression() {
        let mut step = Step::Untap;
        step = step.next().unwrap();
        assert_eq!(step, Step::Upkeep);
        step = step.next().unwrap();
        assert_eq!(step, Step::Draw);
    }

    #[test]
    fn test_turn_structure() {
        let player = PlayerId::new(1);
        let mut turn = TurnStructure::new(player);

        assert_eq!(turn.turn_number, 1);
        assert_eq!(turn.current_step, Step::Untap);
        assert_eq!(turn.active_player, player);

        assert!(turn.advance_step());
        assert_eq!(turn.current_step, Step::Upkeep);

        // Advance through entire turn
        while turn.advance_step() {}
        assert_eq!(turn.current_step, Step::Cleanup);

        let player2 = PlayerId::new(2);
        turn.next_turn(player2);
        assert_eq!(turn.turn_number, 2);
        assert_eq!(turn.current_step, Step::Untap);
        assert_eq!(turn.active_player, player2);
    }

    #[test]
    fn test_sorcery_speed() {
        assert!(Step::Main1.is_sorcery_speed());
        assert!(Step::Main2.is_sorcery_speed());
        assert!(!Step::Upkeep.is_sorcery_speed());
        assert!(!Step::DeclareAttackers.is_sorcery_speed());
    }
}
