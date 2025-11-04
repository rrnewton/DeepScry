//! Undo log for efficient game tree search
//!
//! This module provides a transaction log of game actions that can be
//! rewound to efficiently explore the game tree without expensive deep copies.

use crate::core::{CardId, CounterType, PlayerId};
use crate::zones::Zone;
use serde::{Deserialize, Serialize};

use crate::game::GameState;

/// Atomic game actions that can be logged and undone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameAction {
    /// Move a card between zones
    MoveCard {
        card_id: CardId,
        from_zone: Zone,
        to_zone: Zone,
        owner: PlayerId,
    },

    /// Tap/untap a permanent
    TapCard { card_id: CardId, tapped: bool },

    /// Modify life total (delta is the change, not absolute value)
    ModifyLife { player_id: PlayerId, delta: i32 },

    /// Add mana to pool
    AddMana {
        player_id: PlayerId,
        mana: crate::core::ManaCost,
    },

    /// Empty mana pool (stores previous state for undo)
    EmptyManaPool {
        player_id: PlayerId,
        prev_white: u8,
        prev_blue: u8,
        prev_black: u8,
        prev_red: u8,
        prev_green: u8,
        prev_colorless: u8,
    },

    /// Add counter to card
    AddCounter {
        card_id: CardId,
        counter_type: CounterType,
        amount: u8,
    },

    /// Remove counter from card
    RemoveCounter {
        card_id: CardId,
        counter_type: CounterType,
        amount: u8,
    },

    /// Advance game step
    AdvanceStep {
        from_step: crate::game::Step,
        to_step: crate::game::Step,
    },

    /// Change turn (stores RNG state for proper rewind)
    ChangeTurn {
        from_player: PlayerId,
        to_player: PlayerId,
        turn_number: u32,
        /// RNG state at the START of this turn (for snapshot rewind)
        rng_state: Option<Vec<u8>>,
    },

    /// Pump creature (temporary stat modification)
    PumpCreature {
        card_id: CardId,
        power_delta: i32,
        toughness_delta: i32,
    },

    /// Mark a choice point (for tree search and replay)
    ///
    /// Stores both the fact that a choice occurred and what that choice was,
    /// enabling deterministic replay from snapshots.
    ChoicePoint {
        player_id: PlayerId,
        choice_id: u32,
        /// The actual choice made (for replay). None if choice hasn't been recorded yet.
        choice: Option<crate::game::replay_controller::ReplayChoice>,
    },
}

impl GameAction {
    /// Apply the inverse of this action to undo it
    ///
    /// Returns Ok(()) if successful, Err if the action cannot be undone
    pub fn undo(&self, game: &mut GameState) -> Result<(), String> {
        match self {
            GameAction::MoveCard {
                card_id,
                from_zone,
                to_zone,
                owner,
            } => {
                // Reverse the move: move from to_zone back to from_zone
                game.move_card(*card_id, *to_zone, *from_zone, *owner)
                    .map_err(|e| format!("Failed to undo MoveCard: {}", e))?;
            }

            GameAction::TapCard { card_id, tapped } => {
                // Reverse tap state
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.tapped = !tapped;
                } else {
                    return Err(format!("Card {} not found for TapCard undo", card_id.as_u32()));
                }
            }

            GameAction::ModifyLife { player_id, delta } => {
                // Reverse the life change
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.life -= delta;
                } else {
                    return Err(format!("Player {} not found for ModifyLife undo", player_id.as_u32()));
                }
            }

