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

// Networking modules
// Protocol types are always available; client/server require "network" feature
pub mod network;

// Unified web server (static files + lobby WebSocket proxy + optional TLS).
// Replaces the old dual-process deploy (python http.server + `mtg server`).
#[cfg(feature = "web-server")]
pub mod web_server;

pub use error::{MtgError, Result};
