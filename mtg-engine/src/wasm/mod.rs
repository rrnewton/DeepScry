//! WebAssembly bindings for MTG Forge
//!
//! This module provides WASM-compatible APIs for the MTG game engine,
//! enabling browser-based gameplay.
//!
//! ## Architecture
//!
//! The WASM build exposes a JavaScript-friendly API through `wasm-bindgen`:
//! - Game state is managed in Rust and serialized to JSON for JS consumption
//! - Player controllers receive choices via callbacks from JavaScript
//! - Card and deck data is loaded from pre-serialized bincode files
//!
//! ## Usage Example (JavaScript)
//!
//! ```javascript
//! import init, { WasmCardDatabase, WasmGame, version } from './mtg_engine.js';
//!
//! async function main() {
//!     await init();
//!     console.log("MTG Forge version:", version());
//!
//!     // Load deck list + per-set card bins on demand (mtg-464). The decks
//!     // and tokens bins are content-addressed: resolve their hashed names
//!     // from the manifest (tokens+decks cache-skew fix) instead of a fixed URL.
//!     const setIndex  = await fetch('/data/sets/index.json').then(r => r.json());
//!     const decksData = await fetch(`/data/${setIndex.decks}`).then(r => r.arrayBuffer());
//!     const cardDb = new WasmCardDatabase();
//!     cardDb.load_decks(new Uint8Array(decksData));
//!     // Fetch only the sets a deck needs:
//!     const setFiles = new Set();
//!     for (const name of JSON.parse(cardDb.get_deck_card_names_json("white_weenie_classic")))
//!         setFiles.add(setIndex.cards[name]);
//!     await Promise.all([...setFiles].map(async f => {
//!         const r = await fetch(`/data/sets/${f}`);
//!         cardDb.load_set(new Uint8Array(await r.arrayBuffer()));
//!     }));
//!
//!     // Create a game with loaded decks
//!     const game = WasmGame.from_database(cardDb, "white_weenie_classic", "mono_black_control");
//!     game.run_ai_game(100);
//! }
//! ```
//!
//! ## Limitations (compared to native)
//!
//! - No file system access (card/deck data must be provided from JS)
//! - No threading (single-threaded game loop)
//! - Token creation requires pre-loaded token definitions

#[cfg(all(feature = "wasm-tui", target_arch = "wasm32"))]
pub mod fancy_tui;

#[cfg(all(feature = "wasm-tui", target_arch = "wasm32"))]
pub mod deck_builder;

/// Structured view model for the native HTML GUI (`web/native_game.html`).
///
/// Built independently of `target_arch` so unit tests can validate the model
/// even without a wasm32 toolchain. The `tui_get_gui_view_model_json` WASM
/// binding lives in `fancy_tui.rs` (where the WASM-only `WasmFancyTuiState`
/// lives) and delegates to the helpers here.
pub mod gui_view_model;

pub mod human_controller;
pub mod replay_verifier;
pub mod rich_input_controller;

#[cfg(target_arch = "wasm32")]
pub mod image_overlay;

/// Network module for WASM multiplayer
///
/// Provides non-blocking network controllers that return `NeedInput` instead
/// of blocking on channels. JavaScript manages WebSocket and queues messages.
#[cfg(all(feature = "wasm-network", target_arch = "wasm32"))]
pub mod network;

// Re-export network functions for wasm-bindgen
#[cfg(all(feature = "wasm-network", target_arch = "wasm32"))]
pub use network::*;

pub use human_controller::{PendingChoice, WasmHumanController};
pub use rich_input_controller::WasmRichInputController;

use std::collections::HashMap;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

use crate::core::PlayerId;
use crate::game::logger::OutputMode;
use crate::game::{
    derive_player_seed, GameLoop, GameState, HeuristicController, PlayerController, PlayerSlot, RandomController,
    VerbosityLevel, ZeroController,
};
use crate::loader::{CardDefinition, DeckEntry, DeckList};

