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

    /// Tracks which turn the mandatory Draw step card draw was executed.
    /// Prevents double-draw when WASM harness re-creates GameLoop mid-step (the harness creates
    /// a new GameLoop on each step_harness() call; if priority_round blocks with NeedInput,
    /// current_step stays at Draw, and the next call would re-execute draw_card()).
    /// Auto-invalidates when turn_number changes (Some(old_turn) != Some(current_turn)).
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub draw_step_executed_turn: Option<u32>,

    /// Tracks which turn reset_turn_state() has already been called for.
    /// Prevents reset_turn_state from running multiple times per turn when the WASM harness
    /// re-creates GameLoop on each step_harness() call. Re-running reset_turn_state mid-turn
    /// would reset lands_played_this_turn to 0, letting the local WASM client offer PlayLand
    /// again while the server correctly denies it, causing DESYNC in available action lists.
    /// Auto-invalidates when turn_number changes (Some(old_turn) != Some(current_turn)).
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub turn_state_reset_turn: Option<u32>,

    /// Tracks which turn the DeclareAttackers choice has already been made for.
    /// Prevents re-asking the active player for attacker choices when the game loop resumes
    /// after NeedInput (from the subsequent priority_round). Without this flag, when the
    /// active player chose no attackers (no creatures tapped), re-entering declare_attackers_step
    /// would find available attackers again and consume the wrong opponent choice message.
    /// Auto-invalidates when turn_number changes.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub attackers_declared_turn: Option<u32>,

    /// Tracks which turn the DeclareBlockers choice has already been made for.
    /// Prevents re-asking the defending player for blocker choices when the game loop resumes
    /// after NeedInput (from the subsequent priority_round). Without this flag, re-entering
    /// declare_blockers_step would consume the wrong ChoiceRequest from the server queue,
    /// causing the WASM shadow state's action_count to fall 1 behind the server.
    /// Auto-invalidates when turn_number changes.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub blockers_declared_turn: Option<u32>,

    /// Tracks which turn first-strike combat damage has already been dealt.
    /// Prevents re-dealing first-strike damage when the game loop resumes after NeedInput
    /// from the subsequent priority_round. The WASM harness recreates GameLoop on every
    /// step_harness() call, so without this guard, first-strike damage would be applied
    /// multiple times across calls.
    /// Auto-invalidates when turn_number changes.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub combat_first_strike_damage_dealt_turn: Option<u32>,

    /// Tracks which turn the priority round AFTER first-strike damage has completed.
    /// Required because has_first_strike_combat() may return false on re-entry (if
    /// first-strike creatures died), which would cause the first-strike priority_round
    /// to be skipped entirely on subsequent step_harness() calls.
    /// Auto-invalidates when turn_number changes.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub combat_first_strike_priority_done_turn: Option<u32>,

    /// Tracks which turn normal combat damage has already been dealt.
    /// Prevents re-dealing combat damage when the game loop resumes after NeedInput
    /// from the subsequent priority_round. The WASM harness recreates GameLoop on every
    /// step_harness() call, causing double damage and action_count divergence without
    /// this guard.
    /// Auto-invalidates when turn_number changes.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub combat_damage_dealt_turn: Option<u32>,

    /// Tracks which turn the beginning-of-upkeep phase triggers were already fired.
    /// Prevents re-firing the upkeep triggers when the WASM harness recreates the
    /// GameLoop after the upkeep `priority_round` blocks with NeedInput: current_step
    /// stays at Upkeep, so the next step_harness() call would re-enter upkeep_step and
    /// call check_phase_triggers(BeginningOfUpkeep) a SECOND time. For a trigger that
    /// mutates state once per upkeep (e.g. All Hallow's Eve removing one scream counter
    /// and, at zero, mass-resurrecting), double-firing diverges the WASM shadow from
    /// the server (mtg-609). Auto-invalidates when turn_number changes.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub upkeep_triggers_checked_turn: Option<u32>,

    /// Tracks which turn the beginning-of-end-step phase triggers were already fired.
    /// Same WASM re-entry hazard as `upkeep_triggers_checked_turn`: end_step calls
    /// check_phase_triggers(BeginningOfEndStep) then a blocking priority_round, so
    /// re-entry would double-fire end-step triggers. Auto-invalidates per turn.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub end_step_triggers_checked_turn: Option<u32>,

    /// Tracks which turn the beginning-of-draw-step phase triggers were already
    /// fired. Same WASM re-entry hazard as `upkeep_triggers_checked_turn`:
    /// draw_step fires check_phase_triggers(BeginningOfDraw) then a blocking
    /// priority_round, so re-entry would double-fire draw-step triggers (e.g.
    /// Grafted Skullcap drawing an extra card twice). Auto-invalidates per turn.
    /// Not serialized - this is transient within a single game session.
    #[serde(skip)]
    pub draw_triggers_checked_turn: Option<u32>,

    /// Tracks which turn the Main1 `Mode$ Phase` delayed triggers (e.g. Mana
    /// Drain's deferred mana) already fired, so a WASM re-entry of main_phase
    /// (which fires then blocks on a priority_round) does not double-fire.
    /// Same `#[serde(skip)]` + auto-invalidate-per-turn pattern as the
    /// begin-of-phase trigger guards above. The delayed-trigger lifecycle
    /// itself is fully undo-logged (RegisterDelayedTrigger / FireDelayedTrigger
    /// / SetRememberedAmount + AddMana), so snapshot/resume + rewind/replay
    /// reverse it correctly without this guard being serialized; it is reset by
    /// reset_transient_guards on rewind-to-turn-start.
    #[serde(skip)]
    pub main1_delayed_fired_turn: Option<u32>,

    /// Like `main1_delayed_fired_turn` but for the post-combat main phase.
    #[serde(skip)]
    pub main2_delayed_fired_turn: Option<u32>,
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
            draw_step_executed_turn: None,
            turn_state_reset_turn: None,
            attackers_declared_turn: None,
            blockers_declared_turn: None,
            combat_first_strike_damage_dealt_turn: None,
            combat_first_strike_priority_done_turn: None,
            combat_damage_dealt_turn: None,
            upkeep_triggers_checked_turn: None,
            end_step_triggers_checked_turn: None,
            draw_triggers_checked_turn: None,
            main1_delayed_fired_turn: None,
            main2_delayed_fired_turn: None,
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
            draw_step_executed_turn: None,
            turn_state_reset_turn: None,
            attackers_declared_turn: None,
            blockers_declared_turn: None,
            combat_first_strike_damage_dealt_turn: None,
            combat_first_strike_priority_done_turn: None,
            combat_damage_dealt_turn: None,
            upkeep_triggers_checked_turn: None,
            end_step_triggers_checked_turn: None,
            draw_triggers_checked_turn: None,
            main1_delayed_fired_turn: None,
            main2_delayed_fired_turn: None,
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

    /// Reset transient guard fields to None.
    ///
    /// These fields are `#[serde(skip)]` and not in the undo log, so they persist
    /// their end-of-game values after a full rewind. Call this when the game has been
    /// completely rewound to its initial state (undo log empty) so that the next
    /// play-through starts with clean guard state.
    pub fn reset_transient_guards(&mut self) {
        self.draw_step_executed_turn = None;
        self.turn_state_reset_turn = None;
        self.attackers_declared_turn = None;
        self.blockers_declared_turn = None;
        self.combat_first_strike_damage_dealt_turn = None;
        self.combat_first_strike_priority_done_turn = None;
        self.combat_damage_dealt_turn = None;
        self.upkeep_triggers_checked_turn = None;
        self.end_step_triggers_checked_turn = None;
        self.draw_triggers_checked_turn = None;
        // Serialized, but a full rewind to the initial state must also clear
        // these so a same-session replay re-fires the main-phase delayed
        // triggers (mirrors the draw_step_executed_turn rationale above).
        self.main1_delayed_fired_turn = None;
        self.main2_delayed_fired_turn = None;
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
