//! Headless AI Harness for Network Game Testing
//!
//! Provides the `run_network_ai_step` WASM export used by `wasm_ai_harness.html`
//! for headless browser-based equivalence testing.
//!
//! ## Design
//!
//! The harness manages a thread_local [`WasmAiHarness`] that persists game state
//! and controller state across multiple calls to `run_network_ai_step`. JavaScript
//! calls this function from a poll timer and from the WebSocket message handler,
//! allowing the game loop to advance incrementally as network messages arrive.
//!
//! Unlike the FancyTUI (which renders to a DOM canvas), this harness is headless:
//! it runs the game loop, submits choices to the server, and signals completion
//! via the return value.
//!
//! ## Flow
//!
//! 1. JS calls `network_init(...)` and connects WebSocket → state = `connecting`
//! 2. Server sends `GameStarted` → state = `in_game`
//! 3. JS poll timer calls `run_network_ai_step(controller, seed)`
//! 4. On first call: harness initializes game state from network client
//! 5. On each call: runs `GameLoop::run_until_input` until blocked
//! 6. Returns `"need_input"` (waiting for server), `"complete"` (game done),
//!    `"choice_made"` (choice submitted), or `"error:..."` (fatal error)
//! 7. JS checks `network_get_state()` for `"game_ended"` to know when to stop

use super::client::SharedNetworkClient;
use super::exports::ensure_client;
use super::game_init::init_game_reserve_only_wasm;
use super::{WasmNetworkLocalController, WasmRemoteController};
use crate::core::PlayerId;
use crate::game::{
    derive_player_seed, GameLoop, GameLoopState, GameState, HeuristicController, PlayerSlot, RandomController,
    VerbosityLevel, ZeroController,
};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

// ═══════════════════════════════════════════════════════════════════════════
// HARNESS STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Concrete AI controller variants stored in the harness.
///
/// We use enum dispatch to avoid `dyn` overhead and to preserve each
/// controller's internal state (e.g., RNG seed for `RandomController`)
/// across multiple calls to `run_network_ai_step`.
enum WasmAiControllerEnum {
    Random(WasmNetworkLocalController<RandomController>),
    Heuristic(WasmNetworkLocalController<HeuristicController>),
    Zero(WasmNetworkLocalController<ZeroController>),
}

/// Persistent state for the headless AI harness.
///
/// Stored in thread_local storage and initialized on the first call to
/// `run_network_ai_step` after the network client enters the `in_game` state.
struct WasmAiHarness {
    game: GameState,
    our_id: PlayerId,
    opponent_id: PlayerId,
    we_are_p1: bool,
    controller: WasmAiControllerEnum,
}

