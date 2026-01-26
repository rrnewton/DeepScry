//! Network-aware local controller wrapper
//!
//! This wraps any PlayerController and sends choices to the server after each decision.
//!
//! ## Architecture (Pre-Choice Hook Design)
//!
//! The pre-choice hook in GameLoop handles all network message processing:
//! - Blocks on the message channel
//! - Processes CardRevealed messages to update GameState
//! - Returns ChoiceRequest/OpponentChoice signals
//!
//! This controller simply:
//! 1. Delegates to the wrapped inner controller
//! 2. Sends the choice to the server via client_tx
//!
//! It does NOT:
//! - Block on message_rx (the hook does this)
//! - Process CardRevealed messages (the hook does this)
//! - Wait for ChoiceAccepted (the hook handles this on next choice)

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::network::client::{ChoiceAcceptedInfo, LocalChoiceInfo, SharedNetworkState};
use crate::network::protocol::ClientMessage;
use crate::network::ClientMessageSender;
use smallvec::SmallVec;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// A choice made by the local player, to be sent to the server
#[derive(Debug, Clone)]
pub struct LocalChoice {
    /// The choice indices selected (multiple for attackers/blockers/discard)
    pub choice_indices: Vec<usize>,
    /// Human-readable description
    pub description: String,
    /// Action count (undo log position) at the time of choice
    pub action_count: u64,
    /// Last N actions from the undo log (for sync debugging)
    pub last_actions: Option<String>,
    /// Client's computed state hash (for server validation in debug mode)
    pub client_state_hash: Option<u64>,
    /// Debug synchronization info (only in network debug mode)
    pub debug_info: Option<super::DebugSyncInfo>,
}

/// Choice information returned from get_choice_info
#[derive(Debug)]
struct ChoiceInfoResult {
    choice_seq: u32,
    server_action_count: u64,
    /// Server's authoritative abilities for Priority choices (eliminates race conditions)
    abilities: Option<Vec<Option<SpellAbility>>>,
    /// Server's unique card names for LibrarySearchByName choices
    /// Used when client can't compute valid_cards due to hidden card identities
    library_search_names: Option<Vec<String>>,
    /// Server's count of cards for each unique name (enables instance selection)
    /// Same length as library_search_names
    #[allow(dead_code)] // Kept for protocol compatibility, may be used in future
    library_search_counts: Option<Vec<usize>>,
}

/// A controller that wraps a local controller and sends choices to the server.
///
/// This is used on the client side for our player. In the MVar architecture:
/// 1. Waits for ChoiceRequest from MVar (contains choice_seq)
/// 2. Delegates to the inner controller
/// 3. Sends the choice to the server
///
/// Supports two modes:
/// - Legacy: Uses Rc<Cell<u32>> for choice_seq (with pre-choice hook)
/// - MVar: Uses SharedNetworkState for choice info
pub struct NetworkLocalController<C: PlayerController> {
    /// The wrapped local controller
    inner: C,
    /// Channel to send client messages (choices) to WebSocket writer
    client_tx: ClientMessageSender,
    /// Network debug mode: include action log info in choices for sync validation
    network_debug: bool,
    /// Our player ID (for MVar architecture validation)
    _our_player_id: Option<PlayerId>,
    /// Shared network state (MVar architecture) - takes precedence if set
    shared_state: Option<Arc<SharedNetworkState>>,
    /// Legacy: Shared choice sequence number (pre-choice hook updates it, controller reads it)
    choice_seq: Rc<Cell<u32>>,
}

impl<C: PlayerController> NetworkLocalController<C> {
    /// Create a new network local controller (legacy mode with pre-choice hook)
    ///
    /// # Arguments
    /// * `inner` - The actual controller to delegate choices to
    /// * `client_tx` - Channel to send client messages to WebSocket writer
    /// * `choice_seq` - Shared choice sequence number (hook updates it, we read it)
    pub fn new(inner: C, client_tx: ClientMessageSender, choice_seq: Rc<Cell<u32>>) -> Self {
        Self {
            inner,
            client_tx,
            network_debug: false,
            _our_player_id: None,
            shared_state: None,
            choice_seq,
        }
    }