/// Initialize the WASM module (called automatically)
#[wasm_bindgen(start)]
pub fn wasm_init() {
    // Set up panic hook for better error messages in browser console
    console_error_panic_hook::set_once();

    // Initialize the log crate to output to browser console
    // Default level is Info; can be changed with console_log::init_with_level()
    // For debug output, use: console_log::init_with_level(log::Level::Debug)
    console_log::init_with_level(log::Level::Info).ok();
}

/// Version information
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Check if WASM module is properly initialized
#[wasm_bindgen]
pub fn is_ready() -> bool {
    true
}

/// Set the logging level for the WASM module
///
/// Valid levels: "trace", "debug", "info", "warn", "error"
/// Default level at startup is "info".
///
/// Example from JavaScript:
/// ```js
/// set_log_level("debug");  // Enable debug messages
/// set_log_level("trace");  // Enable all messages including trace
/// set_log_level("warn");   // Only warnings and errors
/// ```
#[wasm_bindgen]
pub fn set_log_level(level: &str) {
    let log_level = match level.to_lowercase().as_str() {
        "trace" => log::Level::Trace,
        "debug" => log::Level::Debug,
        "info" => log::Level::Info,
        "warn" | "warning" => log::Level::Warn,
        "error" => log::Level::Error,
        _ => {
            web_sys::console::warn_1(&format!("Unknown log level '{}', using 'info'", level).into());
            log::Level::Info
        }
    };
    // Re-initialize the logger with the new level
    // Note: console_log::init_with_level returns Err if already initialized,
    // but the log crate's set_max_level works for runtime changes
    log::set_max_level(log_level.to_level_filter());
    log::info!("Log level set to: {}", level);
}

/// WASM-compatible card and deck database
///
/// Loads card definitions and deck lists from pre-serialized bincode data.
/// Use the `mtg export-wasm` command to generate the data files.
#[wasm_bindgen]
pub struct WasmCardDatabase {
    pub(crate) cards: HashMap<String, Arc<CardDefinition>>,
    pub(crate) decks: HashMap<String, DeckList>,
    /// Token definitions loaded from per-deck token packs
    pub(crate) tokens: HashMap<String, Arc<CardDefinition>>,
}

#[wasm_bindgen]
impl WasmCardDatabase {
    /// Create an empty card database
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmCardDatabase {
        WasmCardDatabase {
            cards: HashMap::new(),
            decks: HashMap::new(),
            tokens: HashMap::new(),
        }
    }

    /// Load a per-set card bin from bincode data (mtg-464).
    ///
    /// The data should be the contents of one `data/sets/<YYYY>-<CODE>.bin`
    /// file generated by `mtg export-wasm`. Definitions are merged into the
    /// existing card map idempotently (same `Arc`-wrap, `or_insert_with`
    /// pattern as [`Self::load_tokens`]), so calling this multiple times with
    /// overlapping or repeat sets is safe.
    ///
    /// Returns the number of *new* cards inserted by this call (i.e. excludes
    /// definitions already present from a prior set).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if bincode deserialization fails.
    pub fn load_set(&mut self, data: &[u8]) -> Result<u32, JsValue> {
        let cards: HashMap<String, CardDefinition> = bincode::deserialize(data)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize set bin: {}", e)))?;

        let mut newly_inserted: u32 = 0;
        for (name, mut def) in cards {
            // `parsed_svars` is `#[serde(skip)]`, so a bincode-deserialized
            // CardDefinition arrives with an EMPTY parsed_svars map. Trigger /
            // ability parsing resolves `Execute$ <SVar>` effects via
            // `parsed_svars` (loader/card.rs parse_triggers), so without this
            // rebuild EVERY SVar-backed trigger silently parses to zero effects
            // (e.g. City of Brass `Taps`->TrigDamage self-ping, Su-Chi death
            // ->TrigMana) — diverging the WASM target from native, which loads
            // cards from cardsfolder with parsed_svars populated. The native
            // network path already rebuilds here (network/client.rs,
            // reveal_processor.rs); WASM `load_set` must do the same. (mtg-8scpx)
            def.rebuild_parsed_svars();
            self.cards.entry(name).or_insert_with(|| {
                newly_inserted += 1;
                Arc::new(def)
            });
        }

        web_sys::console::log_1(
            &format!("load_set: +{} new cards (total: {})", newly_inserted, self.cards.len()).into(),
        );
        Ok(newly_inserted)
    }

