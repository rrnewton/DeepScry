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
mod protocol;

pub use controller::*;
pub use protocol::*;

/// Default port for MTG network games
pub const DEFAULT_PORT: u16 = 17771;
