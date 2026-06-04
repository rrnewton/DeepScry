//! WASM Network Local Controller
//!
//! Wraps the local player's controller and coordinates with the server.
//! Returns `NeedInput` when waiting for server synchronization.
//!
//! This is generic over any `PlayerController`, mirroring the native
//! `NetworkLocalController<C>`. For AI controllers like Random, the inner
//! controller makes choices immediately. For Human controllers, the inner
//! controller may return NeedInput waiting for user input.
//!
//! ## Flow
//!
//! 1. Wait for ChoiceRequest from server (or NeedInput)
//! 2. Delegate to inner controller for actual choice
//! 3. Queue SubmitChoice message
//! 4. Wait for ChoiceAccepted (or NeedInput)
//! 5. Return choice to GameLoop

use super::client::SharedNetworkClient;
use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;

/// Extended choice context variants for network waiting
///
/// These are returned when we need to wait for the network.
/// They use the existing ChoiceContext enum but with empty data
/// to signal a "waiting" state to the UI layer.
fn waiting_for_server_context() -> ChoiceContext {
    // Use SpellAbility context with empty data to signal waiting
    ChoiceContext::SpellAbility {
        available: vec![],
        formatted_choices: vec!["Waiting for server...".to_string()],
    }
}

fn waiting_for_ack_context() -> ChoiceContext {
    ChoiceContext::SpellAbility {
        available: vec![],
        formatted_choices: vec!["Waiting for acknowledgment...".to_string()],
    }
}

/// WASM Network Local Controller
///
/// Wraps any `PlayerController` and ensures synchronization with the server
/// before and after each choice. This mirrors the native `NetworkLocalController<C>`.
///
/// For AI controllers (Random, Heuristic, Zero), the inner controller makes
/// choices immediately. For Human controllers, the inner controller may return
/// NeedInput waiting for user input.
///
/// ## State Machine
///
/// The controller tracks choice submission state to prevent duplicate processing.
/// The state is stored in the shared network client (not locally) so it persists
/// across controller instances:
/// 1. Wait for ChoiceRequest from server
/// 2. If we already submitted for this request (tracking by choice_seq), wait for ack
/// 3. Make choice via inner controller
/// 4. Submit to server (client tracks the sequence number)
/// 5. Return choice to local game (don't wait for ack - local game can advance)
/// 6. When ack arrives, client clears submitted state for next request
pub struct WasmNetworkLocalController<C: PlayerController> {
    /// The inner controller that makes actual decisions
    inner: C,
    /// Shared reference to the network client
    network_client: SharedNetworkClient,
}

impl<C: PlayerController> WasmNetworkLocalController<C> {
    /// Create a new network local controller wrapping an existing controller
    pub fn new(inner: C, network_client: SharedNetworkClient) -> Self {
        Self { inner, network_client }
    }

    /// Get a mutable reference to the inner controller
    pub fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Check if a ChoiceRequest is available and we haven't already submitted for it
    ///
    /// Returns Some(choice_seq) if we should make a choice, None if we should wait
    fn check_choice_request_ready(&self) -> Option<u32> {
        let client = self.network_client.borrow();
        let last_submitted = client.last_submitted_choice_seq();

        if let Some(req) = client.peek_choice_request() {
            // Check if we already submitted for this sequence
            if last_submitted == Some(req.choice_seq) {
                // Already submitted, wait for ack
                log::debug!(
                    "WasmNetworkLocalController: Already submitted for seq={}, waiting for ack",
                    req.choice_seq
                );
                None
            } else {
                log::debug!(
                    "WasmNetworkLocalController: ChoiceRequest seq={} ready (last_submitted={:?})",
                    req.choice_seq,
                    last_submitted
                );
                Some(req.choice_seq)
            }
        } else {
            None
        }
    }

