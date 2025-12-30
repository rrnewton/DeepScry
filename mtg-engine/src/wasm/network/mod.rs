//! WASM Network Client Module
//!
//! This module provides non-blocking network controllers for browser-based
//! multiplayer gameplay. It adapts the native network protocol for WASM's
//! event-driven environment.
//!
//! ## Architecture
//!
//! Unlike the native client which uses blocking channels, the WASM client:
//! - Returns `NeedInput` when waiting for server messages (same pattern as human input)
//! - Queues messages received from JavaScript WebSocket callbacks
//! - Lets JavaScript poll for outbound messages to send
//!
//! ## Key Components
//!
//! - `WasmNetworkClient`: State machine managing connection and message queues
//! - `WasmNetworkLocalController`: Wraps local player controller, syncs with server
//! - `WasmRemoteController`: Handles opponent choices from server
//!
//! ## Code Sharing
//!
//! This module reuses protocol types from `crate::network` and shares
//! the same JSON message format as the native client, enabling interoperability.

mod client;
mod exports;
mod local_controller;
mod remote_controller;

pub use client::{NetworkState, WasmNetworkClient};
pub use exports::*;
pub use local_controller::WasmNetworkLocalController;
pub use remote_controller::WasmRemoteController;

// Re-export protocol types for convenience
pub use crate::network::{CardReveal, ChoiceType, ClientMessage, DeckSubmission, RevealReason, ServerMessage};
