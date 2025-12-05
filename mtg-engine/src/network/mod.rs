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

mod controller;
mod local_controller;
mod protocol;
mod remote_controller;

#[cfg(feature = "network")]
mod client;
#[cfg(feature = "network")]
mod server;

pub use controller::*;
pub use local_controller::*;
pub use protocol::*;
pub use remote_controller::*;

#[cfg(feature = "network")]
pub use client::*;
#[cfg(feature = "network")]
pub use server::*;

/// Default port for MTG network games
pub const DEFAULT_PORT: u16 = 17771;