    /// Check if choice was acknowledged (clears submitted state)
    fn check_and_clear_ack(&self) -> bool {
        let client = self.network_client.borrow();
        let acked = client.is_choice_acknowledged();
        if acked {
            drop(client);
            self.network_client.borrow_mut().clear_last_submitted_choice_seq();
        }
        acked
    }

    /// Check if we have a pending submission waiting for ack
    fn has_pending_submission(&self) -> bool {
        self.network_client.borrow().last_submitted_choice_seq().is_some()
    }

    /// Get server's abilities from the current ChoiceRequest for DESYNC DETECTION.
    ///
    /// Used to validate that locally-computed abilities match the server's. If they
    /// don't match, this is a FATAL DESYNC error (per NETWORK_ARCHITECTURE.md).
    /// This data is for validation only, NOT for recovery — we never substitute
    /// server abilities for locally-computed ones.
    ///
    /// Returns the abilities if available, skipping index 0 ("Pass priority" placeholder).
    fn get_server_abilities(&self) -> Option<Vec<SpellAbility>> {
        let client = self.network_client.borrow();
        client.peek_choice_request().and_then(|req| {
            req.abilities.as_ref().map(|server_abilities| {
                // Extract non-None abilities from server list (index 0 is "Pass")
                server_abilities
                    .iter()
                    .skip(1) // Skip "Pass priority" placeholder
                    .filter_map(|opt| opt.clone())
                    .collect()
            })
        })
    }

    /// Get server's option count from the current ChoiceRequest for DESYNC DETECTION.
    ///
    /// Used to validate that the local hand/option count matches the server's.
    /// If they don't match, this is a FATAL DESYNC error.
    ///
    /// Returns the number of options in the server's ChoiceRequest.
    fn get_server_option_count(&self) -> usize {
        let client = self.network_client.borrow();
        client.peek_choice_request().map(|req| req.options.len()).unwrap_or(0)
    }

    /// Get server's discard count from the current ChoiceRequest for DESYNC DETECTION.
    ///
    /// Used to validate that the local discard count matches the server's.
    /// If they don't match, this is a FATAL DESYNC error.
    ///
    /// Returns the count from ChoiceType::Discard, or None if not a discard choice.
    fn get_server_discard_count(&self) -> Option<usize> {
        use crate::network::ChoiceType;
        let client = self.network_client.borrow();
        client.peek_choice_request().and_then(|req| match &req.choice_type {
            ChoiceType::Discard { count } => Some(*count),
            _ => None,
        })
    }

    /// Submit a choice to the server
    ///
    /// CRITICAL: Uses the server's action_count from ChoiceRequest, NOT the local view's count.
    /// The local WASM game state doesn't actually execute server actions, so view.action_count()
    /// would be wrong. The server's action_count is authoritative.
    ///
    /// The client tracks the submitted sequence number to prevent duplicate processing.
    fn submit_choice_to_server(&self, choice_indices: Vec<usize>, view: &GameStateView) {
        let mut client = self.network_client.borrow_mut();

        // Get server's action_count from the current ChoiceRequest
        let action_count = client
            .peek_choice_request()
            .map(|req| req.action_count)
            .unwrap_or_else(|| {
                log::warn!(
                    "WasmNetworkLocalController: No ChoiceRequest available, using local action_count {} (may cause sync error)",
                    view.action_count()
                );
                view.action_count() as u64
            });

        let state_hash = self.compute_submit_hash(&client, view, action_count);

        // submit_choice internally tracks the sequence and consumes the ChoiceRequest
        client.submit_choice(choice_indices, action_count, state_hash);
    }

