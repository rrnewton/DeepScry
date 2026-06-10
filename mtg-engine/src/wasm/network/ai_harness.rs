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
use crate::game::replay_controller::ReplayChoice;
use crate::game::{
    derive_player_seed, GameLoopState, GameState, HeuristicController, PlayerController, PlayerSlot, RandomController,
    ReplayController, ZeroController,
};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

// ═══════════════════════════════════════════════════════════════════════════
// HARNESS STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Persistent state for the headless AI harness.
///
/// Stored in thread_local storage and initialized on the first call to
/// `run_network_ai_step` after the network client enters the `in_game` state.
struct WasmAiHarness {
    game: GameState,
    our_id: PlayerId,
    opponent_id: PlayerId,
    we_are_p1: bool,
    /// The PERSISTENT network controller for OUR player: a type-erased
    /// `WasmNetworkLocalController<Inner>`. Created ONCE and reused across every
    /// `run_network_ai_step` re-entry so the inner controller's RNG advances
    /// monotonically — byte-identical to the server's single persistent
    /// controller (mtg-610). Box-erased (rather than the old enum-dispatch)
    /// so the genuinely-new frontier choice can be delegated through a
    /// [`ReplayController`] on a mid-turn re-entry, exactly as the unified
    /// `fancy_tui` network path does. The micro-cost of dyn dispatch is one
    /// virtual call per real choice — negligible, and it is what lets the
    /// rewind+replay resume model (the gfr2a fix) work here.
    controller: Box<dyn PlayerController>,
    /// The turn number whose forward run we have already started (mtg-610 /
    /// mtg-885). When a `run_network_ai_step` entry finds the game on this
    /// same turn, it is a mid-turn RE-ENTRY (the loop blocked awaiting a
    /// choice and JS re-called us) → rewind to the turn start and replay. When
    /// the turn number differs it is the FIRST forward run of a new turn → run
    /// forward without rewinding. `None` until the first step.
    ///
    /// This replaces the old NO-REWIND resume model, where each re-entry called
    /// `GameLoop::run_until_input` directly on the already-advanced game. That
    /// model re-executed the CURRENT step from the top on resume; for a step
    /// that logs an action BEFORE its priority round (`end_combat_step` logs
    /// `ClearCombat`) the action was logged a SECOND time, advancing the
    /// shadow's `action_count` by one with no state change — a fatal
    /// `compute_view_hash` desync at the next state-hash check (mtg-885).
    forward_run_turn: Option<u32>,
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