    /// Create a new network local controller (MVar architecture)
    ///
    /// # Arguments
    /// * `inner` - The actual controller to delegate choices to
    /// * `client_tx` - Channel to send client messages to WebSocket writer
    /// * `shared_state` - Shared network state for MVar-based choice synchronization
    /// * `our_player_id` - Our player ID for validation
    pub fn new_with_shared_state(
        inner: C,
        client_tx: ClientMessageSender,
        shared_state: Arc<SharedNetworkState>,
        player_id: PlayerId,
    ) -> Self {
        Self {
            inner,
            client_tx,
            network_debug: false,
            _our_player_id: Some(player_id),
            shared_state: Some(shared_state),
            choice_seq: Rc::new(Cell::new(0)), // Not used in MVar mode
        }
    }

    /// Enable network debug mode for action log transmission
    pub fn with_network_debug(mut self, enabled: bool) -> Self {
        self.network_debug = enabled;
        self
    }

    /// Get access to the inner controller
    pub fn inner(&self) -> &C {
        &self.inner
    }

    /// Get mutable access to the inner controller
    pub fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Get choice info for the current choice
    ///
    /// In MVar mode: Takes ChoiceRequest from local_choice_mvar (blocking if needed)
    /// In legacy mode: Returns the shared choice_seq from pre-choice hook
    ///
    /// Returns None if the game should exit (GameEnded/Error received)
    /// Returns Some(ChoiceInfoResult) on success
    fn get_choice_info(&self) -> Option<ChoiceInfoResult> {
        if let Some(ref state) = self.shared_state {
            // MVar mode: take from LOCAL choice MVar (dedicated for this controller)
            match state.take_local_choice() {
                Some(LocalChoiceInfo::Request {
                    choice_seq,
                    action_count,
                    abilities,
                    library_search_names,
                    library_search_counts,
                }) => {
                    log::debug!(
                        "NetworkLocalController: got ChoiceRequest seq={} action={} abilities={} lib_search={}",
                        choice_seq,
                        action_count,
                        abilities.as_ref().map(|a| a.len()).unwrap_or(0),
                        library_search_names.as_ref().map(|n| n.len()).unwrap_or(0)
                    );
                    Some(ChoiceInfoResult {
                        choice_seq,
                        server_action_count: action_count,
                        abilities,
                        library_search_names,
                        library_search_counts,
                    })
                }
                Some(LocalChoiceInfo::Exit { winner }) => {
                    log::info!("NetworkLocalController: game ended, winner={:?}", winner);
                    None
                }
                Some(LocalChoiceInfo::Error { message }) => {
                    log::error!("NetworkLocalController: error from server: {}", message);
                    None
                }
                None => {
                    log::debug!("NetworkLocalController: MVar returned None (exit signaled)");
                    None
                }
            }
        } else {
            // Legacy mode: use the pre-populated choice_seq, action_count 0 (not validated)
            Some(ChoiceInfoResult {
                choice_seq: self.choice_seq.get(),
                server_action_count: 0,
                abilities: None,
                library_search_names: None,
                library_search_counts: None,
            })
        }
    }

    /// Verify client and server action counts match (informational)
    ///
    /// Note: This check can show false positives due to timing:
    /// - The server sends ChoiceRequest AFTER processing reveals
    /// - The client receives ChoiceRequest but sync_callback hasn't run yet
    /// - This check sees the mismatch before sync_callback processes reveals
    ///
    /// The server is authoritative and will detect real desyncs when the client
    /// submits its choice. This client-side check is for early warning/debugging.
    ///
    /// # Arguments
    /// * `view` - The current game state view (for client's action count)
    /// * `server_action_count` - Action count from server's ChoiceRequest
    #[inline]
    fn verify_action_count_sync(&self, view: &GameStateView, server_action_count: u64) {
        let client_action_count = view.action_count() as u64;
        if client_action_count != server_action_count && server_action_count > 0 {
            // Log the discrepancy for debugging
            // Note: This can be a false positive if sync_callback hasn't run yet
            // with the new server_action_count. The server will catch real desyncs.
            log::warn!(
                "NetworkLocalController: action_count mismatch (may be timing issue)! \
                 client={} server={} (diff={})",
                client_action_count,
                server_action_count,
                server_action_count as i64 - client_action_count as i64
            );
        }
    }