    /// Load token definitions from bincode data
    ///
    /// The data should be the contents of `tokens.bin` generated by `mtg export-wasm`.
    /// Tokens are merged into the existing database.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if bincode deserialization fails.
    pub fn load_tokens(&mut self, data: &[u8]) -> Result<u32, JsValue> {
        let tokens: HashMap<String, CardDefinition> = bincode::deserialize(data)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize tokens: {}", e)))?;

        let count = tokens.len() as u32;
        for (name, mut def) in tokens {
            // See `load_set`: rebuild parsed_svars dropped by `#[serde(skip)]`
            // so token triggers/abilities that reference SVars via `Execute$`
            // resolve to their effects rather than silently parsing empty.
            // (mtg-8scpx)
            def.rebuild_parsed_svars();
            self.tokens.insert(name, Arc::new(def));
        }

        web_sys::console::log_1(&format!("Loaded {} token definitions", count).into());
        Ok(count)
    }

    /// Load decks from bincode data
    ///
    /// The data should be the contents of `decks.bin` generated by `mtg export-wasm`.
    /// Returns the number of decks loaded, or an error message.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if bincode deserialization fails.
    pub fn load_decks(&mut self, data: &[u8]) -> Result<u32, JsValue> {
        let decks: HashMap<String, DeckList> = bincode::deserialize(data)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize decks: {}", e)))?;

        let count = decks.len() as u32;
        self.decks = decks;

        web_sys::console::log_1(&format!("Loaded {} decks", count).into());
        Ok(count)
    }

    /// Get the number of loaded cards
    pub fn card_count(&self) -> u32 {
        self.cards.len() as u32
    }

    /// Get the number of loaded decks
    pub fn deck_count(&self) -> u32 {
        self.decks.len() as u32
    }

