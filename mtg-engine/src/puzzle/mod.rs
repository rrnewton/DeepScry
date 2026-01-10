//! Puzzle file format support
//!
//! This module provides parsing and loading for .pzl (puzzle) files, which allow
//! creating specific mid-game states for testing and puzzle scenarios.
//!
//! See docs/PZL_FORMAT_ANALYSIS.md for detailed format documentation.

pub mod card_notation;
pub mod format;
#[cfg(feature = "native")]
pub mod loader;
pub mod metadata;
pub mod state;

pub use card_notation::CardModifier;
pub use format::PuzzleFile;
#[cfg(feature = "native")]
pub use loader::load_puzzle_into_game;
pub use metadata::{Difficulty, GoalType, PuzzleMetadata};
pub use state::{CardDefinition, GameStateDefinition, PlayerStateDefinition};

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
}
