//! Puzzle file format support
//!
//! This module provides parsing and loading for .pzl (puzzle) files, which allow
//! creating specific mid-game states for testing and puzzle scenarios.
//!
//! See docs/PZL_FORMAT_ANALYSIS.md for detailed format documentation.
//!
//! ## Inline assertions (`puzzle-assert` feature)
//!
//! When built with `--features puzzle-assert`, `.pzl` files can include an
//! optional `[assertions]` section whose lines describe expected outcomes.
//! The assertions are evaluated after a puzzle run and produce a pass/fail
//! report.  The engine hot-path is never affected: all assertion code
//! compiles out when the feature is off.
//!
//! See `ai_docs/reference/PUZZLE_ASSERTION_DSL.md` for the full specification
//! and tracking issue mtg-935.

pub mod card_notation;
pub mod format;
#[cfg(feature = "native")]
pub mod loader;
pub mod metadata;
pub mod state;

#[cfg(feature = "puzzle-assert")]
pub mod assert;

pub use card_notation::CardModifier;
pub use format::PuzzleFile;
#[cfg(feature = "native")]
pub use loader::load_puzzle_into_game;
pub use metadata::{Difficulty, GoalType, PuzzleMetadata};
pub use state::{CardDefinition, GameStateDefinition, PlayerStateDefinition};

#[cfg(feature = "puzzle-assert")]
pub use assert::{evaluate_assertions, parse_assertions, Assertion, AssertionReport};

use crate::Result;

impl PuzzleFile {
    /// Load a puzzle file from disk (native only - requires filesystem access)
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    #[cfg(feature = "native")]
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::parse(&contents)
    }

    /// Parse a puzzle file from a string
    ///
    /// # Errors
    ///
    /// Returns an error if the puzzle format is invalid or missing required sections.
    pub fn parse(contents: &str) -> Result<Self> {
        format::parse_puzzle(contents)
    }

    /// Build the [`PlayerController`](crate::game::PlayerController) for one
    /// player of this puzzle.
    ///
    /// This is the single, shared wiring point used by every puzzle runner
    /// (the bulk runner and the e2e harness) so script handling stays DRY:
    ///
    /// * If the puzzle declared a `[p0_script]` / `[p1_script]` section for
    ///   `player_idx`, the player is driven by a
    ///   [`RichInputController`](crate::game::RichInputController) replaying the
    ///   scripted semantic commands (`cast`, `target`, `activate`, `attack`,
    ///   `PASS_UNTIL`, …). The script is deterministic and information-
    ///   independent — it uses only the same public commands a human types — so
    ///   it produces identical decisions on server / client / native / WASM.
    /// * Otherwise the player keeps the default
    ///   [`HeuristicController`](crate::game::HeuristicController), exactly as
    ///   before the action-script capability existed (zero behavioural change
    ///   for the ~all unscripted puzzles).
    #[cfg(feature = "native")]
    pub fn build_controller(
        &self,
        player_idx: usize,
        player_id: crate::core::PlayerId,
    ) -> Box<dyn crate::game::PlayerController> {
        match self.script_for(player_idx) {
            Some(script) => Box::new(crate::game::RichInputController::new(player_id, script.to_vec())),
            None => Box::new(crate::game::HeuristicController::new(player_id)),
        }
    }
}
