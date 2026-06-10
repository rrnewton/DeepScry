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

    /// Parse a Forge card-script phase/step token into a [`Step`].
    ///
    /// Used by `ActivationPhases$ <start>-><end>` (Jade Statue's combat-only
    /// animate, Siren's Call's upkeep window, …). Forge writes these tokens
    /// without spaces (`BeginCombat`, `EndCombat`, `DeclareBlockers`) but some
    /// scripts use the spaced human form (`Declare Blockers`, `End of Turn`),
    /// so we normalise by stripping whitespace and matching case-insensitively.
    /// Returns `None` for an unrecognised token so the caller can warn rather
    /// than silently mis-restrict an ability.
    pub fn from_script_name(name: &str) -> Option<Step> {
        // Strip ASCII whitespace so "Declare Blockers" == "DeclareBlockers".
        let mut normalized = String::with_capacity(name.len());
        for ch in name.chars() {
            if !ch.is_ascii_whitespace() {
                normalized.push(ch.to_ascii_lowercase());
            }
        }
        Some(match normalized.as_str() {
            "untap" => Step::Untap,
            "upkeep" => Step::Upkeep,
            "draw" => Step::Draw,
            // Bare "Main" (Forge writes `ActivationPhases$ Main` / `Upkeep->Main`)
            // denotes the pre-combat main phase.
            "main" | "main1" | "premaincombat" | "precombatmain" => Step::Main1,
            "begincombat" | "beginningofcombat" => Step::BeginCombat,
            "declareattackers" => Step::DeclareAttackers,
            "declareblockers" => Step::DeclareBlockers,
            "combatdamage" => Step::CombatDamage,
            "endcombat" | "endofcombat" => Step::EndCombat,
            "main2" | "postcombatmain" | "postmaincombat" => Step::Main2,
            "end" | "endofturn" | "endstep" => Step::End,
            "cleanup" => Step::Cleanup,
            _ => return None,
        })
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
    // mtg-610: the per-turn WASM re-entry guard family (draw_step_executed_turn,
    // turn_state_reset_turn, blockers_declared_turn,
    // combat_first_strike_damage_dealt_turn, combat_first_strike_priority_done_turn,
    // combat_damage_dealt_turn, upkeep_triggers_checked_turn,
    // end_step_triggers_checked_turn, draw_triggers_checked_turn) was DELETED. Those
    // `#[serde(skip)]` fields existed solely to suppress double-application when the
    // WASM AI harness re-ran a step from the top after a NeedInput block without
    // rewinding. Both network paths (fancy_tui.rs run_network_mode_human_v2 /
    // run_network_ai_replay) now RESUME via undo-log rewind+replay: a re-entry
    // rewinds to the turn start and replays, so each once-per-turn step runs exactly
    // once from a clean state. The guards were dead and obscured the real
    // (rewind+replay) resume mechanism — see docs/NETWORK_ACTION_LOG.md / mtg-610.
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
