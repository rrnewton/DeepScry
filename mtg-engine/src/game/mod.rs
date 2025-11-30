//! Core game state and turn structure

pub mod actions;
pub mod combat;
pub mod continuous_effects;
pub mod controller;
pub mod fancy_fixed_controller;
pub mod fancy_tui_controller;
pub mod fixed_script_controller;
pub mod game_loop;
pub mod game_state_evaluator;
pub mod hand_setup;
pub mod heuristic_controller;
pub mod interactive_controller;
pub mod logger;
pub mod mana_colors;
pub mod mana_engine;
pub mod mana_index;
pub mod mana_payment;
pub mod mana_source_cache;
pub mod phase;
pub mod random_controller;
pub mod replay_controller;
pub mod rich_input_controller;
pub mod snapshot;
pub mod state;
pub mod state_hash;
pub mod stop_condition;
pub mod zero_controller;

#[cfg(test)]
mod controller_tests;
#[cfg(test)]
mod counter_tests;
#[cfg(test)]
mod test_spider_suit;

pub use combat::CombatState;
pub use continuous_effects::PTBreakdown;
pub use controller::{format_choice_menu, GameStateView, PlayerController};
pub use fancy_fixed_controller::FancyFixedController;
pub use fancy_tui_controller::FancyTuiController;
pub use fixed_script_controller::FixedScriptController;
pub use game_loop::{GameEndReason, GameLoop, GameResult, VerbosityLevel};
pub use game_state_evaluator::{GameStateEvaluator, Score};
pub use hand_setup::{setup_opening_hands, HandSetup};
pub use heuristic_controller::HeuristicController;
pub use interactive_controller::InteractiveController;
pub use logger::{GameLogger, LogEntry, OutputFormat, OutputMode};
pub use mana_colors::ManaColors;
pub use mana_engine::{ManaCapacity, ManaEngine};
pub use mana_index::{ManaColorBucket, ManaProducerBucket, ManaProducerIndex};
pub use mana_payment::{GreedyManaResolver, ManaPaymentResolver, ManaSource, PaymentResult, SimpleManaResolver};
pub use mana_source_cache::ManaSourceCache;

// Re-export mana production types from core for convenience
pub use crate::core::{ManaColor, ManaProduction, ManaProductionKind};
pub use phase::{Phase, Step, TurnStructure};
pub use random_controller::RandomController;
pub use replay_controller::{ReplayChoice, ReplayController};
pub use rich_input_controller::RichInputController;
pub use snapshot::{ControllerState, ControllerType, GameSnapshot, SnapshotError};
pub use state::GameState;
pub use state_hash::{compute_state_hash, compute_undo_test_hash, format_hash};
pub use stop_condition::{StopCondition, StopPlayer};
pub use zero_controller::ZeroController;