    /// Compute the view hash for a choice submission and, in network-debug mode,
    /// emit the shared WASM_CARD_DETAIL + WASM_FULL_UNDO_DUMP diagnostics. Shared
    /// by BOTH the normal (`submit_choice_to_server`) and SMART-damage
    /// (`submit_damage_choice_to_server`) submit paths so the damage/blocker
    /// submissions are no longer an UNINSTRUMENTED hash source — the rejected
    /// seed-2 client hash was produced here and was previously invisible
    /// (mtg-yexvc / mtg-mb668 class-A). `action_count` is the server-echoed
    /// action_count from the current ChoiceRequest.
    fn compute_submit_hash(
        &self,
        client: &super::client::WasmNetworkClient,
        view: &GameStateView,
        action_count: u64,
    ) -> Option<u64> {
        if client.is_network_debug() {
            let hash = crate::game::compute_view_hash(view);
            // Debug: log each field used in compute_view_hash so we can compare with server
            use crate::core::PlayerId;
            let local_action_count = view.action_count() as u64;
            log::trace!(
                "WASM_HASH_DEBUG: turn={} active={} step_hash_u32={} action_count={} (local={}) | P0: life={} hand={} lib={} gyard={} | P1: life={} hand={} lib={} gyard={} | bf={} stack={} | hash={:016x}",
                view.turn_number(),
                view.active_player().as_u32(),
                view.current_step().as_hash_u32(),
                action_count,  // server's action_count
                local_action_count,  // local (WASM) action_count
                view.player_life(PlayerId::new(0)),
                view.player_hand_size(PlayerId::new(0)),
                view.player_library_size(PlayerId::new(0)),
                view.player_graveyard_size(PlayerId::new(0)),
                view.player_life(PlayerId::new(1)),
                view.player_hand_size(PlayerId::new(1)),
                view.player_library_size(PlayerId::new(1)),
                view.player_graveyard_size(PlayerId::new(1)),
                view.battlefield().len(),
                view.stack().len(),
                hash,
            );
            // mtg-mb668 class-A: emit the WASM shadow's REAL per-card detail (the
            // server only ever saw a reconstructed client view because the WASM
            // SubmitChoice carries debug_info=None). log::warn so the e2e captures
            // it. KEYED BY choice_seq (the authoritative shared sequence the server
            // reports in its mismatch box) — NOT action_count, which is not 1:1
            // between server and shadow. This pinpoints WHICH battlefield card's
            // (tapped,ctrl) or graveyard id diverges at the failing choice_seq.
            let choice_seq = client
                .peek_choice_request()
                .map(|req| req.choice_seq)
                .unwrap_or(u32::MAX);
            log::warn!(
                "WASM_CARD_DETAIL seq={} server_ac={} local_ac={} hash={:016x} {}",
                choice_seq,
                action_count,
                local_action_count,
                hash,
                crate::game::state_hash::format_view_card_detail(view),
            );
            // mtg-mb668 class-A: UNCONDITIONAL shadow undo-log tail (network_debug).
            // The action-count-mismatch dump below NEVER fires because the WASM
            // ECHOES the server action_count (action_count == local_action_count
            // always), so the shadow undo-log was never captured. The shadow's
            // action SEQUENCE is ground-truth (unlike the suspect seq↔hash mapping):
            // diffing it against the server's SERVER_FULL_UNDO_DUMP names the actions
            // the shadow SKIPS (the reserved-id branch-on-absence sites, mtg-mb668).
            {
                const SHADOW_DUMP_TAIL: usize = 60;
                log::warn!(
                    "WASM_FULL_UNDO_DUMP_BEGIN seq={} local_ac={}\n{}WASM_FULL_UNDO_DUMP_END",
                    choice_seq,
                    local_action_count,
                    view.format_last_n_actions(SHADOW_DUMP_TAIL),
                );
            }
            // Always dump last actions in debug mode for comparison with server
            if action_count != local_action_count {
                log::warn!(
                    "WASM_HASH_DEBUG: ACTION COUNT MISMATCH! server={} local={} (diff={})\nWASM last 15 actions:\n{}",
                    action_count,
                    local_action_count,
                    action_count as i64 - local_action_count as i64,
                    view.format_last_n_actions(15),
                );
                // mtg-610: emit a BOUNDED tail of the WASM-shadow undo log so the
                // JS e2e harness (which truncates console lines for display) can
                // capture it to a file and diff it against the server's tail dump.
                // Bracketed with unique markers for byte-exact extraction. A
                // desync surfaces near the log tail (the diverging entries are the
                // most recent), so 120 actions captures the divergence with the
                // index prefix [NNNN] preserved for alignment, while staying
                // bounded (no O(n) per-mismatch blowup on a long game).
                const FULL_UNDO_DUMP_TAIL: usize = 120;
                log::warn!(
                    "WASM_FULL_UNDO_DUMP_BEGIN server={} local={} diff={}\n{}WASM_FULL_UNDO_DUMP_END",
                    action_count,
                    local_action_count,
                    action_count as i64 - local_action_count as i64,
                    view.format_last_n_actions(FULL_UNDO_DUMP_TAIL),
                );
            } else {
                log::trace!("WASM_ACTION_DUMP: last 30 actions:\n{}", view.format_last_n_actions(30),);
            }
            Some(hash)
        } else {
            None
        }
    }

