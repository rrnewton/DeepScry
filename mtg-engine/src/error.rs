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
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Task join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, MtgError>;