    /// Get a list of available deck names as JSON array
    pub fn get_deck_names_json(&self) -> String {
        let names: Vec<&String> = self.decks.keys().collect();
        serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string())
    }

    /// Get deck info as JSON (name, card count)
    pub fn get_deck_info_json(&self, deck_name: &str) -> String {
        if let Some(deck) = self.decks.get(deck_name) {
            let info = serde_json::json!({
                "name": deck_name,
                "main_deck_count": deck.total_cards(),
                "sideboard_count": deck.sideboard_size(),
                "unique_cards": deck.unique_card_names().len()
            });
            serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string())
        } else {
            format!("{{\"error\": \"Deck '{}' not found\"}}", deck_name)
        }
    }

    /// Get deck as JSON for network submission
    ///
    /// Returns a DeckSubmission-compatible JSON object with main_deck and sideboard
    /// as arrays of [card_name, count] pairs.
    pub fn get_deck_json(&self, deck_name: &str) -> String {
        if let Some(deck) = self.decks.get(deck_name) {
            // Convert deck to network submission format
            let main_deck: Vec<(String, u8)> = deck.main_deck.iter().map(|e| (e.card_name.clone(), e.count)).collect();
            let sideboard: Vec<(String, u8)> = deck.sideboard.iter().map(|e| (e.card_name.clone(), e.count)).collect();

            let submission = serde_json::json!({
                "main_deck": main_deck,
                "sideboard": sideboard
            });
            serde_json::to_string(&submission).unwrap_or_else(|_| "{}".to_string())
        } else {
            format!("{{\"error\": \"Deck '{}' not found\"}}", deck_name)
        }
    }

    /// Check if a card is available in the database
    pub fn has_card(&self, card_name: &str) -> bool {
        self.cards.contains_key(card_name)
    }

    /// Check if a deck is available in the database
    pub fn has_deck(&self, deck_name: &str) -> bool {
        self.decks.contains_key(deck_name)
    }

    /// Get the number of loaded token definitions
    pub fn token_count(&self) -> u32 {
        self.tokens.len() as u32
    }

    /// Check if all cards needed for a deck are loaded
    ///
    /// Returns a list of missing card names, or empty if all cards are available.
    pub fn get_missing_cards_for_deck(&self, deck_name: &str) -> Vec<String> {
        if let Some(deck) = self.decks.get(deck_name) {
            deck.unique_card_names()
                .into_iter()
                .filter(|name| !self.cards.contains_key(name))
                .collect()
        } else {
            vec![format!("Deck '{}' not found", deck_name)]
        }
    }

    /// Get the list of card names needed for a deck (for fetching)
    pub fn get_deck_card_names_json(&self, deck_name: &str) -> String {
        if let Some(deck) = self.decks.get(deck_name) {
            let names = deck.unique_card_names();
            serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string())
        } else {
            "[]".to_string()
        }
    }

    /// Register a custom deck from JSON
    ///
    /// This allows custom decks (created in the deck builder and stored in localStorage)
    /// to be registered with the card database so they can be used in games.
    ///
    /// JSON format: { "main_deck": [[card_name, count], ...], "sideboard": [[card_name, count], ...] }
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if JSON parsing fails.
    pub fn register_custom_deck(&mut self, deck_name: &str, deck_json: &str) -> Result<(), JsValue> {
        // Parse the deck JSON
        let parsed: serde_json::Value = serde_json::from_str(deck_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse deck JSON: {}", e)))?;

        let mut main_deck = Vec::new();
        let mut sideboard = Vec::new();

        // Parse main_deck: [[card_name, count], ...]
        if let Some(main) = parsed.get("main_deck").and_then(|v| v.as_array()) {
            for entry in main {
                if let Some(arr) = entry.as_array() {
                    if arr.len() >= 2 {
                        if let (Some(name), Some(count)) = (arr[0].as_str(), arr[1].as_u64()) {
                            main_deck.push(DeckEntry {
                                card_name: name.to_string(),
                                count: count as u8,
                            });
                        }
                    }
                }
            }
        }

        // Parse sideboard (optional)
        if let Some(side) = parsed.get("sideboard").and_then(|v| v.as_array()) {
            for entry in side {
                if let Some(arr) = entry.as_array() {
                    if arr.len() >= 2 {
                        if let (Some(name), Some(count)) = (arr[0].as_str(), arr[1].as_u64()) {
                            sideboard.push(DeckEntry {
                                card_name: name.to_string(),
                                count: count as u8,
                            });
                        }
                    }
                }
            }
        }

        let deck = DeckList {
            main_deck,
            sideboard,
            commanders: Vec::new(),
        };
        let card_count = deck.total_cards();

        web_sys::console::log_1(&format!("Registered custom deck '{}' with {} cards", deck_name, card_count).into());

        self.decks.insert(deck_name.to_string(), deck);
        Ok(())
    }
}

impl Default for WasmCardDatabase {
    fn default() -> Self {
        Self::new()
    }
}

/// Controller type for WASM games
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasmControllerType {
    /// Always passes priority (does nothing)
    Zero,
    /// Makes random legal choices
    Random,
    /// Uses heuristic AI evaluation
    Heuristic,
    /// Human player (interactive input via UI)
    Human,
    /// Fixed script controller (uses WasmRichInputController)
    /// Script must be set separately via set_p1_script() before launching
    Fixed,
    /// Network player (connects to remote server)
    /// Uses WasmNetworkLocalController for local player
    Network,
    /// Remote opponent in network game
    /// Uses WasmRemoteController - returns choices from server messages
    Remote,
}