            GameAction::AddMana { player_id, mana } => {
                // Remove the mana that was added
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool.white = player.mana_pool.white.saturating_sub(mana.white);
                    player.mana_pool.blue = player.mana_pool.blue.saturating_sub(mana.blue);
                    player.mana_pool.black = player.mana_pool.black.saturating_sub(mana.black);
                    player.mana_pool.red = player.mana_pool.red.saturating_sub(mana.red);
                    player.mana_pool.green = player.mana_pool.green.saturating_sub(mana.green);
                    player.mana_pool.colorless = player.mana_pool.colorless.saturating_sub(mana.colorless);
                } else {
                    return Err(format!("Player {} not found for AddMana undo", player_id.as_u32()));
                }
            }

            GameAction::EmptyManaPool {
                player_id,
                prev_white,
                prev_blue,
                prev_black,
                prev_red,
                prev_green,
                prev_colorless,
            } => {
                // Restore previous mana pool state
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool.white = *prev_white;
                    player.mana_pool.blue = *prev_blue;
                    player.mana_pool.black = *prev_black;
                    player.mana_pool.red = *prev_red;
                    player.mana_pool.green = *prev_green;
                    player.mana_pool.colorless = *prev_colorless;
                } else {
                    return Err(format!(
                        "Player {} not found for EmptyManaPool undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::AddCounter {
                card_id,
                counter_type,
                amount,
            } => {
                // Remove the counters that were added
                game.remove_counters(*card_id, *counter_type, *amount)
                    .map_err(|e| format!("Failed to undo AddCounter: {}", e))?;
            }

            GameAction::RemoveCounter {
                card_id,
                counter_type,
                amount,
            } => {
                // Add back the counters that were removed
                game.add_counters(*card_id, *counter_type, *amount)
                    .map_err(|e| format!("Failed to undo RemoveCounter: {}", e))?;
            }

            GameAction::AdvanceStep { from_step, to_step: _ } => {
                // Restore previous step
                game.turn.current_step = *from_step;
            }

            GameAction::ChangeTurn {
                from_player,
                to_player: _,
                turn_number,
                rng_state,
            } => {
                // Restore previous turn state
                game.turn.active_player = *from_player;
                // Find the player index
                if let Some(idx) = game.players.iter().position(|p| p.id == *from_player) {
                    game.turn.active_player_idx = idx;
                }

                // Restore turn number to the previous turn
                // ChangeTurn logs the NEW turn number, so previous is turn_number - 1
                game.turn.turn_number = turn_number.saturating_sub(1);

                // Restore RNG state if available
                if let Some(rng_bytes) = rng_state {
                    use serde::Deserialize;
                    let mut deserializer = serde_json::Deserializer::from_slice(rng_bytes);
                    if let Ok(rng) = rand_chacha::ChaCha12Rng::deserialize(&mut deserializer) {
                        *game.rng.borrow_mut() = rng;
                    } else {
                        return Err("Failed to deserialize RNG state".to_string());
                    }
                }
            }

            GameAction::PumpCreature {
                card_id,
                power_delta,
                toughness_delta,
            } => {
                // Reverse the pump by applying negative deltas
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    // Handle Option<i8> by mapping to subtract the delta
                    card.power = card.power.map(|p| p.saturating_sub(*power_delta as i8));
                    card.toughness = card.toughness.map(|t| t.saturating_sub(*toughness_delta as i8));
                } else {
                    return Err(format!("Card {} not found for PumpCreature undo", card_id.as_u32()));
                }
            }

            GameAction::ChoicePoint { .. } => {
                // ChoicePoints don't modify game state, nothing to undo
            }
        }

        Ok(())
    }
}

/// Undo log for tracking and rewinding game actions
///
/// This allows efficient tree search by mutating game state forward
/// and then rewinding via the log, instead of expensive deep copies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoLog {
    /// Stack of actions (most recent at end)
    actions: Vec<GameAction>,

    /// Is logging enabled? (can be compiled out for replay benchmarks)
    enabled: bool,

    /// Mark positions for choice points
    choice_points: Vec<usize>,
}

impl UndoLog {
    pub fn new() -> Self {
        UndoLog {
            actions: Vec::new(),
            enabled: true,
            choice_points: Vec::new(),
        }
    }

    /// Create a disabled undo log (for benchmarking)
    pub fn disabled() -> Self {
        UndoLog {
            actions: Vec::new(),
            enabled: false,
            choice_points: Vec::new(),
        }
    }

    /// Log an action
    pub fn log(&mut self, action: GameAction) {
        if self.enabled {
            self.actions.push(action);
        }
    }

    /// Mark a choice point in the log
    pub fn mark_choice_point(&mut self) {
        if self.enabled {
            self.choice_points.push(self.actions.len());
        }
    }

    /// Get the most recent action without removing it
    pub fn peek(&self) -> Option<&GameAction> {
        self.actions.last()
    }

    /// Pop and return the most recent action
    pub fn pop(&mut self) -> Option<GameAction> {
        self.actions.pop()
    }

    /// Get number of actions in log
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Clear all actions up to the most recent choice point
    pub fn rewind_to_choice_point(&mut self) {
        if let Some(checkpoint) = self.choice_points.pop() {
            self.actions.truncate(checkpoint);
        }
    }

    /// Rewind to the most recent ChangeTurn action, extracting all ChoicePoint actions
    /// encountered along the way (in forward chronological order).
    ///
    /// This method actually UNDOES the game state by applying the inverse of each action.
    ///
    /// Returns (turn_number, intra_turn_choices, actions_rewound) where:
    /// - turn_number: The turn number from the most recent ChangeTurn action
    /// - intra_turn_choices: All ChoicePoint actions that occurred after that turn change
    /// - actions_rewound: Total number of actions popped from the log
    ///
    /// Returns None if no ChangeTurn action is found in the log.
    pub fn rewind_to_turn_start(&mut self, game: &mut GameState) -> Option<(u32, Vec<GameAction>, usize)> {
        if !self.enabled {
            return None;
        }

        let mut choices_reversed = Vec::new();
        let mut turn_number = None;
        let mut actions_rewound = 0;

        // Pop actions in reverse until we find ChangeTurn
        while let Some(action) = self.pop() {
            actions_rewound += 1;
            match action {
                GameAction::ChangeTurn { turn_number: tn, .. } => {
                    // DON'T undo the ChangeTurn action - we want the snapshot to represent
                    // the START of this turn, not the END of the previous turn.
                    // Put it back on the log so the game state stays at the turn boundary.
                    self.actions.push(action);
                    actions_rewound -= 1; // Don't count this as rewound since we kept it
                    turn_number = Some(tn);
                    break;
                }
                GameAction::ChoicePoint { .. } => {
                    // Collect choice points in reverse (don't need to undo, they're non-mutating)
                    choices_reversed.push(action);
                }
                _ => {
                    // Undo all other actions to restore game state
                    if let Err(e) = action.undo(game) {
                        eprintln!("WARNING: Failed to undo action {:?}: {}", action, e);
                    }
                }
            }
        }

        turn_number.map(|tn| {
            // Reverse the choices to get forward chronological order
            choices_reversed.reverse();
            (tn, choices_reversed, actions_rewound)
        })
    }

