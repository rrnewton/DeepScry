//! WASM-bindgen exports for network functionality
//!
//! These functions are exposed to JavaScript for WebSocket integration.
//! JavaScript manages the WebSocket connection and calls these functions
//! to pass messages into the WASM module.

use super::client::{new_shared_client, NetworkState, SharedNetworkClient, WasmNetworkClient};
use crate::network::DeckSubmission;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

// Thread-local storage for the global network client
// This allows multiple exports to share the same client instance
thread_local! {
    static NETWORK_CLIENT: RefCell<Option<SharedNetworkClient>> = const { RefCell::new(None) };
}

/// Get or create the global network client
fn with_client<F, R>(f: F) -> R
where
    F: FnOnce(&mut WasmNetworkClient) -> R,
{
    let client = NETWORK_CLIENT.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            *opt = Some(new_shared_client());
        }
        opt.as_ref().unwrap().clone()
    });
    let mut borrowed = client.borrow_mut();
    f(&mut borrowed)
}

/// Get the shared client reference (for use by controllers)
pub fn get_shared_client() -> Option<SharedNetworkClient> {
    NETWORK_CLIENT.with(|cell| cell.borrow().clone())
}

/// Ensure the network client exists and return a shared reference
pub fn ensure_client() -> SharedNetworkClient {
    NETWORK_CLIENT.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            *opt = Some(new_shared_client());
        }
        opt.as_ref().unwrap().clone()
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// CONNECTION LIFECYCLE
// ═══════════════════════════════════════════════════════════════════════════

/// Called when WebSocket connection opens
///
/// JavaScript should call this from the WebSocket onopen handler.
#[wasm_bindgen]
pub fn network_on_open() {
    with_client(|client| client.on_open());
}

/// Called when WebSocket connection closes
///
/// JavaScript should call this from the WebSocket onclose handler.
#[wasm_bindgen]
pub fn network_on_close() {
    with_client(|client| client.on_close());
}

/// Called when WebSocket encounters an error
///
/// JavaScript should call this from the WebSocket onerror handler.
#[wasm_bindgen]
pub fn network_on_error(error: &str) {
    with_client(|client| client.on_error(error));
}

/// Initialize the network client with connection parameters
///
/// Call this before connecting to set up the connection parameters.
/// The deck_json should be a valid DeckSubmission JSON.
#[wasm_bindgen]
pub fn network_init(server_url: &str, password: &str, player_name: &str, deck_json: &str) {
    with_client(|client| {
        client.set_connection_params(server_url, password, player_name, deck_json);
    });
}

/// Queue authentication message
///
/// Call this after the WebSocket opens to authenticate with the server.
/// The deck should be a JSON object with main_deck and sideboard arrays.
#[wasm_bindgen]
pub fn network_authenticate(password: &str, player_name: &str, deck_json: &str) -> Result<(), JsValue> {
    let deck: DeckSubmission =
        serde_json::from_str(deck_json).map_err(|e| JsValue::from_str(&format!("Invalid deck JSON: {}", e)))?;

    with_client(|client| client.authenticate(password, player_name, deck));
    Ok(())
}

/// Queue disconnect message
#[wasm_bindgen]
pub fn network_disconnect() {
    with_client(|client| client.disconnect());
}

/// Reset the network client for a new game
#[wasm_bindgen]
pub fn network_reset() {
    with_client(|client| client.reset());
}

// ═══════════════════════════════════════════════════════════════════════════
// MESSAGE HANDLING
// ═══════════════════════════════════════════════════════════════════════════

/// Process a server message received from WebSocket
///
/// JavaScript should call this from the WebSocket onmessage handler,
/// passing the message data as a JSON string.
///
/// Returns true if the message was processed successfully.
#[wasm_bindgen]
pub fn network_on_message(json: &str) -> bool {
    with_client(|client| client.on_message(json))
}

/// Get the next outbound message to send
///
/// JavaScript should poll this function and send any messages via WebSocket.
/// Returns null if no messages are pending.
#[wasm_bindgen]
pub fn network_get_outbound_message() -> Option<String> {
    with_client(|client| client.get_outbound_message())
}

/// Check if there are outbound messages pending
#[wasm_bindgen]
pub fn network_has_outbound_messages() -> bool {
    with_client(|client| client.has_outbound_messages())
}

/// Queue a ping message for keepalive
#[wasm_bindgen]
pub fn network_ping() {
    with_client(|client| client.ping());
}

// ═══════════════════════════════════════════════════════════════════════════
// STATE QUERIES
// ═══════════════════════════════════════════════════════════════════════════

/// Get the current connection state as a string
///
/// Returns one of: "disconnected", "connecting", "waiting_for_opponent",
/// "in_game", "game_ended", "error"
#[wasm_bindgen]
pub fn network_get_state() -> String {
    with_client(|client| client.state().as_str().to_string())
}

