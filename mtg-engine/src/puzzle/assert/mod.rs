//! Puzzle inline assertion DSL
//!
//! This module provides parsing and evaluation of `[assertions]` sections
//! in `.pzl` puzzle files. It is gated behind the `puzzle-assert` cargo
//! feature — the entire module compiles out when the feature is off,
//! producing **zero runtime overhead** in the engine hot path.
//!
//! See `ai_docs/reference/PUZZLE_ASSERTION_DSL.md` for the full specification
//! and the rationale for deferring log-derived (event) assertions to a later
//! phase (tracking issue: mtg-0oopj).
//!
//! # Architecture
//! - `parser.rs`    — text → typed `Vec<Assertion>` AST
//! - `evaluator.rs` — `&GameState` + `&GameResult` → `AssertionReport`
//!
//! Both are pure library functions; the runner (CLI or test) decides what to
//! do with the report. The engine core is never touched.

pub mod evaluator;
pub mod parser;

pub use evaluator::{evaluate_assertions, AssertionFailure, AssertionReport};
pub use parser::parse_assertions;

use crate::Result;

/// Which player an assertion targets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerScope {
    /// P0 — the puzzle's "human" / local player (default when scope is omitted)
    Me,
    /// P1 — the puzzle's AI / opponent player
    Opponent,
}

/// Comparison operators for numeric predicates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Comparator {
    /// Evaluate `lhs <op> rhs`
    pub fn eval<T: PartialOrd>(self, lhs: T, rhs: T) -> bool {
        match self {
            Comparator::Eq => lhs == rhs,
            Comparator::Ne => lhs != rhs,
            Comparator::Lt => lhs < rhs,
            Comparator::Le => lhs <= rhs,
            Comparator::Gt => lhs > rhs,
            Comparator::Ge => lhs >= rhs,
        }
    }
}

impl std::fmt::Display for Comparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Comparator::Eq => write!(f, "eq"),
            Comparator::Ne => write!(f, "ne"),
            Comparator::Lt => write!(f, "lt"),
            Comparator::Le => write!(f, "le"),
            Comparator::Gt => write!(f, "gt"),
            Comparator::Ge => write!(f, "ge"),
        }
    }
}

/// The zone referenced in a zone assertion
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertZone {
    Hand,
    Graveyard,
    Battlefield,
    Exile,
    Library,
}

impl std::fmt::Display for AssertZone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssertZone::Hand => write!(f, "hand"),
            AssertZone::Graveyard => write!(f, "graveyard"),
            AssertZone::Battlefield => write!(f, "battlefield"),
            AssertZone::Exile => write!(f, "exile"),
            AssertZone::Library => write!(f, "library"),
        }
    }
}

/// The game-result predicate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResultPred {
    /// P0 (me) won
    Won,
    /// P0 (me) lost
    Lost,
    /// Game was a draw
    Drawn,
    /// Game ended for any reason (winner or draw — just not still running)
    Ended,
}

/// The core of a single assertion (without negation/scope, which wrap it)
#[derive(Debug, Clone)]
pub enum AssertionKind {
    /// Player life total comparison
    Life {
        scope: PlayerScope,
        cmp: Comparator,
        value: i32,
    },
    /// Number of cards in a player-owned zone (or on battlefield controlled by player)
    ZoneCount {
        scope: PlayerScope,
        zone: AssertZone,
        cmp: Comparator,
        value: usize,
    },
    /// Zone contains a card with the given name (case-insensitive)
    ZoneContains {
        scope: PlayerScope,
        zone: AssertZone,
        card_name: String,
    },
    /// Top N cards of library contain a card with the given name
    LibraryTopContains {
        scope: PlayerScope,
        depth: usize,
        card_name: String,
    },
    /// Game result predicate
    GameResult(GameResultPred),
    /// Number of turns played comparison
    TurnNumber { cmp: Comparator, value: u32 },

    /// A triggered ability fired from the named source (case-insensitive).
    /// Empty `source_name` matches any trigger.
    TriggerFired {
        /// Source card name to match (empty = any trigger)
        source_name: String,
    },

    /// A spell was cast with the given name (case-insensitive).
    /// Empty `card_name` matches any spell cast.
    SpellCast {
        /// Card name to match (empty = any spell)
        card_name: String,
    },

    /// A creature with the given name died (case-insensitive).
    /// Empty `card_name` matches any creature death.
    CreatureDied {
        /// Card name to match (empty = any creature death)
        card_name: String,
    },

    /// A player gained life (sum of positive `LifeChanged` deltas in event log).
    LifeGained {
        scope: PlayerScope,
        cmp: Comparator,
        /// Total life gained to compare against
        value: i32,
    },
}

/// A single parsed assertion from an `[assertions]` line
#[derive(Debug, Clone)]
pub struct Assertion {
    /// Whether the predicate is negated (NOT prefix)
    pub negated: bool,
    /// The predicate
    pub kind: AssertionKind,
    /// Original source text, for error messages
    pub source_line: String,
}

impl Assertion {
    /// Parse one assertion line.
    ///
    /// # Errors
    ///
    /// Returns an error if the line cannot be parsed as a valid assertion.
    pub fn parse(line: &str) -> Result<Self> {
        parser::parse_one_assertion(line)
    }
}
