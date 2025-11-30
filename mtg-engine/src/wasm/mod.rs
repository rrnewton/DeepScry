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
//! - All I/O (file loading, network) is handled by the JavaScript host
//!
//! ## Usage Example (JavaScript)
//!
//! ```javascript
//! import init, { WasmGame, version } from './mtg_forge_rs.js';
//!
//! async function main() {
//!     await init();
//!     console.log("MTG Forge version:", version());
//!
//!     // Create a new game
//!     const game = new WasmGame("Player 1", "Player 2", 20);
//!
//!     // Get game state as JSON
//!     const state = JSON.parse(game.get_state_json());
//!     console.log("Turn:", state.turn_number);
//! }
//! ```
//!
//! ## Limitations (compared to native)
//!
//! - No file system access (card/deck data must be provided from JS)
//! - No threading (single-threaded game loop)
//! - Token creation requires pre-loaded token definitions

use wasm_bindgen::prelude::*;

use crate::game::{
    GameLoop, GameState, HeuristicController, PlayerController, RandomController, VerbosityLevel,
    ZeroController,
};

/// Initialize the WASM module (called automatically)
#[wasm_bindgen(start)]
pub fn wasm_init() {
    // Set up panic hook for better error messages in browser console
    console_error_panic_hook_setup();
}

/// Set up console_error_panic_hook for better WASM error messages
fn console_error_panic_hook_setup() {
    // This provides better error messages in the browser console
    // when a panic occurs in Rust code
    std::panic::set_hook(Box::new(|panic_info| {
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic".to_string()
        };

        let location = panic_info
            .location()
            .map(|l| format!(" at {}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();

        web_sys::console::error_1(&format!("MTG Forge panic: {}{}", message, location).into());
    }));
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

/// Controller type for WASM games
#[wasm_bindgen]
#[derive(Clone, Copy, Debug)]
pub enum WasmControllerType {
    /// Always passes priority (does nothing)
    Zero,
    /// Makes random legal choices
    Random,
    /// Uses heuristic AI evaluation
    Heuristic,
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
    /// Create a new game with two players
    ///
    /// # Arguments
    /// * `p1_name` - Player 1's name
    /// * `p2_name` - Player 2's name
    /// * `starting_life` - Starting life total for both players
    #[wasm_bindgen(constructor)]
    pub fn new(p1_name: &str, p2_name: &str, starting_life: i32) -> WasmGame {
        let game = GameState::new_two_player(p1_name.to_string(), p2_name.to_string(), starting_life);

        WasmGame {
            game,
            p1_controller_type: WasmControllerType::Heuristic,
            p2_controller_type: WasmControllerType::Heuristic,
            game_seed: 0,
        }
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

        let mut controller1: Box<dyn PlayerController> = match self.p1_controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(p1_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(p1_id, self.game_seed)),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(p1_id)),
        };

        let mut controller2: Box<dyn PlayerController> = match self.p2_controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(p2_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(p2_id, self.game_seed.wrapping_add(1))),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(p2_id)),
        };

        let mut game_loop = GameLoop::new(&mut self.game)
            .with_verbosity(VerbosityLevel::Silent)
            .with_max_turns(max_turns);

        let result = game_loop.run_game(controller1.as_mut(), controller2.as_mut());

        match result {
            Ok(game_result) => {
                let result_view = WasmGameResult {
                    winner: game_result.winner.map(|p| {
                        self.game
                            .players
                            .iter()
                            .position(|player| player.id == p)
                            .unwrap_or(0)
                    }),
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

        let mut controller1: Box<dyn PlayerController> = match self.p1_controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(p1_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(p1_id, self.game_seed)),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(p1_id)),
        };

        let mut controller2: Box<dyn PlayerController> = match self.p2_controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(p2_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(p2_id, self.game_seed.wrapping_add(1))),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(p2_id)),
        };

        let mut game_loop = GameLoop::new(&mut self.game)
            .with_verbosity(VerbosityLevel::Silent);

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