/// Check if currently in a game
#[wasm_bindgen]
pub fn network_is_in_game() -> bool {
    with_client(|client| client.state() == NetworkState::InGame)
}

/// Check if waiting for opponent
#[wasm_bindgen]
pub fn network_is_waiting_for_opponent() -> bool {
    with_client(|client| client.state() == NetworkState::WaitingForOpponent)
}

/// Check if game has ended
#[wasm_bindgen]
pub fn network_is_game_ended() -> bool {
    with_client(|client| client.state() == NetworkState::GameEnded)
}

/// Check if game is ready to start (both players connected)
#[wasm_bindgen]
pub fn network_is_game_ready() -> bool {
    with_client(|client| client.state() == NetworkState::InGame)
}

/// Get our player ID (0 or 1), or -1 if not assigned
#[wasm_bindgen]
pub fn network_get_our_player_id() -> i32 {
    with_client(|client| client.our_player_id().map(|p| p.as_u32() as i32).unwrap_or(-1))
}

/// Get opponent's name, or null if not known
#[wasm_bindgen]
pub fn network_get_opponent_name() -> Option<String> {
    with_client(|client| client.opponent_name().map(|s| s.to_string()))
}

/// Get the last error message, or null if none
#[wasm_bindgen]
pub fn network_get_last_error() -> Option<String> {
    with_client(|client| client.last_error().map(|s| s.to_string()))
}

/// Get the game winner (0 or 1), -1 for draw, -2 if game not ended
#[wasm_bindgen]
pub fn network_get_winner() -> i32 {
    with_client(|client| {
        match client.winner() {
            Some(Some(player)) => player.as_u32() as i32,
            Some(None) => -1, // Draw
            None => -2,       // Game not ended
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// CHOICE/SYNC STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Check if a choice request is pending from the server
#[wasm_bindgen]
pub fn network_has_choice_request() -> bool {
    with_client(|client| client.has_choice_request())
}

/// Check if the last submitted choice was acknowledged
#[wasm_bindgen]
pub fn network_is_choice_acknowledged() -> bool {
    with_client(|client| client.is_choice_acknowledged())
}

/// Check if an opponent choice is pending
#[wasm_bindgen]
pub fn network_has_opponent_choice() -> bool {
    with_client(|client| client.has_opponent_choice())
}

/// Check if card reveals are pending
#[wasm_bindgen]
pub fn network_has_pending_reveals() -> bool {
    with_client(|client| client.has_pending_reveals())
}

/// Get pending reveals as JSON array
///
/// Returns a JSON array of reveal objects, or "[]" if none.
#[wasm_bindgen]
pub fn network_get_pending_reveals_json() -> String {
    with_client(|client| {
        let reveals: Vec<_> = client
            .drain_reveals()
            .iter()
            .map(|(owner, card, reason)| {
                serde_json::json!({
                    "owner": owner.as_u32(),
                    "card_id": card.card_id.as_u32(),
                    "name": card.name,
                    "mana_cost": card.mana_cost,
                    "type_line": card.type_line,
                    "text": card.text,
                    "pt": card.pt,
                    "reason": format!("{:?}", reason),
                })
            })
            .collect();
        serde_json::to_string(&reveals).unwrap_or_else(|_| "[]".to_string())
    })
}

/// Get the current choice request as JSON, or null if none
#[wasm_bindgen]
pub fn network_get_choice_request_json() -> Option<String> {
    with_client(|client| {
        client.peek_choice_request().map(|req| {
            serde_json::to_string(&serde_json::json!({
                "choice_seq": req.choice_seq,
                "choice_type": format!("{:?}", req.choice_type),
                "options": req.options,
                "action_count": req.action_count,
            }))
            .unwrap_or_else(|_| "{}".to_string())
        })
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// DECK SUBMISSION HELPER
// ═══════════════════════════════════════════════════════════════════════════

/// Create a deck submission JSON from card arrays
///
/// Takes two JSON arrays: main_deck and sideboard, where each element
/// is [card_name, count].
#[wasm_bindgen]
pub fn network_create_deck_json(main_deck_json: &str, sideboard_json: &str) -> Result<String, JsValue> {
    let main_deck: Vec<(String, u8)> = serde_json::from_str(main_deck_json)
        .map_err(|e| JsValue::from_str(&format!("Invalid main_deck JSON: {}", e)))?;

    let sideboard: Vec<(String, u8)> = serde_json::from_str(sideboard_json)
        .map_err(|e| JsValue::from_str(&format!("Invalid sideboard JSON: {}", e)))?;

    let deck = DeckSubmission::new(main_deck, sideboard);

    serde_json::to_string(&deck).map_err(|e| JsValue::from_str(&format!("Failed to serialize deck: {}", e)))
}