/// Create an AI controller for the given controller type and player.
///
/// Handles the three standard AI controller types (Zero, Random, Heuristic).
/// For Human, Fixed, Network, and Remote types, falls back to Zero — callers
/// that need these should handle them before calling this function.
///
/// # Arguments
/// * `controller_type` - The type of controller to create
/// * `player_id` - The player ID this controller manages
/// * `seed` - Per-player seed (already derived from the master via
///   [`crate::game::derive_player_seed`]). Both `Random` and `Heuristic`
///   receive this seed; passing `0` means the controller's RNG starts from
///   the same fixed point as a `--seed=0` native run, which is what callers
///   want when they have no explicit seed configured.
pub fn create_ai_controller(
    controller_type: WasmControllerType,
    player_id: crate::core::PlayerId,
    seed: u64,
) -> Box<dyn PlayerController> {
    match controller_type {
        WasmControllerType::Zero => Box::new(ZeroController::new(player_id)),
        WasmControllerType::Random => Box::new(RandomController::with_seed(player_id, seed)),
        WasmControllerType::Heuristic => Box::new(HeuristicController::with_seed(player_id, seed)),
        WasmControllerType::Human
        | WasmControllerType::Fixed
        | WasmControllerType::Network
        | WasmControllerType::Remote => Box::new(ZeroController::new(player_id)),
    }
}

/// WASM-compatible game wrapper
///
/// This struct wraps the Rust GameState and provides a JavaScript-friendly API.
#[wasm_bindgen]
pub struct WasmGame {
    game: GameState,
    p1_controller_type: WasmControllerType,
    p2_controller_type: WasmControllerType,
    game_seed: u64,
}

#[wasm_bindgen]
impl WasmGame {
    /// Create a new game with two players (no decks)
    ///
    /// # Arguments
    /// * `p1_name` - Player 1's name
    /// * `p2_name` - Player 2's name
    /// * `starting_life` - Starting life total for both players
    #[wasm_bindgen(constructor)]
    pub fn new(p1_name: &str, p2_name: &str, starting_life: i32) -> WasmGame {
        let mut game = GameState::new_two_player(p1_name.to_string(), p2_name.to_string(), starting_life);

        // Configure logger for WASM: capture to memory, enable normal verbosity
        game.logger.set_output_mode(OutputMode::Memory);
        game.logger.set_verbosity(VerbosityLevel::Normal);

        WasmGame {
            game,
            p1_controller_type: WasmControllerType::Heuristic,
            p2_controller_type: WasmControllerType::Heuristic,
            game_seed: 0,
        }
    }