    /// Get the most recent turn number from the log, if any ChangeTurn exists
    pub fn current_turn(&self) -> Option<u32> {
        self.actions.iter().rev().find_map(|action| {
            if let GameAction::ChangeTurn { turn_number, .. } = action {
                Some(*turn_number)
            } else {
                None
            }
        })
    }

    /// Clear the entire log
    pub fn clear(&mut self) {
        self.actions.clear();
        self.choice_points.clear();
    }

    /// Get all actions (for debugging/serialization)
    pub fn actions(&self) -> &[GameAction] {
        &self.actions
    }
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_log() {
        let mut log = UndoLog::new();
        assert_eq!(log.len(), 0);

        let action = GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -3,
        };

        log.log(action.clone());
        assert_eq!(log.len(), 1);

        let popped = log.pop().unwrap();
        assert!(matches!(popped, GameAction::ModifyLife { .. }));
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn test_choice_points() {
        let mut log = UndoLog::new();

        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });
        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });

        log.mark_choice_point();

        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });
        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });

        assert_eq!(log.len(), 4);

        log.rewind_to_choice_point();
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn test_disabled_log() {
        let mut log = UndoLog::disabled();

        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });

        assert_eq!(log.len(), 0); // Nothing logged when disabled
    }

    #[test]
    fn test_rewind_to_turn_start() {
        let mut log = UndoLog::new();
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Simulate turn 1 starting
        log.log(GameAction::ChangeTurn {
            from_player: PlayerId::new(0),
            to_player: PlayerId::new(1),
            turn_number: 1,
            rng_state: None,
        });

        // Some actions during turn 1
        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });

        log.log(GameAction::ChoicePoint {
            player_id: PlayerId::new(1),
            choice_id: 1,
            choice: None,
        });

        log.log(GameAction::TapCard {
            card_id: CardId::new(1),
            tapped: true,
        });

        log.log(GameAction::ChoicePoint {
            player_id: PlayerId::new(1),
            choice_id: 2,
            choice: None,
        });

        assert_eq!(log.len(), 5);

        // Rewind to turn start (now requires GameState)
        let result = log.rewind_to_turn_start(&mut game);
        assert!(result.is_some());

        let (turn_number, choices, actions_rewound) = result.unwrap();
        assert_eq!(turn_number, 1);
        assert_eq!(choices.len(), 2);
        assert_eq!(actions_rewound, 4); // All 4 actions after ChangeTurn (ChangeTurn is kept)

        // Verify choices are in forward chronological order
        assert!(matches!(
            choices[0],
            GameAction::ChoicePoint {
                player_id: _,
                choice_id: 1,
                choice: None
            }
        ));
        assert!(matches!(
            choices[1],
            GameAction::ChoicePoint {
                player_id: _,
                choice_id: 2,
                choice: None
            }
        ));

        // Log should have the ChangeTurn action still (we stopped AT the turn boundary)
        assert_eq!(log.len(), 1);
        assert!(matches!(log.peek().unwrap(), GameAction::ChangeTurn { .. }));
    }

    #[test]
    fn test_rewind_to_turn_start_no_turn() {
        let mut log = UndoLog::new();
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Add some actions but no ChangeTurn
        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });

        log.log(GameAction::ChoicePoint {
            player_id: PlayerId::new(1),
            choice_id: 1,
            choice: None,
        });

        let result = log.rewind_to_turn_start(&mut game);
        assert!(result.is_none());
    }

    #[test]
    fn test_current_turn() {
        let mut log = UndoLog::new();

        assert_eq!(log.current_turn(), None);

        log.log(GameAction::ChangeTurn {
            from_player: PlayerId::new(0),
            to_player: PlayerId::new(1),
            turn_number: 1,
            rng_state: None,
        });

        assert_eq!(log.current_turn(), Some(1));

        log.log(GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -1,
        });

        log.log(GameAction::ChangeTurn {
            from_player: PlayerId::new(1),
            to_player: PlayerId::new(0),
            turn_number: 2,
            rng_state: None,
        });

        // Should return the most recent turn
        assert_eq!(log.current_turn(), Some(2));
    }
}