thread_local! {
    static AI_HARNESS: RefCell<Option<WasmAiHarness>> = const { RefCell::new(None) };
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize the harness from the current network client state.
///
/// Called automatically by `run_network_ai_step` on the first invocation
/// after the server sends `GameStarted`.
fn init_harness(client: &SharedNetworkClient, controller_type: &str, seed: u32) -> Result<WasmAiHarness, String> {
    let client_ref = client.borrow();

    let starting_life = client_ref.starting_life();
    let our_player_id = client_ref
        .our_player_id()
        .ok_or_else(|| "our_player_id not assigned by server".to_string())?;
    let opponent_name = client_ref.opponent_name().unwrap_or("Opponent").to_string();
    let our_name = client_ref.our_name().unwrap_or("AI").to_string();
    let ranges = client_ref
        .deck_card_ids()
        .cloned()
        .ok_or_else(|| "deck_card_ids not set (GameStarted not received?)".to_string())?;
    let rng_state = client_ref.rng_state().to_vec();
    // Clone token definitions before releasing the borrow
    let token_defs: Vec<(String, crate::loader::CardDefinition)> = client_ref
        .token_definitions()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    drop(client_ref);

    let we_are_p1 = our_player_id.as_u32() == 0;

    let (p1_name, p2_name) = if we_are_p1 {
        (our_name, opponent_name)
    } else {
        (opponent_name, our_name)
    };

    let mut game = init_game_reserve_only_wasm(p1_name, p2_name, starting_life, &ranges);

    // Populate token definitions so shadow game can create tokens (e.g. Clue tokens)
    for (name, def) in token_defs {
        game.token_definitions.insert(name, std::sync::Arc::new(def));
    }

    // Initialize RNG from server state for deterministic shuffles
    if !rng_state.is_empty() {
        use rand_chacha::ChaCha12Rng;
        match bincode::deserialize::<ChaCha12Rng>(&rng_state) {
            Ok(rng) => {
                *game.rng.borrow_mut() = rng;
                log::info!(
                    "ai_harness: Initialized RNG from server state ({} bytes)",
                    rng_state.len()
                );
            }
            Err(e) => {
                log::warn!("ai_harness: Failed to deserialize server RNG state: {}", e);
            }
        }
    }

    // Determine opponent player ID
    let opponent_id = if we_are_p1 {
        game.players[1].id
    } else {
        game.players[0].id
    };

    // Treat the JS-supplied `seed` as the MASTER seed (not a per-player seed)
    // and derive our per-slot controller seed via the canonical helper. This
    // keeps the WASM network harness in lockstep with the native CLI: two
    // headless harnesses joining the same server with the same `--seed` will
    // produce identical controller decisions for whichever slot the server
    // assigns each one. Without this, the harness used to pass `seed as u64`
    // verbatim, which silently disagreed with the native salt scheme and was
    // a desync risk (see `docs/NETWORK_ARCHITECTURE.md`).
    let our_slot = PlayerSlot::from_index(if we_are_p1 { 0 } else { 1 }).unwrap_or(PlayerSlot::P1);
    let derived_seed = derive_player_seed(seed as u64, our_slot);

    // Create the controller for our player
    let controller = match controller_type {
        "random" | "rand" => {
            let inner = RandomController::with_seed(our_player_id, derived_seed);
            WasmAiControllerEnum::Random(WasmNetworkLocalController::new(inner, client.clone()))
        }
        "heuristic" | "heur" => {
            // HeuristicController is stateful (see `is_safe_to_hold_land_for_main2`),
            // so seed it with the same derived value so cross-mode determinism holds.
            let inner = HeuristicController::with_seed(our_player_id, derived_seed);
            WasmAiControllerEnum::Heuristic(WasmNetworkLocalController::new(inner, client.clone()))
        }
        "zero" | _ => {
            let inner = ZeroController::new(our_player_id);
            WasmAiControllerEnum::Zero(WasmNetworkLocalController::new(inner, client.clone()))
        }
    };

    log::info!(
        "ai_harness: Initialized game (controller={}, seed={}, we_are_p1={}, life={})",
        controller_type,
        seed,
        we_are_p1,
        starting_life
    );

    Ok(WasmAiHarness {
        game,
        our_id: our_player_id,
        opponent_id,
        we_are_p1,
        controller,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME STEP
// ═══════════════════════════════════════════════════════════════════════════

/// Run a single step of the AI game loop.
///
/// Called from the harness state. Runs `GameLoop::run_until_input` until
/// the game loop blocks waiting for network input, or until the game ends.
fn step_harness(harness: &mut WasmAiHarness, client: SharedNetworkClient) -> String {
    let our_id = harness.our_id;
    let we_are_p1 = harness.we_are_p1;
    let opponent_id = harness.opponent_id;

    // Create a remote controller for the opponent (cheap, just holds refs)
    let mut remote = WasmRemoteController::new(opponent_id, client.clone());

    // Build the sync callback that applies pending state-sync entries
    // (reveals + library reorders) from the WS-fed ActionLog. This is the
    // WASM equivalent of the native client's blocking sync mechanism, but
    // non-destructive — see docs/NETWORK_ACTION_LOG.md § 3.2.
    let client_for_sync = client.clone();
    let sync_callback = move |game: &mut GameState, target_action: u64| {
        // mtg-o99ow L3: apply deltas keyed by game ac, bounded by the position the
        // GameLoop is syncing to (no longer a greedy up-to-frontier drain).
        let applied = client_for_sync
            .borrow_mut()
            .apply_state_sync_at(game, Some(our_id), target_action);
        if applied > 0 {
            log::debug!("ai_harness sync_callback: applied {} state-sync entries", applied);
        }
    };

    // Run the game loop until blocked or complete
    let result = {
        let mut game_loop = GameLoop::new(&mut harness.game)
            .with_verbosity(VerbosityLevel::Normal)
            .with_sync_callback(sync_callback)
            .skip_opening_hands()
            .with_deferred_game_end();

        match &mut harness.controller {
            WasmAiControllerEnum::Random(ctrl) => {
                if we_are_p1 {
                    game_loop.run_until_input(ctrl, &mut remote)
                } else {
                    game_loop.run_until_input(&mut remote, ctrl)
                }
            }
            WasmAiControllerEnum::Heuristic(ctrl) => {
                if we_are_p1 {
                    game_loop.run_until_input(ctrl, &mut remote)
                } else {
                    game_loop.run_until_input(&mut remote, ctrl)
                }
            }
            WasmAiControllerEnum::Zero(ctrl) => {
                if we_are_p1 {
                    game_loop.run_until_input(ctrl, &mut remote)
                } else {
                    game_loop.run_until_input(&mut remote, ctrl)
                }
            }
        }
    };

    match result {
        Ok(GameLoopState::Complete(_)) => {
            log::info!("ai_harness: Game loop completed");
            "complete".to_string()
        }
        Ok(GameLoopState::AwaitingInput(_)) => {
            // Controller returned NeedInput - we're blocked waiting for server message
            "need_input".to_string()
        }
        Err(e) => {
            log::error!("ai_harness: Game loop error: {:?}", e);
            format!("error:{}", e)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WASM EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

/// Run one step of the AI network game loop.
///
/// Called from JavaScript (via poll timer or WebSocket message handler).
/// On first call after `GameStarted`, initializes game state from the
/// network client. On subsequent calls, continues from the saved state.
///
/// # Parameters
/// - `controller_type`: `"random"`, `"heuristic"`, or `"zero"`
/// - `seed`: RNG seed for the random controller
///
/// # Returns
/// One of:
/// - `"need_input"` - blocked waiting for a server message; call again after next message
/// - `"complete"` - game loop finished (game may not be ended yet; wait for `game_ended` message)
/// - `"choice_made"` - a choice was submitted to the server (alias for need_input currently)
/// - `"error:<msg>"` - fatal error
#[wasm_bindgen]
pub fn run_network_ai_step(controller_type: &str, seed: u32) -> String {
    let client = match ensure_client_opt() {
        Some(c) => c,
        None => return "error:no_network_client".to_string(),
    };

    AI_HARNESS.with(|cell| {
        let mut opt = cell.borrow_mut();

        // Initialize on first call
        if opt.is_none() {
            match init_harness(&client, controller_type, seed) {
                Ok(harness) => *opt = Some(harness),
                Err(e) => {
                    log::error!("ai_harness: Init failed: {}", e);
                    return format!("error:{}", e);
                }
            }
        }

        let harness = opt.as_mut().unwrap();
        step_harness(harness, client)
    })
}

/// Reset the AI harness state.
///
/// Call this when starting a new game or after the network client resets.
#[wasm_bindgen]
pub fn network_ai_reset() {
    AI_HARNESS.with(|cell| {
        *cell.borrow_mut() = None;
    });
    log::debug!("ai_harness: Reset");
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Get the shared client if it exists (non-creating variant for harness use)
fn ensure_client_opt() -> Option<SharedNetworkClient> {
    // ensure_client() creates the client if it doesn't exist.
    // For the harness we just need the one that was already set up by network_init.
    Some(ensure_client())
}