    // Create the PERSISTENT network controller for our player (mtg-610),
    // type-erased so the rewind+replay resume path can wrap it in a
    // `ReplayController` for the frontier choice on a mid-turn re-entry.
    let controller: Box<dyn PlayerController> = match controller_type {
        "random" | "rand" => {
            let inner = RandomController::with_seed(our_player_id, derived_seed);
            Box::new(WasmNetworkLocalController::new(inner, client.clone()))
        }
        "heuristic" | "heur" => {
            // HeuristicController is stateful (see `is_safe_to_hold_land_for_main2`),
            // so seed it with the same derived value so cross-mode determinism holds.
            let inner = HeuristicController::with_seed(our_player_id, derived_seed);
            Box::new(WasmNetworkLocalController::new(inner, client.clone()))
        }
        "zero" | _ => {
            let inner = ZeroController::new(our_player_id);
            Box::new(WasmNetworkLocalController::new(inner, client.clone()))
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
        forward_run_turn: None,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME STEP
// ═══════════════════════════════════════════════════════════════════════════

/// Run a single step of the AI game loop via the PRINCIPLED rewind+replay
/// resume model (mtg-610 / mtg-885), unified with the `fancy_tui` network
/// path's resume mechanism.
///
/// On the FIRST forward run of a turn we run the game loop straight through.
/// On every mid-turn RE-ENTRY (the loop blocked awaiting a choice and JS
/// re-called us) we rewind to the turn start and replay both players'
/// recorded choices, delegating ONLY the genuinely-new frontier choice to the
/// persistent inner controller — so no begin-of-step / combat action is
/// re-applied. The old NO-REWIND model re-executed the current step from the
/// top on resume, double-logging `end_combat_step`'s `ClearCombat` and
/// desyncing `action_count` (mtg-885).
///
/// Turn 1 now carries a clean post-setup `ChangeTurn` boundary marker
/// (`GameState::ensure_turn_one_boundary`), so a turn-1 rewind stops there just
/// like turn 2+ — no full-state baseline special case is needed (proven by the
/// native rewind/replay oracle, which round-trips turn 1 through the SAME
/// `rewind_to_turn_start` path).
fn step_harness(harness: &mut WasmAiHarness, client: SharedNetworkClient) -> String {
    let current_turn = harness.game.turn.turn_number;

    // Re-entry detection: if we already started a forward run for this turn
    // number, this entry is a mid-turn re-entry → rewind+replay. Otherwise it
    // is the first forward run of a fresh turn → run straight through.
    let is_reentry = harness.forward_run_turn == Some(current_turn);

    let result = if is_reentry {
        step_replay(harness, client)
    } else {
        step_forward(harness, client)
    };

    // Track the turn we last ran so the next entry can tell forward-vs-reentry.
    harness.forward_run_turn = Some(harness.game.turn.turn_number);

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

/// Build the sync callback that applies pending state-sync entries (reveals +
/// library reorders) from the WS-fed ActionLog. This is the WASM equivalent of
/// the native client's blocking sync mechanism, but non-destructive — see
/// docs/NETWORK_ACTION_LOG.md § 3.2.
fn make_sync_callback(client: SharedNetworkClient, our_id: PlayerId) -> impl Fn(&mut GameState, u64) {
    move |game: &mut GameState, target_action: u64| {
        // mtg-752 L3: apply deltas keyed by game ac, bounded by the position the
        // GameLoop is syncing to (no longer a greedy up-to-frontier drain).
        let applied = client
            .borrow_mut()
            .apply_state_sync_at(game, Some(our_id), target_action);
        if applied > 0 {
            log::debug!("ai_harness sync_callback: applied {} state-sync entries", applied);
        }
    }
}

/// FIRST forward run of a turn. Drives the persistent inner controller and the
/// live remote opponent straight through `run_until_input`.
fn step_forward(harness: &mut WasmAiHarness, client: SharedNetworkClient) -> crate::Result<GameLoopState> {
    let our_id = harness.our_id;
    let we_are_p1 = harness.we_are_p1;
    let opponent_id = harness.opponent_id;

    let mut remote = WasmRemoteController::new(opponent_id, client.clone());
    let sync_callback = make_sync_callback(client, our_id);

    // Shared shadow-driver core (DRY with fancy_tui's network forward run). The
    // headless harness wires no authoritative library-search lookup (mtg-728 is
    // a fancy_tui-only concern), so pass `None` with a concrete type.
    crate::game::replay_controller::run_shadow_until_input(
        &mut harness.game,
        we_are_p1,
        harness.controller.as_mut(),
        &mut remote,
        sync_callback,
        no_searched_card_lookup(),
    )
}

/// Mid-turn re-entry: rewind to the turn start and replay both players'
/// recorded choices, delegating ONLY the new frontier choice to the persistent
/// inner controller (its RNG advances exactly once per real choice →
/// byte-identical to the server's forward-only run). The opponent's choices
/// can't be recomputed locally, so they are replayed from the recorded log and
/// then delegated to a fresh remote controller for anything beyond the replay
/// point. Mirrors `fancy_tui::run_network_ai_replay`.
fn step_replay(harness: &mut WasmAiHarness, client: SharedNetworkClient) -> crate::Result<GameLoopState> {
    let our_id = harness.our_id;
    let we_are_p1 = harness.we_are_p1;
    let opponent_id = harness.opponent_id;

    // Rewind to the start of the current turn, extracting both players'
    // recorded choices. Turn 1 has a post-setup ChangeTurn marker so this stops
    // at the turn boundary, not the pre-game setup.
    let (our_choices, opponent_choices) = rewind_and_partition(harness, our_id, client.clone());

    // Clear transient game-loop state not tracked by the undo log so the replay
    // starts clean (mirrors the fancy_tui replay branch).
    harness.game.sub_action_scratch.spell_targets.clear();
    harness.game.sub_action_scratch.pending_activation = None;
    harness.game.sub_action_scratch.pending_activation_effect_idx = None;
    harness.game.sub_action_scratch.pending_cycling_search = None;

    log::debug!(
        "ai_harness REPLAY: after rewind turn {}, undo_log={}, {} our + {} opponent choices",
        harness.game.turn.turn_number,
        harness.game.undo_log.len(),
        our_choices.len(),
        opponent_choices.len()
    );

    // Wrap the PERSISTENT inner controller in a fresh ReplayController carrying
    // our accumulated choice history. The wrapper ECHOES every prior choice
    // from cache (inner NOT invoked → RNG untouched) and delegates to the
    // persistent inner ONLY for the new frontier choice.
    let inner = std::mem::replace(&mut harness.controller, placeholder_controller(our_id));
    let mut our_replay = ReplayController::new(our_id, inner, our_choices);

    let fresh_remote = WasmRemoteController::new(opponent_id, client.clone());
    let mut opponent_replay = ReplayController::new(opponent_id, Box::new(fresh_remote), opponent_choices);

    let result = {
        let sync_callback = make_sync_callback(client, our_id);
        crate::game::replay_controller::run_shadow_until_input(
            &mut harness.game,
            we_are_p1,
            &mut our_replay,
            &mut opponent_replay,
            sync_callback,
            no_searched_card_lookup(),
        )
    };

    // Recover the persistent inner so its RNG carries forward to the next
    // re-entry / turn.
    harness.controller = our_replay.into_inner();
    result
}

/// Rewind the shadow game to the current turn's start, returning
/// `(our_choices, opponent_choices)` in forward chronological order. Undoes
/// every action back to the turn's `ChangeTurn` boundary (which is also what
/// removes any in-flight `ClearCombat` logged by a forward run that blocked at
/// the EndCombat priority — the mtg-885 fix) and unwinds the reveal-history
/// buffer to the rewound position so the rewound `cards` set is deterministic.
fn rewind_and_partition(
    harness: &mut WasmAiHarness,
    our_id: PlayerId,
    client: SharedNetworkClient,
) -> (Vec<ReplayChoice>, Vec<ReplayChoice>) {
    // The headless harness has no rewind/replay debug verifier, so the
    // turn-start hook is a no-op; the unwind hook drives the shadow
    // undo-completeness unwind (mtg-610): any async opponent instance a reveal
    // stamped past the rewound boundary materialised is removed, and the
    // forward replay re-consumes reveals in lockstep as it re-advances.
    // `rewind_to_turn_start` reverses its collected choices back to FORWARD
    // chronological order, so the partition preserves replay order.
    crate::game::replay_controller::rewind_partition_truncate(
        &mut harness.game,
        our_id,
        |game, retained_action| {
            client.borrow_mut().unwind_state_sync_to(game, retained_action);
        },
        |_game, _log_size_at_turn| {},
    )
}

/// A concretely-typed `None` for the shared `run_shadow_until_input`'s optional
/// `searched_card_lookup` generic. The headless harness wires no authoritative
/// library-search lookup (mtg-728 is a `fancy_tui`-only concern), but the shared
/// helper's `L` type parameter still needs to be inferable — this names a
/// concrete closure type so `None::<L>` resolves without a turbofish at the call
/// site.
fn no_searched_card_lookup() -> Option<impl Fn(&GameState, PlayerId) -> Option<crate::core::CardId> + 'static> {
    None::<fn(&GameState, PlayerId) -> Option<crate::core::CardId>>
}

/// A throwaway controller used to temporarily fill `harness.controller` while
/// the real persistent inner is moved into a `ReplayController`. Never invoked
/// for a choice (the `ReplayController` holds the real inner for the whole
/// replay run); it exists only so `std::mem::replace` can move the box out and
/// back without an `Option` dance.
fn placeholder_controller(our_id: PlayerId) -> Box<dyn PlayerController> {
    Box::new(ZeroController::new(our_id))
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
