//! Error types for MTG Forge

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MtgError {
    #[error("Invalid card format: {0}")]
    InvalidCardFormat(String),

    #[error("Invalid deck format: {0}")]
    InvalidDeckFormat(String),

    #[error("Entity not found: {0}")]
    EntityNotFound(u32),

    #[error("Invalid game action: {0}")]
    InvalidAction(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    #[cfg(feature = "native")]
    IoError(#[from] std::io::Error),

    #[error("IO error: {0}")]
    #[cfg(not(feature = "native"))]
    IoError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Task join error: {0}")]
    #[cfg(feature = "native")]
    JoinError(#[from] tokio::task::JoinError),

    /// Game needs human input to continue (WASM only)
    ///
    /// This is not really an error - it's a signal that the game loop
    /// should pause and wait for human input. The contained ChoiceContext
    /// describes what input is needed.
    ///
    /// Used by `run_until_input()` to implement the interrupt pattern.
    #[error("Game needs input: waiting for human player")]
    NeedInput(crate::game::controller::ChoiceContext),
}

pub type Result<T> = std::result::Result<T, MtgError>;