    /// Create a new game with decks from the card database
    ///
    /// # Arguments
    /// * `card_db` - The loaded card database
    /// * `p1_deck_name` - Name of player 1's deck
    /// * `p2_deck_name` - Name of player 2's deck
    /// * `starting_life` - Starting life total for both players
    /// * `seed` - Random seed for shuffling and game RNG
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if a deck is not found or card definitions are missing.
    pub fn from_database(
        card_db: &WasmCardDatabase,
        p1_deck_name: &str,
        p2_deck_name: &str,
        starting_life: i32,
        seed: u64,
    ) -> Result<WasmGame, JsValue> {
        // Look up decks
        let p1_deck = card_db
            .decks
            .get(p1_deck_name)
            .ok_or_else(|| JsValue::from_str(&format!("Deck '{}' not found", p1_deck_name)))?;
        let p2_deck = card_db
            .decks
            .get(p2_deck_name)
            .ok_or_else(|| JsValue::from_str(&format!("Deck '{}' not found", p2_deck_name)))?;

        // Create game state
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), starting_life);
        game.seed_rng(seed);

        // Configure logger for WASM: capture to memory, enable normal verbosity
        game.logger.set_output_mode(OutputMode::Memory);
        game.logger.set_verbosity(VerbosityLevel::Normal);

        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Helper to add cards from a deck entry
        let add_deck_cards = |game: &mut GameState,
                              owner: PlayerId,
                              entry: &DeckEntry,
                              cards: &HashMap<String, Arc<CardDefinition>>|
         -> Result<(), String> {
            let card_def = cards
                .get(&entry.card_name)
                .ok_or_else(|| format!("Card '{}' not found in database", entry.card_name))?;

            for _ in 0..entry.count {
                let card_id = game.next_entity_id();
                let card = card_def.instantiate(card_id, owner);
                // Insert card into game's card registry
                game.cards.insert(card_id, card);
                // Add to player's library zone
                if let Some(zones) = game.get_player_zones_mut(owner) {
                    zones.library.add(card_id);
                }
            }
            Ok(())
        };

        // Add player 1's deck
        for entry in &p1_deck.main_deck {
            add_deck_cards(&mut game, p1_id, entry, &card_db.cards).map_err(|e| JsValue::from_str(&e))?;
        }

        // Add player 2's deck
        for entry in &p2_deck.main_deck {
            add_deck_cards(&mut game, p2_id, entry, &card_db.cards).map_err(|e| JsValue::from_str(&e))?;
        }

        // Copy token definitions from card database into game state
        if !card_db.tokens.is_empty() {
            game.token_definitions = card_db.tokens.clone();
            web_sys::console::log_1(
                &format!("Loaded {} token definitions into game", game.token_definitions.len()).into(),
            );
        }

        // Shuffle libraries
        game.shuffle_library(p1_id);
        game.shuffle_library(p2_id);

        // Draw opening hands (7 cards each)
        for _ in 0..7 {
            let _ = game.draw_card(p1_id);
            let _ = game.draw_card(p2_id);
        }

        web_sys::console::log_1(
            &format!(
                "Created game: {} ({} cards) vs {} ({} cards)",
                p1_deck_name,
                p1_deck.total_cards(),
                p2_deck_name,
                p2_deck.total_cards()
            )
            .into(),
        );

        Ok(WasmGame {
            game,
            p1_controller_type: WasmControllerType::Heuristic,
            p2_controller_type: WasmControllerType::Heuristic,
            game_seed: seed,
        })
    }

    /// Set the controller type for player 1
    pub fn set_p1_controller(&mut self, controller_type: WasmControllerType) {
        self.p1_controller_type = controller_type;
    }

    /// Set the controller type for player 2
    pub fn set_p2_controller(&mut self, controller_type: WasmControllerType) {
        self.p2_controller_type = controller_type;
    }

    /// Set the game seed for reproducible games
    pub fn set_seed(&mut self, seed: u64) {
        self.game_seed = seed;
        self.game.seed_rng(seed);
    }

    /// Get the current game state as JSON
    ///
    /// Returns a JSON string containing:
    /// - Turn number
    /// - Current phase/step
    /// - Active player
    /// - Player life totals
    /// - Battlefield state (simplified)
    pub fn get_state_json(&self) -> String {
        // Create a simplified view of the game state for JS
        let state = WasmGameStateView {
            turn_number: self.game.turn.turn_number,
            current_step: format!("{:?}", self.game.turn.current_step),
            active_player_idx: self.game.turn.active_player_idx,
            players: self
                .game
                .players
                .iter()
                .map(|p| WasmPlayerView {
                    name: p.name.to_string(),
                    life: p.life,
                    lands_played: p.lands_played_this_turn,
                })
                .collect(),
            battlefield_count: self.game.battlefield.cards.len(),
            stack_count: self.game.stack.cards.len(),
        };

        serde_json::to_string(&state).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
    }

    /// Get the current turn number
    pub fn get_turn_number(&self) -> u32 {
        self.game.turn.turn_number
    }

    /// Get a player's life total
    pub fn get_player_life(&self, player_idx: usize) -> i32 {
        self.game.players.get(player_idx).map(|p| p.life).unwrap_or(0)
    }

    /// Check if the game is over
    pub fn is_game_over(&self) -> bool {
        // Check if any player has 0 or less life
        self.game.players.iter().any(|p| p.life <= 0)
    }

    /// Run the game with AI controllers until completion
    ///
    /// Returns a JSON string with the game result:
    /// - winner: player index or null for draw
    /// - turns_played: number of turns
    /// - end_reason: why the game ended
    pub fn run_ai_game(&mut self, max_turns: u32) -> String {
        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Treat `self.game_seed` as the MASTER seed (not a P1 seed) and derive
        // per-player seeds via the canonical helper. Replaces an earlier
        // seed/seed+1 hack that was inconsistent with the native CLI's salt
        // scheme and silently caused identical --seed runs in WASM and native
        // to produce completely different choice streams.
        let p1_seed = derive_player_seed(self.game_seed, PlayerSlot::P1);
        let p2_seed = derive_player_seed(self.game_seed, PlayerSlot::P2);
        let mut controller1 = create_ai_controller(self.p1_controller_type, p1_id, p1_seed);
        let mut controller2 = create_ai_controller(self.p2_controller_type, p2_id, p2_seed);

        // Scope game_loop tightly so self.game can be accessed in match arms
        let result = {
            let mut game_loop = GameLoop::new(&mut self.game)
                .with_verbosity(VerbosityLevel::Normal)
                .with_max_turns(max_turns);
            game_loop.run_game(controller1.as_mut(), controller2.as_mut())
        };

        match result {
            Ok(game_result) => {
                let result_view = WasmGameResult {
                    winner: game_result
                        .winner
                        .map(|p| self.game.players.iter().position(|player| player.id == p).unwrap_or(0)),
                    turns_played: game_result.turns_played,
                    end_reason: format!("{:?}", game_result.end_reason),
                };
                serde_json::to_string(&result_view).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    /// Run a single turn with AI controllers
    ///
    /// Returns true if game is still ongoing, false if game ended
    pub fn run_one_turn(&mut self) -> bool {
        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Same per-slot derivation as `run_ai_game` — see that method's comment.
        let p1_seed = derive_player_seed(self.game_seed, PlayerSlot::P1);
        let p2_seed = derive_player_seed(self.game_seed, PlayerSlot::P2);
        let mut controller1 = create_ai_controller(self.p1_controller_type, p1_id, p1_seed);
        let mut controller2 = create_ai_controller(self.p2_controller_type, p2_id, p2_seed);

        let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);

        match game_loop.run_turns(controller1.as_mut(), controller2.as_mut(), 1) {
            Ok(result) => result.winner.is_none(),
            Err(_) => false,
        }
    }

    /// Get the game logs as a JSON array of strings
    pub fn get_logs_json(&self) -> String {
        let logs: Vec<String> = self.game.logger.logs().iter().map(|l| l.message.clone()).collect();
        serde_json::to_string(&logs).unwrap_or_else(|_| "[]".to_string())
    }

    /// Clear the game logs
    pub fn clear_logs(&mut self) {
        self.game.logger.clear_logs();
    }
}

/// Simplified game state view for JSON serialization
#[derive(serde::Serialize)]
struct WasmGameStateView {
    turn_number: u32,
    current_step: String,
    active_player_idx: usize,
    players: Vec<WasmPlayerView>,
    battlefield_count: usize,
    stack_count: usize,
}

/// Simplified player view for JSON serialization
#[derive(serde::Serialize)]
struct WasmPlayerView {
    name: String,
    life: i32,
    lands_played: u8,
}

/// Game result for JSON serialization
#[derive(serde::Serialize)]
struct WasmGameResult {
    winner: Option<usize>,
    turns_played: u32,
    end_reason: String,
}