    /// Send a choice to the server (fire-and-forget, no waiting for ack)
    ///
    /// For Priority choices, `spell_ability` should contain the actual ability chosen.
    /// This is more robust than relying on indices, which can mismatch if client/server
    /// have different views of available options due to sync drift.
    fn send_choice(
        &self,
        choice_seq: u32,
        choice_indices: Vec<usize>,
        action_count: u64,
        client_state_hash: Option<u64>,
        debug_info: Option<super::DebugSyncInfo>,
        spell_ability: Option<SpellAbility>,
    ) {
        let client_msg = ClientMessage::SubmitChoice {
            choice_seq,
            choice_indices,
            action_count,
            timestamp_ms: 0,
            client_state_hash,
            debug_info,
            spell_ability,
        };

        // Fire and forget
        let _ = self.client_tx.send(client_msg);
    }

    /// Get debug fields for a choice when network_debug is enabled
    fn get_debug_fields(&self, view: &GameStateView) -> (Option<u64>, Option<super::DebugSyncInfo>) {
        if self.network_debug {
            let client_state_hash = Some(crate::game::compute_view_hash(view));
            // Note: rng_hash is None here because controllers don't have direct RNG access.
            // Full RNG verification would require GameLoop to pass the hash through.
            // Pass player_id to include hand CardIds for desync detection.
            let debug_info = Some(crate::game::build_debug_sync_info(
                view,
                10,
                None,
                Some(view.player_id()),
            ));
            (client_state_hash, debug_info)
        } else {
            (None, None)
        }
    }

    /// Helper to wrap a choice result and send to server (legacy mode)
    #[allow(dead_code)]
    fn handle_choice<T>(
        &self,
        view: &GameStateView,
        choice_seq: u32,
        result: ChoiceResult<T>,
        indices: Vec<usize>,
    ) -> ChoiceResult<T> {
        if let ChoiceResult::Ok(_) = &result {
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, view.action_count() as u64, hash, debug, None);
        }
        result
    }
}

impl<C: PlayerController> PlayerController for NetworkLocalController<C> {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Get choice info from MVar (blocks in MVar mode)
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame, // Game ended
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;

        // Log any action count discrepancy (informational)
        self.verify_action_count_sync(view, server_action_count);

        // Use server's authoritative abilities if available (eliminates race condition)
        // The server sends abilities computed with full card knowledge.
        // The client may compute abilities before CardRevealed messages arrive,
        // causing desync. Using server abilities fixes this.
        let effective_available: Vec<SpellAbility> = if let Some(ref server_abilities) = info.abilities {
            // Extract non-None abilities from server list (index 0 is "Pass")
            server_abilities
                .iter()
                .skip(1) // Skip "Pass priority" placeholder
                .filter_map(|opt| opt.clone())
                .collect()
        } else {
            // Fallback to locally-computed abilities (legacy path)
            available.to_vec()
        };

        // Log if server abilities differ from local
        if info.abilities.is_some() && effective_available.len() != available.len() {
            log::info!(
                "NetworkLocalController: using server abilities ({}) vs local ({})",
                effective_available.len(),
                available.len()
            );
        }

        let result = self.inner.choose_spell_ability_to_play(view, &effective_available);

