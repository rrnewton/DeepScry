//! DeepScry - a high-performance MTG engine in Rust for AI research
//!
//! An independent Rust engine inspired by the MTG Forge project (which
//! it credits for card data and rules heritage), optimized for efficient
//! tree search and AI gameplay. Not a line-by-line port of Forge's Java.
//!
//! ## Feature Flags
//!
//! - `native`: Enable native platform features (CLI, TUI, file I/O, threading)
//! - `wasm`: Enable WebAssembly support (browser-compatible, no threading)
//! - `verbose-logging`: Enable verbose game event logging (increases allocations)

#![feature(allocator_api)]

pub mod asset_hash;
pub mod core;
pub mod deck_builder;
#[cfg(feature = "native")]
pub mod download;

pub mod error;
pub mod game;
pub mod loader;
pub mod puzzle;
// Scryfall CDN image-URL core (mtg-722 / task #7). Dependency-free so it
// compiles for BOTH the native downloader and the wasm client — one URL
// implementation, no Rust/JS drift.
pub mod scryfall;
// Card-lookup table BUILDER (mtg-722 / task #7) — generator-only (parses the
// Scryfall bulk dump). Native-gated; the wasm client only READS the table.
#[cfg(feature = "native")]
pub mod scryfall_table;
#[cfg(feature = "native")]
pub mod tournament;
pub mod undo;
pub mod version;
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
