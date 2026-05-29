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
//! 4. On first call: harness initializes game state from network client and runs
//!    `GameLoop::run_until_input` forward until it blocks (NeedInput) or completes
//! 5. On every SUBSEQUENT call: REWIND the persistent shadow game to the start of
//!    the current turn (via `undo.rs::rewind_to_turn_start`), REPLAY the recorded
//!    intra-turn choices forward through `ReplayController`s (logging suppressed),
//!    then continue forward to the next block. This is the shared time-travel
//!    mechanism (undo-log rewind/replay) used by snapshot/resume and the human
//!    fancy-TUI network path — NOT a re-run with per-step idempotency guards
//!    (mtg-j4128 / mtg-610).
//! 6. Returns `"need_input"` (waiting for server), `"complete"` (game done),
//!    or `"error:..."` (fatal error)
//! 7. JS checks `network_get_state()` for `"game_ended"` to know when to stop

use super::client::SharedNetworkClient;
use super::exports::ensure_client;
use super::game_init::{init_game_reserve_only_wasm, process_card_reveal_wasm};
use super::{WasmNetworkLocalController, WasmRemoteController};
use crate::core::PlayerId;
use crate::game::controller::PlayerController;
use crate::game::replay_controller::{ReplayChoice, ReplayController};
use crate::game::{
    derive_player_seed, GameLoop, GameLoopState, GameState, HeuristicController, PlayerSlot, RandomController,
    VerbosityLevel, ZeroController,
};
use crate::undo::GameAction;
use crate::wasm::replay_verifier::{
    capture_pre_rewind, finish_capture, record_turn_start_hash_with_snapshot, verify_replay,
};
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

// ═══════════════════════════════════════════════════════════════════════════
// HARNESS STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Which AI controller kind drives our local player.
///
/// We store the *kind* + derived seed rather than a live controller instance
/// because the rewind/replay model (mtg-j4128 / mtg-610) recreates a fresh
/// controller on every `step_harness()` call: the game is rewound to turn
/// start and replayed forward, so a fresh deterministically-seeded controller
/// produces identical decisions. This mirrors the local-AI rewind path in
/// `fancy_tui.rs` (`run_network_mode_human_v2` / the P2 `ReplayController`),
/// which also recreates the inner AI controller each step.
#[derive(Clone, Copy)]
enum WasmAiControllerKind {
    Random,
    Heuristic,
    Zero,
}

/// Persistent state for the headless AI harness.
///
/// Stored in thread_local storage and initialized on the first call to
/// `run_network_ai_step` after the network client enters the `in_game` state.
///
/// ## Rewind/replay model (mtg-j4128, supersedes the TurnStructure guard hacks)
///
/// `game` persists across calls. The FIRST `step_harness()` runs the loop
/// forward normally. Every SUBSEQUENT call REWINDS `game` to the start of the
/// current turn via `undo.rs::rewind_to_turn_start`, replays the recorded
/// intra-turn choices (ours + opponent's) through `ReplayController`s with
/// logging suppressed, and continues forward to the next block. This is the
/// SAME shared time-travel mechanism used by snapshot/resume and the human
/// fancy-TUI network path — no WASM-specific re-entry guards in the core engine.
struct WasmAiHarness {
    game: GameState,
    our_id: PlayerId,
    opponent_id: PlayerId,
    we_are_p1: bool,
    /// Which controller kind drives our local player.
    controller_kind: WasmAiControllerKind,
    /// Per-slot derived seed for recreating the inner controller deterministically.
    derived_seed: u64,
    /// True once the first forward run has happened. While false, `step_harness`
    /// runs forward without a preceding rewind (there is nothing to rewind yet).
    started: bool,
    /// Debug-only per-turn cache of the post-rewind turn-start state hash.
    /// Each rewind to turn N must reproduce the same hash; a drift means the
    /// undo log is no longer a faithful inverse of forward play (fatal).
    #[cfg(debug_assertions)]
    turn_start_hashes: HashMap<u32, u64>,
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

    let controller_kind = match controller_type {
        "random" | "rand" => WasmAiControllerKind::Random,
        "heuristic" | "heur" => WasmAiControllerKind::Heuristic,
        _ => WasmAiControllerKind::Zero,
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
        controller_kind,
        derived_seed,
        started: false,
        #[cfg(debug_assertions)]
        turn_start_hashes: HashMap::new(),
    })
}