        // Convert result to index and send
        // Use SERVER's action_count - this is a correlation ID for the ChoiceRequest
        // Also include the actual SpellAbility for robust choice matching (mtg-wsl8g)
        if let ChoiceResult::Ok(ref choice) = result {
            let idx = match choice {
                None => 0, // Pass
                Some(ability) => effective_available
                    .iter()
                    .position(|a| a == ability)
                    .map(|i| i + 1)
                    .unwrap_or(0),
            };
            let (hash, debug) = self.get_debug_fields(view);
            // Pass the chosen spell ability so server can match by CardId if indices don't match
            self.send_choice(choice_seq, vec![idx], server_action_count, hash, debug, choice.clone());
        }

        result
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self.inner.choose_targets(view, spell, valid_targets);

        if let ChoiceResult::Ok(ref targets) = result {
            let indices: Vec<usize> = targets
                .iter()
                .filter_map(|t| valid_targets.iter().position(|v| v == t))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self.inner.choose_mana_sources_to_pay(view, cost, available_sources);

        if let ChoiceResult::Ok(ref sources) = result {
            let indices: Vec<usize> = sources
                .iter()
                .filter_map(|s| available_sources.iter().position(|a| a == s))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self.inner.choose_attackers(view, available_creatures);

        if let ChoiceResult::Ok(ref attackers) = result {
            // Index 0 = pass, index N = creature N-1
            let indices: Vec<usize> = attackers
                .iter()
                .filter_map(|a| available_creatures.iter().position(|c| c == a).map(|i| i + 1))
                .collect();
            let indices = if indices.is_empty() { vec![0] } else { indices };
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self.inner.choose_blockers(view, available_blockers, attackers);

        if let ChoiceResult::Ok(ref blocks) = result {
            // Index 0 = pass, index N = (blocker_idx, attacker_idx) pair + 1
            let indices: Vec<usize> = blocks
                .iter()
                .filter_map(|(blocker, attacker)| {
                    let blocker_idx = available_blockers.iter().position(|b| b == blocker)?;
                    let attacker_idx = attackers.iter().position(|a| a == attacker)?;
                    Some(blocker_idx * attackers.len() + attacker_idx + 1)
                })
                .collect();
            let indices = if indices.is_empty() { vec![0] } else { indices };
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self.inner.choose_damage_assignment_order(view, attacker, blockers);

        if let ChoiceResult::Ok(ref order) = result {
            let indices: Vec<usize> = order
                .iter()
                .filter_map(|b| blockers.iter().position(|bl| bl == b))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self.inner.choose_cards_to_discard(view, hand, count);

        if let ChoiceResult::Ok(ref discards) = result {
            let indices: Vec<usize> = discards
                .iter()
                .filter_map(|d| hand.iter().position(|h| h == d))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_card_names: &[&str]) -> ChoiceResult<Option<usize>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        // Check if server provided library_search_names (late-binding architecture)
        // In this case, client can't compute valid_card_names locally due to hidden card identities
        log::info!(
            "[NetworkLocalController] choose_from_library: library_search_names={:?}, valid_card_names.len={}",
            info.library_search_names.as_ref().map(|n| n.len()),
            valid_card_names.len()
        );

        // Build the names list for the inner controller
        // Prefer server-provided names if available (for hidden library cards)
        let names_for_inner: Vec<&str> = if let Some(ref names) = info.library_search_names {
            names.iter().map(|s| s.as_str()).collect()
        } else {
            valid_card_names.to_vec()
        };

        // Call inner controller with names - it returns an index directly
        let inner_result = self.inner.choose_from_library(view, &names_for_inner);

        // Map inner result to protocol format
        let (name_idx, instance_idx) = match &inner_result {
            ChoiceResult::Ok(Some(index)) => {
                // Protocol uses 1-based indexing (0 = decline)
                (*index + 1, 0)
            }
            ChoiceResult::Ok(None) => (0, 0), // Declined
            ChoiceResult::ExitGame => return ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => {
                log::error!("[NetworkLocalController] inner choose_from_library error: {}", e);
                if names_for_inner.is_empty() {
                    (0, 0)
                } else {
                    (1, 0)
                } // Fallback
            }
            ChoiceResult::UndoRequest(_) | ChoiceResult::NeedInput(_) => {
                log::warn!("[NetworkLocalController] unexpected UndoRequest/NeedInput from inner choose_from_library");
                if names_for_inner.is_empty() {
                    (0, 0)
                } else {
                    (1, 0)
                } // Fallback
            }
        };

        log::debug!(
            "[NetworkLocalController] choose_from_library: names_for_inner.len={}, name_idx={}, instance_idx={}",
            names_for_inner.len(),
            name_idx,
            instance_idx
        );
        let (hash, debug) = self.get_debug_fields(view);
        // Send [name_idx, instance_idx] - server will use both to pick the specific CardId
        self.send_choice(
            choice_seq,
            vec![name_idx, instance_idx],
            server_action_count,
            hash,
            debug,
            None,
        );

        // Wait for ChoiceAccepted to confirm the server processed our choice
        // The server returns the index it used, which should match what we sent
        if let Some(ref state) = self.shared_state {
            match state.take_choice_accepted_for_seq(choice_seq) {
                Some(ChoiceAcceptedInfo::Accepted { .. }) => {
                    log::info!("[NetworkLocalController] choose_from_library: ChoiceAccepted received");
                    // Return the index we chose (0-based for the trait interface)
                    return inner_result;
                }
                Some(ChoiceAcceptedInfo::Exit { .. }) => {
                    log::info!(
                        "[NetworkLocalController] choose_from_library: game ended while waiting for ChoiceAccepted"
                    );
                    return ChoiceResult::ExitGame;
                }
                Some(ChoiceAcceptedInfo::Error { message }) => {
                    log::error!(
                        "[NetworkLocalController] choose_from_library: error while waiting for ChoiceAccepted: {}",
                        message
                    );
                    return ChoiceResult::ExitGame;
                }
                None => {
                    log::warn!("[NetworkLocalController] choose_from_library: take_choice_accepted returned None (exit signaled)");
                    return ChoiceResult::ExitGame;
                }
            }
        }

        inner_result
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self
            .inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description);

        if let ChoiceResult::Ok(ref sacrifices) = result {
            let indices: Vec<usize> = sacrifices
                .iter()
                .filter_map(|s| valid_permanents.iter().position(|p| p == s))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self
            .inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents);

        if let ChoiceResult::Ok(ref stay_tapped) = result {
            let indices: Vec<usize> = stay_tapped
                .iter()
                .filter_map(|s| may_not_untap_permanents.iter().position(|p| p == s))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
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
        let info = match self.get_choice_info() {
            Some(info) => info,
            None => return ChoiceResult::ExitGame,
        };
        let choice_seq = info.choice_seq;
        let server_action_count = info.server_action_count;
        self.verify_action_count_sync(view, server_action_count);

        let result = self
            .inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat);

        if let ChoiceResult::Ok(ref modes) = result {
            let indices: Vec<usize> = modes.iter().copied().collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(choice_seq, indices, server_action_count, hash, debug, None);
        }

        result
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.inner.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.inner.on_game_end(view, won);
    }

    fn get_controller_type(&self) -> ControllerType {
        // Return Network type so GameLoop knows this is a network-controlled local player
        ControllerType::Network
    }
}

// Legacy types for backward compatibility (can be removed later)

/// A buffered card reveal waiting to be processed
#[derive(Debug, Clone)]
pub struct BufferedReveal {
    pub owner: PlayerId,
    pub card: crate::network::protocol::CardReveal,
    pub reason: crate::network::protocol::RevealReason,
}

/// Shared buffer for pending reveals (legacy - used by old architecture)
pub type PendingReveals = std::sync::Arc<std::sync::Mutex<Vec<BufferedReveal>>>;

/// A bundled card reveal (legacy)
#[derive(Debug, Clone)]
pub struct BundledReveal {
    pub owner: PlayerId,
    pub card_id: CardId,
    pub card_name: String,
    pub reason: crate::network::protocol::RevealReason,
}
