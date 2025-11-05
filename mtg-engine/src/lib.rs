//! MTG Forge - High-performance Rust port for AI research
//!
//! This is a port of the MTG Forge game engine from Java to Rust,
//! optimized for efficient tree search and AI gameplay.

// Enable unstable allocator_api for per-thread bump allocators
#![feature(allocator_api)]
#![feature(slice_ptr_get)]

pub mod core;
pub mod error;
pub mod game;
pub mod loader;
pub mod puzzle;
pub mod tournament;
pub mod undo;
pub mod zones;

pub use error::{MtgError, Result};