/// Build a fresh local AI controller for our player, wrapped in the network
/// local controller so its choices are submitted to the server. Returned as a
/// boxed `dyn PlayerController` so it can be wrapped in a `ReplayController`.
///
/// A fresh controller is recreated on every step because the rewind/replay
/// model re-runs the turn from its start; `ReplayController` feeds back the
/// already-made choices (without consulting the inner controller) and only
/// delegates to this fresh controller for genuinely new choices.
fn build_local_controller(
    kind: WasmAiControllerKind,
    our_id: PlayerId,
    derived_seed: u64,
    client: &SharedNetworkClient,
) -> Box<dyn PlayerController> {
    match kind {
        WasmAiControllerKind::Random => Box::new(WasmNetworkLocalController::new(
            RandomController::with_seed(our_id, derived_seed),
            client.clone(),
        )),
        WasmAiControllerKind::Heuristic => Box::new(WasmNetworkLocalController::new(
            HeuristicController::with_seed(our_id, derived_seed),
            client.clone(),
        )),
        WasmAiControllerKind::Zero => Box::new(WasmNetworkLocalController::new(
            ZeroController::new(our_id),
            client.clone(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME STEP
// ═══════════════════════════════════════════════════════════════════════════

/// Build the network sync callback that drains pending card reveals and
/// library reorders into the shadow game state. This is the WASM equivalent of
/// the native client's blocking sync mechanism and is identical on the first
/// forward run and on every rewind/replay continuation.
fn make_sync_callback(client: SharedNetworkClient, our_id: PlayerId) -> impl Fn(&mut GameState, u64) {
    move |game: &mut GameState, _target_action: u64| {
        // mtg-589: apply library reorders BEFORE reveals so the shadow's
        // library is in the server-authoritative order before any draw.
        // Protocol sends the order top-to-bottom; the library Vec is
        // bottom-to-top (draw_top pops the last element), so reverse.
        let reorders = client.borrow_mut().drain_library_reorders();
        for (player, new_order) in reorders {
            log::debug!(
                "ai_harness sync_callback: applying library reorder for {:?} ({} cards)",
                player,
                new_order.len()
            );
            if let Some(zones) = game.get_player_zones_mut(player) {
                zones.library.cards = new_order.into_iter().rev().collect();
            }
        }

        let reveals = client.borrow_mut().drain_reveals();
        if !reveals.is_empty() {
            log::debug!("ai_harness sync_callback: processing {} reveals", reveals.len());
            for (owner, card, reason) in reveals {
                process_card_reveal_wasm(game, owner, card, reason, Some(our_id));
            }
        }
    }
}

/// Result of rewinding the shadow game to turn start: the recorded intra-turn
/// choices partitioned by player, plus (debug builds only) the verification
/// capture used to assert the rewind+replay round-trips exactly.
struct RewindResult {
    our_choices: Vec<ReplayChoice>,
    opponent_choices: Vec<ReplayChoice>,
    #[cfg(debug_assertions)]
    verification: Option<crate::wasm::replay_verifier::RewindVerification>,
}

/// Rewind the persistent shadow game to the start of the current turn and
/// partition the recorded intra-turn choices into ours vs the opponent's.
///
/// This is the harness's equivalent of `fancy_tui.rs::rewind_to_turn_start`.
/// It also clears transient game-loop state that is not tracked by the undo
/// log (so it doesn't leak stale values into the replay).
fn harness_rewind_to_turn_start(harness: &mut WasmAiHarness) -> RewindResult {
    let our_id = harness.our_id;

    // Debug capture: snapshot pre-rewind hash/counts BEFORE the rewind mutates state.
    #[cfg(debug_assertions)]
    let pre_capture = capture_pre_rewind(&harness.game);

    let mut undo_log = std::mem::take(&mut harness.game.undo_log);
    let rewind = undo_log.rewind_to_turn_start(&mut harness.game);
    harness.game.undo_log = undo_log;

    let (choice_actions, log_size_at_turn) = match rewind {
        Some((_turn, choices, _rewound, log_size)) => (choices, log_size),
        None => {
            // Undo log disabled — should not happen for the network harness.
            log::warn!("ai_harness: rewind_to_turn_start returned None (undo log disabled?)");
            return RewindResult {
                our_choices: Vec::new(),
                opponent_choices: Vec::new(),
                #[cfg(debug_assertions)]
                verification: None,
            };
        }
    };

    // Debug capture phase 2: snapshot the log tail (about to be truncated) and
    // record the post-rewind turn-start hash, BEFORE truncating the logger.
    #[cfg(debug_assertions)]
    let verification = {
        let mut v = finish_capture(pre_capture, &harness.game, log_size_at_turn);
        record_turn_start_hash_with_snapshot(&mut v, &harness.game);
        Some(v)
    };

    // Truncate the logger to the turn boundary so the replay regenerates exactly
    // the same log prefix (instead of duplicating it).
    harness.game.logger.truncate_to(log_size_at_turn);

    // Clear transient game-loop state not tracked by the undo log; otherwise it
    // would leak stale values from the interrupted run into the replay.
    harness.game.spell_targets.clear();
    harness.game.pending_cast = None;
    harness.game.pending_activation = None;
    harness.game.pending_activation_effect_idx = None;
    harness.game.pending_cycling_search = None;

    // Partition recorded choices by player.
    let mut our_choices = Vec::new();
    let mut opponent_choices = Vec::new();
    for action in choice_actions {
        if let GameAction::ChoicePoint {
            player_id,
            choice: Some(c),
            ..
        } = action
        {
            if player_id == our_id {
                our_choices.push(c);
            } else {
                opponent_choices.push(c);
            }
        }
    }

    log::debug!(
        "ai_harness: rewound to turn start, replaying {} our + {} opponent choices",
        our_choices.len(),
        opponent_choices.len()
    );

    RewindResult {
        our_choices,
        opponent_choices,
        #[cfg(debug_assertions)]
        verification,
    }
}

/// Run a single step of the AI game loop using the shared rewind/replay
/// time-travel mechanism (mtg-j4128 / mtg-610).
///
/// - First call: run the loop forward normally (nothing to rewind yet).
/// - Subsequent calls: REWIND the persistent shadow game to turn start, REPLAY
///   the recorded intra-turn choices (ours + opponent's) with logging
///   suppressed, then continue forward to the next block. Replaying re-evolves
///   the game state through the exact same actions, so no per-step idempotency
///   guards are needed in the core engine.
fn step_harness(harness: &mut WasmAiHarness, client: SharedNetworkClient) -> String {
    let our_id = harness.our_id;
    let we_are_p1 = harness.we_are_p1;
    let opponent_id = harness.opponent_id;
    let kind = harness.controller_kind;
    let derived_seed = harness.derived_seed;

    let result = if !harness.started {
        // ── First call: plain forward run, no rewind. ──────────────────────
        harness.started = true;
        let mut local = build_local_controller(kind, our_id, derived_seed, &client);
        let mut remote = WasmRemoteController::new(opponent_id, client.clone());
        let sync_callback = make_sync_callback(client.clone(), our_id);

        let mut game_loop = GameLoop::new(&mut harness.game)
            .with_verbosity(VerbosityLevel::Normal)
            .with_sync_callback(sync_callback)
            .skip_opening_hands()
            .with_deferred_game_end();

        if we_are_p1 {
            game_loop.run_until_input(local.as_mut(), &mut remote)
        } else {
            game_loop.run_until_input(&mut remote, local.as_mut())
        }
    } else {
        // ── Re-entry: rewind to turn start, replay forward. ────────────────
        let rewound = harness_rewind_to_turn_start(harness);

        // Fresh inner controllers wrapped in ReplayControllers: replay recorded
        // choices first, then delegate to the live controllers for new choices.
        let local = build_local_controller(kind, our_id, derived_seed, &client);
        let mut our_replay = ReplayController::new(our_id, local, rewound.our_choices);

        let fresh_remote = WasmRemoteController::new(opponent_id, client.clone());
        let mut opponent_replay = ReplayController::new(opponent_id, Box::new(fresh_remote), rewound.opponent_choices);

        let sync_callback = make_sync_callback(client.clone(), our_id);

        let run_result = {
            let mut game_loop = GameLoop::new(&mut harness.game)
                .with_verbosity(VerbosityLevel::Normal)
                .with_sync_callback(sync_callback)
                .skip_opening_hands()
                .with_deferred_game_end();

            if we_are_p1 {
                game_loop.run_until_input(&mut our_replay, &mut opponent_replay)
            } else {
                game_loop.run_until_input(&mut opponent_replay, &mut our_replay)
            }
        };

        // Debug invariant: verify the rewind+replay round-tripped to the exact
        // pre-rewind state (turn-start hash stable across rewinds, regenerated
        // log prefix identical). A divergence is a fatal undo-log incompleteness.
        #[cfg(debug_assertions)]
        if let Some(v) = rewound.verification {
            let prior = harness.turn_start_hashes.get(&v.turn_number).copied();
            let outcome = verify_replay(&v, &harness.game, prior);
            if let Some(msg) = outcome.fatal_message() {
                log::error!("ai_harness: {}", msg);
                return format!("error:{}", msg);
            }
            harness
                .turn_start_hashes
                .insert(v.turn_number, v.post_rewind_turn_start_hash);
        }

        run_result
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
