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
                log::info!(
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

        let state_hash = if client.is_network_debug() {
            let hash = crate::game::compute_view_hash(view);
            // Debug: log each field used in compute_view_hash so we can compare with server
            use crate::core::PlayerId;
            let local_action_count = view.action_count() as u64;
            log::info!(
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
            // Always dump last actions in debug mode for comparison with server
            if action_count != local_action_count {
                log::warn!(
                    "WASM_HASH_DEBUG: ACTION COUNT MISMATCH! server={} local={} (diff={})\nWASM last 15 actions:\n{}",
                    action_count,
                    local_action_count,
                    action_count as i64 - local_action_count as i64,
                    view.format_last_n_actions(15),
                );
            } else {
                log::info!("WASM_ACTION_DUMP: last 30 actions:\n{}", view.format_last_n_actions(30),);
            }
            Some(hash)
        } else {
            None
        };

        // submit_choice internally tracks the sequence and consumes the ChoiceRequest
        client.submit_choice(choice_indices, action_count, state_hash);
    }
}

impl<C: PlayerController> PlayerController for WasmNetworkLocalController<C> {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
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
                log::info!(
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

        match self.inner.choose_targets(view, spell, valid_targets) {
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