    /// Submit a SMART damage assignment choice, attaching the authoritative
    /// CardId in `target_card_ids` so the opponent's shadow game can resolve
    /// the correct blocker by CardId rather than by index.
    ///
    /// Used for `choose_blocker_for_lethal_damage` and
    /// `choose_blocker_for_remaining_damage` (mtg-418).
    fn submit_damage_choice_to_server(&self, choice_indices: Vec<usize>, blocker_id: CardId, view: &GameStateView) {
        let mut client = self.network_client.borrow_mut();

        let action_count = client
            .peek_choice_request()
            .map(|req| req.action_count)
            .unwrap_or_else(|| view.action_count() as u64);

        let state_hash = self.compute_submit_hash(&client, view, action_count);

        client.submit_choice_with_targets(choice_indices, action_count, state_hash, Some(vec![blocker_id]));
    }
}

impl<C: PlayerController + 'static> PlayerController for WasmNetworkLocalController<C> {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    /// Expose ourselves for downcasting so the WASM network dispatch can reach
    /// the inner controller (e.g. inject a human's pending choice into
    /// `WasmNetworkLocalController<WasmHumanController>`; mtg-679 unification).
    fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        Some(self)
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Auto-pass when no abilities available.
        // When available_count == 0, the ONLY valid choice is pass (index 0).
        // We submit immediately if ChoiceRequest is ready, otherwise return NeedInput
        // to exit the game loop and wait for the server's ChoiceRequest to arrive.
        //
        // IMPORTANT: We must NOT advance the local game past this point without
        // a ChoiceRequest, because the server sends ChoiceRequests one at a time
        // and expects a response for each. If we advance past multiple auto-pass
        // points locally, we'd need to answer multiple future ChoiceRequests, but
        // the game state would already be past those points.
        if available.is_empty() {
            if self.check_choice_request_ready().is_some() {
                log::debug!(
                    "WasmNetworkLocalController: Auto-pass with 0 abilities (ChoiceRequest ready, submitting immediately)"
                );
                self.submit_choice_to_server(vec![0], view);
                return ChoiceResult::Ok(None);
            } else {
                // No ChoiceRequest yet - return NeedInput to exit the game loop.
                // tui_run_turn() will re-trigger when the ChoiceRequest arrives
                // (via onMessageProcessed in JavaScript).
                log::debug!("WasmNetworkLocalController: Auto-pass with 0 abilities (no ChoiceRequest, waiting)");
                return ChoiceResult::NeedInput(waiting_for_server_context());
            }
        }

        // Check if ChoiceRequest is ready (not already submitted for this request)
        if self.check_choice_request_ready().is_none() {
            // Either no ChoiceRequest, or we already submitted for it
            // Check if we're waiting for ack
            if self.has_pending_submission() {
                // Already submitted, waiting for ack - check if ack arrived
                if self.check_and_clear_ack() {
                    // Ack arrived, but no new ChoiceRequest yet - wait for next one
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            // No ChoiceRequest and no pending submission - wait for server
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        // BEHAVIORAL IDENTITY: Use locally-computed abilities (same as native).
        // The sync_callback has already drained CardRevealed messages, so local state
        // MUST be synchronized with server. We compute abilities from local state and
        // validate they match server's abilities — any mismatch is FATAL DESYNC.

        // FATAL validation: verify local abilities match server's.
        // Per NETWORK_ARCHITECTURE.md: desync is ALWAYS fatal, never papered over.
        {
            let server_abilities = self.get_server_abilities();
            if let Some(ref server_abs) = server_abilities {
                if server_abs.len() != available.len() {
                    let msg = format!(
                        "FATAL DESYNC: Local abilities ({}) != server abilities ({}). \
                         Local: {:?}, Server: {:?}",
                        available.len(),
                        server_abs.len(),
                        available,
                        server_abs,
                    );
                    log::error!("{}", msg);
                    return ChoiceResult::Error(msg);
                }
            }
        }

        // ChoiceRequest is ready - delegate to inner controller with LOCAL abilities
        match self.inner.choose_spell_ability_to_play(view, available) {
            ChoiceResult::Ok(choice) => {
                // Submit choice to server and consume the ChoiceRequest.
                // CRITICAL: Index into `available` (original order), NOT a sorted view.
                // The server assigns option indices based on the original availability order.
                let choice_indices = match &choice {
                    None => vec![0], // Pass
                    Some(ability) => vec![available.iter().position(|a| a == ability).map(|i| i + 1).unwrap_or(0)],
                };
                self.submit_choice_to_server(choice_indices, view);

                // Return the choice immediately - local game can advance
                // The ack will arrive asynchronously and be handled next time
                ChoiceResult::Ok(choice)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            other => other,
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        // The inner controller's full target vector (variable count) is mapped
        // to choice_indices below, so multi-target choices round-trip to server.
        match self
            .inner
            .choose_targets(view, spell, valid_targets, min_targets, max_targets)
        {
            ChoiceResult::Ok(targets) => {
                let choice_indices: Vec<usize> = if targets.is_empty() {
                    vec![valid_targets.len()] // "none" option
                } else {
                    targets
                        .iter()
                        .filter_map(|&t| valid_targets.iter().position(|&vt| vt == t))
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(targets)
            }
            other => other,
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_mana_sources_to_pay(view, cost, available_sources) {
            ChoiceResult::Ok(sources) => {
                let choice_indices: Vec<usize> = if sources.is_empty() {
                    vec![available_sources.len()]
                } else {
                    sources
                        .iter()
                        .filter_map(|&s| available_sources.iter().position(|&as_| as_ == s))
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(sources)
            }
            other => other,
        }
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_attackers(view, available_creatures) {
            ChoiceResult::Ok(attackers) => {
                // Index 0 means "done selecting" / no attackers
                // Index N means attacker at position N-1 in available_creatures
                let choice_indices: Vec<usize> = if attackers.is_empty() {
                    vec![0]
                } else {
                    attackers
                        .iter()
                        .filter_map(|&a| available_creatures.iter().position(|&ac| ac == a).map(|i| i + 1))
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(attackers)
            }
            other => other,
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_blockers(view, available_blockers, attackers) {
            ChoiceResult::Ok(blocks) => {
                // Index 0 means "done selecting" / no blockers
                // For each block, encode as blocker_idx * num_attackers + attacker_idx + 1
                let choice_indices: Vec<usize> = if blocks.is_empty() {
                    vec![0]
                } else {
                    blocks
                        .iter()
                        .filter_map(|&(blocker, attacker)| {
                            let blocker_idx = available_blockers.iter().position(|&b| b == blocker)?;
                            let attacker_idx = attackers.iter().position(|&a| a == attacker)?;
                            Some(blocker_idx * attackers.len() + attacker_idx + 1)
                        })
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(blocks)
            }
            other => other,
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_damage_assignment_order(view, attacker, blockers) {
            ChoiceResult::Ok(order) => {
                let choice_indices: Vec<usize> = if order.is_empty() {
                    vec![0]
                } else {
                    order
                        .iter()
                        .filter_map(|&b| blockers.iter().position(|&bl| bl == b))
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(order)
            }
            other => other,
        }
    }

    fn choose_blocker_for_lethal_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        killable_blockers: &[(CardId, i32)],
        remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        // SMART damage assignment (mtg-418). Server pre-sends the matching
        // ChoiceRequest of type LethalDamageAssignment; we delegate to the inner
        // controller, then submit BOTH the index AND the chosen CardId so the
        // opponent's shadow game can resolve the correct blocker even if its
        // killable list ordering differs from ours.
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self
            .inner
            .choose_blocker_for_lethal_damage(view, attacker, killable_blockers, remaining_power)
        {
            ChoiceResult::Ok(blocker_id) => {
                let idx = killable_blockers
                    .iter()
                    .position(|(id, _)| *id == blocker_id)
                    .unwrap_or(0);
                self.submit_damage_choice_to_server(vec![idx], blocker_id, view);
                ChoiceResult::Ok(blocker_id)
            }
            other => other,
        }
    }

    fn choose_blocker_for_remaining_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        remaining_blockers: &[CardId],
        remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        // Mirror of choose_blocker_for_lethal_damage above (mtg-418).
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self
            .inner
            .choose_blocker_for_remaining_damage(view, attacker, remaining_blockers, remaining_damage)
        {
            ChoiceResult::Ok(blocker_id) => {
                let idx = remaining_blockers.iter().position(|id| *id == blocker_id).unwrap_or(0);
                self.submit_damage_choice_to_server(vec![idx], blocker_id, view);
                ChoiceResult::Ok(blocker_id)
            }
            other => other,
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        // BEHAVIORAL IDENTITY: Use locally-computed hand and count (same as native).
        // The sync_callback has already processed reveals, so local state MUST match
        // server. We validate they match — any mismatch is FATAL DESYNC.

        // FATAL validation: verify local state matches server's.
        // Per NETWORK_ARCHITECTURE.md: desync is ALWAYS fatal, never papered over.
        {
            let server_option_count = self.get_server_option_count();
            if server_option_count > 0 && server_option_count != hand.len() {
                let msg = format!(
                    "FATAL DESYNC: Local hand size ({}) != server option count ({})",
                    hand.len(),
                    server_option_count,
                );
                log::error!("{}", msg);
                return ChoiceResult::Error(msg);
            }

            let server_discard_count = self.get_server_discard_count();
            if let Some(server_count) = server_discard_count {
                if server_count != count {
                    let msg = format!(
                        "FATAL DESYNC: Local discard count ({}) != server discard count ({})",
                        count, server_count,
                    );
                    log::error!("{}", msg);
                    return ChoiceResult::Error(msg);
                }
            }
        }

        match self.inner.choose_cards_to_discard(view, hand, count) {
            ChoiceResult::Ok(discards) => {
                let choice_indices: Vec<usize> = if discards.is_empty() {
                    vec![hand.len()]
                } else {
                    discards
                        .iter()
                        .filter_map(|&c| hand.iter().position(|&h| h == c))
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(discards)
            }
            other => other,
        }
    }

    fn choose_scry_order(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::controller::ScryDecision> {
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_scry_order(view, revealed) {
            ChoiceResult::Ok(decision) => {
                // Encode bottom pile as positions in revealed list, in placement order
                // (first index → deepest bottom). Inner-controller decision.bottom is
                // already in that placement order, so map each CardId → revealed index.
                let mut choice_indices: Vec<usize> = Vec::with_capacity(decision.bottom.len());
                for &card_id in &decision.bottom {
                    if let Some(pos) = revealed.iter().position(|&c| c == card_id) {
                        choice_indices.push(pos);
                    } else {
                        let msg = format!(
                            "WasmNetworkLocalController::choose_scry_order: inner returned bottom card {:?} not in revealed list",
                            card_id
                        );
                        log::error!("{}", msg);
                        return ChoiceResult::Error(msg);
                    }
                }
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(decision)
            }
            other => other,
        }
    }

    fn choose_surveil(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::controller::SurveilDecision> {
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_surveil(view, revealed) {
            ChoiceResult::Ok(decision) => {
                // Encode graveyard pile as positions in revealed list, in placement order
                // (first index → deepest in graveyard pile).
                let mut choice_indices: Vec<usize> = Vec::with_capacity(decision.graveyard.len());
                for &card_id in &decision.graveyard {
                    if let Some(pos) = revealed.iter().position(|&c| c == card_id) {
                        choice_indices.push(pos);
                    } else {
                        let msg = format!(
                            "WasmNetworkLocalController::choose_surveil: inner returned graveyard card {:?} not in revealed list",
                            card_id
                        );
                        log::error!("{}", msg);
                        return ChoiceResult::Error(msg);
                    }
                }
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(decision)
            }
            other => other,
        }
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_from_library(view, valid_cards) {
            ChoiceResult::Ok(choice) => {
                // Server LibrarySearchByName protocol is 1-based: 0=decline, 1=first card, 2=second card, etc.
                let choice_index = match choice {
                    None => 0,            // 0 = decline
                    Some(idx) => idx + 1, // 1-based: first card = 1
                };
                self.submit_choice_to_server(vec![choice_index], view);
                ChoiceResult::Ok(choice)
            }
            other => other,
        }
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self
            .inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
        {
            ChoiceResult::Ok(sacrifices) => {
                let choice_indices: Vec<usize> = if sacrifices.is_empty() {
                    vec![valid_permanents.len()] // "none" option
                } else {
                    sacrifices
                        .iter()
                        .filter_map(|&s| valid_permanents.iter().position(|&vp| vp == s))
                        .collect()
                };
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(sacrifices)
            }
            other => other,
        }
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self
            .inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents)
        {
            ChoiceResult::Ok(stay_tapped) => {
                let choice_indices: Vec<usize> = stay_tapped
                    .iter()
                    .filter_map(|s| may_not_untap_permanents.iter().position(|p| p == s))
                    .collect();
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(stay_tapped)
            }
            other => other,
        }
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Check if ChoiceRequest is ready
        if self.check_choice_request_ready().is_none() {
            if self.has_pending_submission() {
                if self.check_and_clear_ack() {
                    return ChoiceResult::NeedInput(waiting_for_server_context());
                }
                return ChoiceResult::NeedInput(waiting_for_ack_context());
            }
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self
            .inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
        {
            ChoiceResult::Ok(modes) => {
                let choice_indices: Vec<usize> = modes.iter().copied().collect();
                self.submit_choice_to_server(choice_indices, view);
                ChoiceResult::Ok(modes)
            }
            other => other,
        }
    }

    fn prepare_for_priority_choice(&mut self) -> bool {
        // In WASM network mode, we can't block like the native controller does.
        // The native controller blocks on MVar until ChoiceRequest arrives,
        // guaranteeing all preceding CardRevealed messages are buffered.
        //
        // For WASM, we check if a ChoiceRequest is available. If yes, all
        // preceding CardRevealed messages are guaranteed to be in pending_reveals
        // (WebSocket delivers messages in order, and JS processes them sequentially).
        // The sync_callback (called by sync_to_action after this returns) will
        // drain and process them before abilities are computed.
        //
        // If no ChoiceRequest is available, we return true anyway - the controller's
        // choose_spell_ability_to_play will return NeedInput, causing the game loop
        // to exit and wait for more network data. This is correct behavior.
        true
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.inner.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.inner.on_game_end(view, won);
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Network
    }
}
