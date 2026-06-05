//! Network protocol for client/server multiplayer
//!
//! This module implements a WebSocket-based protocol for networked MTG games
//! using deterministic simulation with hidden information enforcement.
//!
//! ## Architecture
//!
//! - **Server**: Runs authoritative game state, controls RNG, knows all cards
//! - **Clients**: Run shadow game state, only see revealed cards
//! - **Protocol**: Choice-based sync (not full state transfer)
//!
//! ## Key Principles
//!
//! 1. **Deterministic simulation**: Clients run independent simulation synced via choices
//! 2. **Hidden information by construction**: Clients never receive opponent hand contents,
//!    library order, or RNG state
//! 3. **State verification**: Hash-based checksums at each choice point
//!
//! ## Default Port
//!
//! The default port is 17771.

// Protocol types are always available (needed by WASM network module)
mod protocol;
pub use protocol::*;

// Shared reveal processing (used by both native and WASM clients)
mod reveal_processor;
pub use reveal_processor::*;

// Generic append-only, action_count-indexed log. The foundational primitive
// for the network re-architecture (mtg-o99ow)
// that backs (a) each controller's private choice buffer, (b) the
// NetworkClient-owned shadow state-sync log, and (c) future MCTS
// rollout logs. See docs/NETWORK_ACTION_LOG.md for the ownership split.
mod action_log;
pub use action_log::*;

// Strongly-typed payload for the shadow state-sync `ActionLog<StateSyncEntry>`.
// See docs/NETWORK_ACTION_LOG.md § 3.2.
mod state_sync;
pub use state_sync::*;

// Strongly-typed payload for the per-controller choice buffer
// `ActionLog<ChoiceEntry>`. See docs/NETWORK_ACTION_LOG.md § 3.1.
mod choice_entry;
pub use choice_entry::*;

// Native controller types (require std::sync::mpsc and network feature)
#[cfg(feature = "network")]
mod client;
#[cfg(feature = "network")]
mod controller;
#[cfg(feature = "network")]
pub mod lobby;
#[cfg(feature = "network")]
mod local_controller;
#[cfg(feature = "network")]
pub mod memory;
#[cfg(feature = "network")]
mod mvar;
#[cfg(feature = "network")]
mod remote_controller;
#[cfg(feature = "network")]
mod server;

#[cfg(feature = "network")]
pub use client::*;
#[cfg(feature = "network")]
pub use controller::*;
#[cfg(feature = "network")]
pub use local_controller::*;
#[cfg(feature = "network")]
pub use remote_controller::*;
#[cfg(feature = "network")]
pub use server::*;

/// Default port for MTG network games
pub const DEFAULT_PORT: u16 = 17771;
