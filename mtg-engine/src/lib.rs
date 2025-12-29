//! MTG Forge - High-performance Rust port for AI research
//!
//! This is a port of the MTG Forge game engine from Java to Rust,
//! optimized for efficient tree search and AI gameplay.
//!
//! ## Feature Flags
//!
//! - `native`: Enable native platform features (CLI, TUI, file I/O, threading)
//! - `wasm`: Enable WebAssembly support (browser-compatible, no threading)
//! - `verbose-logging`: Enable verbose game event logging (increases allocations)

#![feature(allocator_api)]

pub mod core;
#[cfg(feature = "native")]
pub mod deck_builder;
#[cfg(feature = "native")]
pub mod download;
pub mod error;
pub mod game;
pub mod loader;
pub mod puzzle;
#[cfg(feature = "native")]
pub mod tournament;
pub mod undo;
pub mod zones;

// WASM-specific modules
#[cfg(feature = "wasm")]
pub mod wasm;

// Networking modules (native only)
#[cfg(feature = "network")]
pub mod network;

pub use error::{MtgError, Result};
