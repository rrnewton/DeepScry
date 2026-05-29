//! Network controller for remote player communication
//!
//! This module provides `NetworkController`, a `PlayerController` implementation
//! that proxies player decisions over a network connection. It's used server-side
//! to represent remote players connected via WebSocket.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::network::protocol::ChoiceType;
use crate::undo::GameAction;
use crate::zones::Zone;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// CARD REVEAL INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Information about a card that was revealed since the player's last choice
///
/// This captures card movements from hidden zones (library) to visible zones.
/// The server uses this to send `CardRevealed` messages to the client before
/// sending the next `ChoiceRequest`.
#[derive(Debug, Clone)]
pub struct CardRevealInfo {
    /// The card that was revealed
    pub card_id: CardId,
    /// Who owns the card
    pub owner: PlayerId,
    /// Zone the card moved from (typically Library)
    pub from_zone: Zone,
    /// Zone the card moved to
    pub to_zone: Zone,
}

// ═══════════════════════════════════════════════════════════════════════════
// CHANNEL TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Request sent to the network handler for forwarding to client
#[derive(Debug, Clone)]
pub struct ChoiceRequest {
    /// Sequence number for correlation
    pub choice_seq: u32,
    /// Type of choice being requested
    pub choice_type: ChoiceType,
    /// Human-readable options
    pub options: Vec<String>,
    /// Game state hash (excluding hidden info)
    pub state_hash: u64,
    /// Action count at this choice point (undo log position)
    /// This is the source of truth for synchronization
    pub action_count: u64,
    /// Cards revealed since this player's last choice
    ///
    /// The server should send `CardRevealed` messages for these before
    /// sending the `ChoiceRequest` to the client.
    pub reveals: Vec<CardRevealInfo>,
    /// Debug synchronization info (only when network_debug is enabled)
    pub debug_info: Option<super::DebugSyncInfo>,
    /// For Priority choices, the actual SpellAbility for each option.
    ///
    /// Index 0 is "Pass priority" (None), indices 1+ are the abilities.
    /// This allows the handler to look up the chosen ability directly
    /// without a separate channel round-trip. (mtg-e66iz channel consolidation)
    pub abilities: Option<Vec<Option<crate::core::SpellAbility>>>,
    /// For LibrarySearchByName choices, ALL CardIds in flat order.
    ///
    /// Cards are ordered by name (matching unique_names order), with all
    /// instances of each name grouped together. Combined with name_counts,
    /// allows the coordinator to decode (name_index, instance_index).
    pub library_search_cards: Option<Vec<CardId>>,

    /// Library reorders that must be broadcast to BOTH clients before the
    /// `ChoiceRequest` itself.
    ///
    /// Queued by the engine (`scry_cards`, `surveil_cards`) when a
    /// hidden-info-dependent heuristic moves cards in a player's library on
    /// the server. Each entry is `(player, top_to_bottom_order)`. The
    /// coordinator drains this list and emits a
    /// `ServerMessage::LibraryReordered` to every connected client so each
    /// shadow game can re-sync its library before running ability enumeration.
    ///
    /// Always empty for non-network controllers and for client-built
    /// requests. See mtg-ced6d1 (Cycle/Mountaincycling FATAL DESYNC).
    pub library_reorders: Vec<(PlayerId, Vec<CardId>)>,
}

/// Response received from the network handler
#[derive(Debug, Clone)]
pub struct ChoiceResponse {
    /// Sequence number (must match request)
    pub choice_seq: u32,
    /// Indices of the chosen options
    ///
    /// For single-select choices, this is a 1-element vec.
    /// For multi-select choices (attackers, blockers), contains all selected indices.
    pub choice_indices: Vec<usize>,
    /// The actual spell ability chosen by the client (for Priority choices)
    ///
    /// Used for VALIDATION ONLY - to detect desync early. The canonical choice
    /// is always determined by index. If the index-based lookup doesn't match
    /// this spell_ability, it indicates a desync and is a FATAL ERROR.
    /// See docs/NETWORK_ARCHITECTURE.md for the "desync is always fatal" principle.
    pub spell_ability: Option<crate::core::SpellAbility>,
    /// Actual target CardIds for target choices
    ///
    /// Used to synchronize opponent's shadow game - the actual CardIds are sent
    /// so the opponent doesn't need to rely on index-based lookup which can fail
    /// if their valid_targets list differs from ours.
    pub target_card_ids: Option<Vec<crate::core::CardId>>,
}

/// Result of request_choice including both indices and optional spell_ability
struct NetworkChoiceResult {
    indices: Vec<usize>,
    spell_ability: Option<crate::core::SpellAbility>,
    target_card_ids: Option<Vec<crate::core::CardId>>,
}

/// Error that can occur during network communication
#[derive(Debug, Clone)]
pub enum NetworkError {
    /// Client disconnected
    Disconnected,
    /// Request timed out
    Timeout,
    /// Sequence number mismatch
    SequenceMismatch { expected: u32, got: u32 },
    /// Invalid choice index
    InvalidChoice { max: usize, got: usize },
    /// Channel error
    ChannelError(String),
    /// Client/server desync - FATAL error indicating state divergence
    /// This should NEVER be recovered from - it indicates a fundamental bug
    DesyncError(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Disconnected => write!(f, "Client disconnected"),
            NetworkError::Timeout => write!(f, "Request timed out"),
            NetworkError::SequenceMismatch { expected, got } => {
                write!(f, "Sequence mismatch: expected {}, got {}", expected, got)
            }
            NetworkError::InvalidChoice { max, got } => {
                write!(f, "Invalid choice index: {} (max {})", got, max)
            }
            NetworkError::ChannelError(msg) => write!(f, "Channel error: {}", msg),
            NetworkError::DesyncError(msg) => write!(f, "FATAL DESYNC: {}", msg),
        }
    }
}

impl std::error::Error for NetworkError {}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK CONTROLLER
// ═══════════════════════════════════════════════════════════════════════════

/// Controller that communicates with a remote player over network
///
/// This is used SERVER-SIDE to represent a remote player. When the game
/// needs a decision from this player, the controller:
/// 1. Builds an options list matching the local display format
/// 2. Computes the network state hash (public info only)
/// 3. Sends a ChoiceRequest over the channel
/// 4. Awaits the ChoiceResponse
/// 5. Returns the selected option to the game loop
///
/// The actual WebSocket communication is handled by a separate task that
/// reads from request_rx and writes to response_tx.
pub struct NetworkController {
    /// Player ID this controller represents
    player_id: PlayerId,
    /// Channel to send choice requests
    request_tx: mpsc::Sender<ChoiceRequest>,
    /// Channel to receive choice responses
    response_rx: mpsc::Receiver<ChoiceResponse>,
    /// Current choice sequence number
    choice_seq: u32,
    /// Shared index into the undo_log where reveals were last sent.
    /// This is shared with the immediate reveal pusher to prevent duplicate reveals.
    /// Initialized to the number of opening hand draw actions (14 for 7+7).
    shared_reveal_index: Arc<AtomicUsize>,
    /// Network debug mode - include debug info in choice requests
    network_debug: bool,
    /// Pending library search CardIds set by game loop before choose_from_library
    ///
    /// The game loop sets this via `set_pending_library_search_card_ids` before
    /// calling `choose_from_library`. This allows the ChoiceRequest to include
    /// the CardIds so the coordinator can resolve the client's name index back
    /// to an authoritative CardId.
    pending_library_search_card_ids: Option<Vec<CardId>>,
}

impl NetworkController {
    /// Create a new network controller with a shared reveal index
    ///
    /// The `shared_reveal_index` is shared with the immediate reveal pusher hook
    /// to ensure both systems track the same position and don't send duplicate reveals.
    pub fn new(
        player_id: PlayerId,
        request_tx: mpsc::Sender<ChoiceRequest>,
        response_rx: mpsc::Receiver<ChoiceResponse>,
        shared_reveal_index: Arc<AtomicUsize>,
    ) -> Self {
        NetworkController {
            player_id,
            request_tx,
            response_rx,
            choice_seq: 0,
            shared_reveal_index,
            network_debug: false,
            pending_library_search_card_ids: None,
        }
    }

    /// Get the shared reveal index for the immediate reveal pusher
    ///
    /// This allows the reveal pusher hook to share the same tracking index.
    pub fn shared_reveal_index(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.shared_reveal_index)
    }

    /// Enable network debug mode
    ///
    /// When enabled, debug synchronization info is included in each choice request.
    pub fn set_network_debug(&mut self, enabled: bool) {
        self.network_debug = enabled;
    }

    /// Send a choice request and wait for response
    ///
    /// This also collects any card reveals since this player's last choice
    /// and includes them in the request, ensuring reveals arrive before the client's
    /// shadow GameLoop needs them (eliminates race conditions with immediate reveals).
    ///
    /// For Priority choices, pass `abilities` containing the SpellAbility for each option.
    /// This allows the handler to look up the chosen ability without a round-trip channel.
    fn request_choice(
        &mut self,
        view: &GameStateView,
        choice_type: ChoiceType,
        options: Vec<String>,
        state_hash: u64,
        abilities: Option<Vec<Option<crate::core::SpellAbility>>>,
        library_search_cards: Option<Vec<CardId>>,
    ) -> Result<NetworkChoiceResult, NetworkError> {
        // Collect reveals since this player's last choice and include them in the request.
        // This ensures reveals are sent BEFORE the ChoiceRequest arrives, eliminating
        // race conditions where the client's shadow GameLoop needs reveals before they arrive.
        //
        // The immediate reveal system (reveal_pusher) coordinates with us via shared_reveal_index
        // to ensure each reveal is only sent once (either here or by the pusher, not both).
        let reveals = self.collect_reveals_since_last_choice(view);

        if !reveals.is_empty() {
            log::debug!(
                "NetworkController {:?}: collected {} reveals for ChoiceRequest (action_count={})",
                self.player_id,
                reveals.len(),
                view.action_count()
            );
            for reveal in &reveals {
                log::debug!(
                    "  Reveal: card {:?} from {:?} to {:?} (owner {:?})",
                    reveal.card_id,
                    reveal.from_zone,
                    reveal.to_zone,
                    reveal.owner
                );
            }
        }

        // Get action count from GameState undo log for synchronization
        let action_count = view.action_count() as u64;

        // Build debug info if network debug mode is enabled
        // Note: rng_hash is None here because controllers don't have direct RNG access.
        // Full RNG verification would require GameLoop to pass the hash through.
        // Pass requesting player ID to include their hand's CardIds for desync detection.
        let debug_info = if self.network_debug {
            // Dump the last 30 actions at each choice point for sync debugging
            log::trace!(
                "SERVER_ACTION_DUMP: {:?} choice_seq={} action_count={}\n{}",
                self.player_id,
                self.choice_seq + 1,
                action_count,
                view.format_last_n_actions(30),
            );
            Some(crate::game::build_debug_sync_info(view, 10, None, Some(self.player_id)))
        } else {
            None
        };

        // Check if this is a LibrarySearchByName BEFORE moving choice_type into request
        // This is used later for validation (LibrarySearchByName has special validation rules)
        let is_library_search = matches!(choice_type, ChoiceType::LibrarySearchByName { .. });

        // Drain any queued library reorders from the engine
        // (e.g., scry/surveil heuristic moved cards). These must be sent to
        // BOTH clients before the ChoiceRequest so their shadow libraries
        // re-sync before ability enumeration. See mtg-ced6d1.
        let library_reorders = view.take_pending_library_reorders();
        if !library_reorders.is_empty() {
            log::debug!(
                "NetworkController {:?}: collected {} library reorder(s) for ChoiceRequest \
                 (action_count={})",
                self.player_id,
                library_reorders.len(),
                action_count
            );
        }

        let request = ChoiceRequest {
            choice_seq: self.choice_seq + 1,
            choice_type,
            options: options.clone(),
            state_hash,
            action_count,
            reveals,
            debug_info,
            abilities,
            library_search_cards,
            library_reorders,
        };

        // Send request
        log::debug!(
            "Server NetworkController {:?}: sending ChoiceRequest #{} (action_count={}, type={:?})",
            self.player_id,
            request.choice_seq,
            request.action_count,
            request.choice_type
        );
        self.request_tx
            .send(request)
            .map_err(|e| NetworkError::ChannelError(e.to_string()))?;

        // Wait for response
        let response = self
            .response_rx
            .recv()
            .map_err(|e| NetworkError::ChannelError(e.to_string()))?;

        // Verify sequence number
        if response.choice_seq != self.choice_seq + 1 {
            return Err(NetworkError::SequenceMismatch {
                expected: self.choice_seq + 1,
                got: response.choice_seq,
            });
        }

        // Validate choice indices - FATAL ERROR on invalid index (desync detection)
        // Per mtg-wsl8g: "Desync is ALWAYS a Fatal Error" - we do NOT paper over desync
        // with recovery hacks like clamping. Instead, we crash with a clear error message.
        //
        // Special case: LibrarySearchByName sends [name_idx+1, instance_idx] where:
        // - name_idx+1 should be validated against options.len() (name selection)
        // - instance_idx is validated separately in choose_from_library (instance within name)
        // Note: is_library_search was computed earlier before choice_type was moved
        for (i, idx) in response.choice_indices.iter().enumerate() {
            // For LibrarySearchByName, only validate the first index (name selection)
            // The second index (instance_idx) can be any value and is validated in choose_from_library
            if is_library_search && i >= 1 {
                continue;
            }
            if *idx >= options.len() {
                // This is a FATAL desync error - the client and server have different views
                // of the available choices. This should NEVER happen with a properly
                // synchronized client. Do NOT recover - crash to expose the bug.
                let error_msg = format!(
                    "DESYNC DETECTED: NetworkController {:?} received invalid choice index {} \
                     (only {} options available). Client sent indices {:?}. \
                     This indicates client/server state divergence - a fundamental bug that \
                     must be fixed, NOT papered over with clamping.",
                    self.player_id,
                    *idx,
                    options.len(),
                    response.choice_indices
                );
                log::error!("{}", error_msg);
                // Return an error rather than clamping - let the caller handle the fatal error
                return Err(NetworkError::DesyncError(error_msg));
            }
        }

        Ok(NetworkChoiceResult {
            indices: response.choice_indices,
            spell_ability: response.spell_ability,
            target_card_ids: response.target_card_ids,
        })
    }

    /// Increment choice sequence after a successful choice
    fn increment_choice_seq(&mut self) {
        self.choice_seq += 1;
    }

    /// Format a SpellAbility for display (matching format_choice_menu)
    fn format_spell_ability(&self, view: &GameStateView, ability: &SpellAbility) -> String {
        match ability {
            SpellAbility::PlayLand { card_id } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Play land: {}", name)
            }
            SpellAbility::CastSpell { card_id } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Cast spell: {}", name)
            }
            SpellAbility::ActivateAbility { card_id, ability_index } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Activate: {} (ability {})", name, ability_index)
            }
            SpellAbility::CastFromExile {
                card_id,
                alternative_cost,
                ..
            } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Cast from exile: {} (for {})", name, alternative_cost)
            }
            SpellAbility::CastFromCommand { card_id, total_cost } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Cast from command zone: {} ({})", name, total_cost)
            }
            SpellAbility::Cycle {
                card_id,
                cost,
                search_type,
            } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                match search_type {
                    Some(land_type) => format!("{}cycling: {} ({})", land_type.as_str(), name, cost),
                    None => format!("Cycle: {} ({})", name, cost),
                }
            }
            SpellAbility::CastFromGraveyard { card_id, .. } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Cast from graveyard: {}", name)
            }
        }
    }

    /// Format a card for display
    fn format_card(&self, view: &GameStateView, card_id: CardId) -> String {
        view.card_name(card_id)
            .unwrap_or_else(|| format!("Card #{}", card_id.as_u32()))
    }

    /// Collect card reveals since this player's last choice
    ///
    /// Per NETWORK_ARCHITECTURE.md, this reads RevealCard actions from the log
    /// instead of inferring reveals from MoveCard actions. The `revealed_to` field
    /// on RevealCard tells us which players should see each reveal.
    ///
    /// A player sees a reveal if:
    /// - revealed_to == All (public zone like battlefield, graveyard, stack)
    /// - revealed_to == Player(id) where id matches this player
    ///
    /// Called by request_choice to bundle reveals with the ChoiceRequest.
    /// Uses the shared_reveal_index to coordinate with the immediate reveal pusher.
    ///
    /// Note: Wildcard is intentional - GameAction has many variants;
    /// we only collect RevealCard actions targeted at this player.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn collect_reveals_since_last_choice(&mut self, view: &GameStateView) -> Vec<CardRevealInfo> {
        use crate::undo::RevealTarget;

        let actions = view.undo_log_actions();
        let mut reveals = Vec::new();
        let total_actions = actions.len();

        // Read the shared index - this may have been updated by the immediate reveal pusher
        let last_reveal_index = self.shared_reveal_index.load(Ordering::Acquire);

        // First pass: build a map of card_id -> to_zone from MoveCard actions
        // This lets us determine the actual destination zone for each revealed card
        // (mtg-ar269 fix: milled cards were incorrectly revealed as Draw instead of Effect)
        let mut card_zones: std::collections::HashMap<CardId, Zone> = std::collections::HashMap::new();
        for action in actions.iter().skip(last_reveal_index) {
            if let GameAction::MoveCard { card_id, to_zone, .. } = action {
                card_zones.insert(*card_id, *to_zone);
            }
        }

        // Scan backwards from the end of the log, but stop at last_reveal_index
        for (rev_idx, action) in actions.iter().rev().enumerate() {
            // Convert reverse index to forward index
            let forward_idx = total_actions.saturating_sub(rev_idx + 1);

            // Stop if we've reached actions that were already handled
            if forward_idx < last_reveal_index {
                break;
            }

            match action {
                // Stop when we hit this player's last choice
                GameAction::ChoicePoint { player_id, .. } if *player_id == self.player_id => {
                    break;
                }
                // Collect RevealCard actions targeted at this player (per NETWORK_ARCHITECTURE.md)
                GameAction::RevealCard {
                    card_id,
                    name,
                    revealed_to,
                    ..
                } => {
                    let should_reveal = match revealed_to {
                        RevealTarget::All => true,
                        RevealTarget::Player(target_id) => *target_id == self.player_id,
                    };

                    // Only include reveals with actual card names (not dummy reveals for opponents)
                    if should_reveal && name.is_some() {
                        // Look up the actual card owner from the game state
                        // CRITICAL: Using self.player_id was WRONG - it caused cards to be
                        // assigned to the wrong player when the reveal was collected by
                        // a different player's controller (mtg-d0jg3 DESYNC fix)
                        let card_owner = view.get_card(*card_id).map(|c| c.owner).unwrap_or(self.player_id); // Fallback to self if card not found

                        // Look up actual destination zone from MoveCard actions
                        // This is critical for determining the correct RevealReason:
                        // - Hand -> Draw (e.g., regular draw, tutored card)
                        // - Graveyard -> Effect (e.g., mill, discard from library)
                        // - Battlefield/Stack -> Played
                        // (mtg-ar269 fix: using placeholder Zone::Hand caused milled cards
                        // to be treated as draws, triggering incorrect "empty library mode")
                        let to_zone = card_zones.get(card_id).copied().unwrap_or(Zone::Hand);

                        reveals.push(CardRevealInfo {
                            card_id: *card_id,
                            owner: card_owner,
                            from_zone: Zone::Library, // Still a placeholder, but less critical
                            to_zone,
                        });
                    }
                }
                _ => {}
            }
        }

        // Update shared index to current position after collecting reveals
        // This tracks our progress through the undo log for reveal collection.
        // The immediate reveal pusher runs in parallel but doesn't update the index,
        // so reveals may be sent twice (once via pusher, once here) - client handles duplicates.
        if !reveals.is_empty() {
            self.shared_reveal_index.store(total_actions, Ordering::Release);
        }

        // Reverse to get chronological order
        reveals.reverse();
        reveals
    }

    /// Compute a network-safe state hash for verification
    ///
    /// Uses `compute_view_hash` to compute a deterministic hash from the view.
    /// This produces identical results on server and client for the same
    /// game state, enabling early sync drift detection.
    fn compute_view_hash(&self, view: &GameStateView) -> u64 {
        crate::game::compute_view_hash(view)
    }
}

impl PlayerController for NetworkController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Build options list
        let mut options = vec!["Pass priority".to_string()];
        for ability in available {
            options.push(self.format_spell_ability(view, ability));
        }

        // Build abilities list for handler to look up chosen ability directly
        // Index 0 is "Pass priority" (None), indices 1+ are the actual abilities
        // This eliminates the need for a separate ability_rx channel (mtg-e66iz)
        let abilities: Vec<Option<SpellAbility>> = std::iter::once(None)
            .chain(available.iter().map(|a| Some(a.clone())))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request and get response
        let choice_type = ChoiceType::Priority {
            available_count: available.len(),
        };

        match self.request_choice(view, choice_type, options, state_hash, Some(abilities), None) {
            Ok(result) if result.indices.first() == Some(&0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(None) // Pass priority
            }
            Ok(result) => {
                self.increment_choice_seq();

                // Canonical: always use index-based lookup
                let idx = result.indices.first().copied().unwrap_or(0);
                let ability_idx = idx - 1;
                if ability_idx >= available.len() {
                    return ChoiceResult::Error(format!(
                        "FATAL DESYNC: Invalid ability index {} from network (available: {})",
                        ability_idx,
                        available.len()
                    ));
                }

                let ability = available[ability_idx].clone();

                // VALIDATION ONLY: If spell_ability is present, verify it matches.
                // A mismatch indicates desync - we crash immediately, no recovery.
                // See docs/NETWORK_ARCHITECTURE.md for why desync is always fatal.
                if let Some(ref expected) = result.spell_ability {
                    if &ability != expected {
                        log::error!(
                            "FATAL DESYNC: NetworkController {:?}: Index {} selected {:?}, \
                             but client sent spell_ability {:?}. \
                             This indicates client/server state divergence.",
                            self.player_id,
                            idx,
                            ability,
                            expected
                        );
                        return ChoiceResult::Error(format!(
                            "FATAL DESYNC: Choice mismatch - index {} selected {:?}, \
                             but client expected {:?}",
                            idx, ability, expected
                        ));
                    }
                    log::debug!(
                        "NetworkController {:?}: Validated spell_ability matches index {}",
                        self.player_id,
                        idx
                    );
                }

                ChoiceResult::Ok(Some(ability))
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Build options list using the SHARED choice formatter so the web /
        // network frontend shows the same ownership + player-relation labels
        // as the native TUI: card targets "(yours)"/"(theirs)", player targets
        // "(you)"/"(them)" (mtg-605). These strings are display-only (not part
        // of the state hash), so they do not affect network determinism.
        let options: Vec<String> = crate::game::controller::format_card_choices(view, valid_targets, self.player_id);

        if options.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Targets {
            spell_id: spell,
            target_count: 1, // FIXME-UNFINISHED: Support multiple targets
        };

        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                // Single-select for now (FIXME-UNFINISHED for multiple targets)
                let idx = result.indices.first().copied().unwrap_or(0);
                if idx < valid_targets.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[valid_targets[idx]]))
                } else {
                    ChoiceResult::Error("Invalid target index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Build options - for each source show what mana it produces
        let options: Vec<String> = available_sources
            .iter()
            .map(|&card_id| self.format_card(view, card_id))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::ManaSources { cost: *cost };

        // FIXME-UNFINISHED: Needs multi-select for paying costs with multiple sources
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                // Single-select for now
                let idx = result.indices.first().copied().unwrap_or(0);
                if idx < available_sources.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[available_sources[idx]]))
                } else {
                    ChoiceResult::Error("Invalid mana source index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Build options - each creature can attack or not
        let mut options = vec!["Done selecting attackers".to_string()];
        for &card_id in available_creatures {
            options.push(format!("Attack with: {}", self.format_card(view, card_id)));
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Attackers {
            available_count: available_creatures.len(),
        };

        // Multi-select for attackers: indices contains all selected attacker indices
        // Index 0 means "done selecting" (no more attackers), indices 1..N are creature indices
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) if result.indices.is_empty() || result.indices == vec![0] => {
                self.increment_choice_seq();
                ChoiceResult::Ok(SmallVec::new()) // No attackers
            }
            Ok(result) => {
                self.increment_choice_seq();
                // Convert indices to CardIds (indices are 1-based, 0 is "done")
                let mut attackers = SmallVec::new();
                for &idx in &result.indices {
                    if idx == 0 {
                        continue; // Skip "done selecting" marker
                    }
                    let creature_idx = idx - 1;
                    if creature_idx < available_creatures.len() {
                        attackers.push(available_creatures[creature_idx]);
                    } else {
                        return ChoiceResult::Error(format!("Invalid attacker index {} from network", idx));
                    }
                }
                ChoiceResult::Ok(attackers)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Build options for each blocker-attacker pair
        let mut options = vec!["Done selecting blockers".to_string()];
        for &blocker in available_blockers {
            for &attacker in attackers {
                options.push(format!(
                    "{} blocks {}",
                    self.format_card(view, blocker),
                    self.format_card(view, attacker)
                ));
            }
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Blockers {
            attacker_count: attackers.len(),
            blocker_count: available_blockers.len(),
        };

        // Multi-select for blockers: indices contains all selected blocker-attacker pairs
        // Index 0 means "done selecting", indices 1..N encode (blocker, attacker) pairs
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) if result.indices.is_empty() || result.indices == vec![0] => {
                self.increment_choice_seq();
                ChoiceResult::Ok(SmallVec::new()) // No blockers
            }
            Ok(result) => {
                self.increment_choice_seq();
                // Convert indices to (blocker, attacker) pairs
                let mut blocks = SmallVec::new();
                for &idx in &result.indices {
                    if idx == 0 {
                        continue; // Skip "done selecting" marker
                    }
                    // Decode blocker-attacker pair from index
                    let pair_idx = idx - 1;
                    let blocker_idx = pair_idx / attackers.len();
                    let attacker_idx = pair_idx % attackers.len();
                    if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                        blocks.push((available_blockers[blocker_idx], attackers[attacker_idx]));
                    } else {
                        return ChoiceResult::Error(format!("Invalid blocker index {} from network", idx));
                    }
                }
                ChoiceResult::Ok(blocks)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Build options for damage order
        let options: Vec<String> = blockers
            .iter()
            .map(|&card_id| format!("Assign damage first to: {}", self.format_card(view, card_id)))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::DamageOrder {
            attacker,
            blocker_count: blockers.len(),
        };

        // Multi-select for damage order: indices specify the full ordering
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                // If indices specify a full ordering, use it directly
                // Otherwise fall back to putting first index at front
                if result.indices.len() == blockers.len() {
                    // Full ordering provided
                    let mut order = SmallVec::new();
                    for &idx in &result.indices {
                        if idx < blockers.len() {
                            order.push(blockers[idx]);
                        } else {
                            return ChoiceResult::Error(format!("Invalid damage order index {} from network", idx));
                        }
                    }
                    ChoiceResult::Ok(order)
                } else {
                    // Single index: put that blocker first, others follow in original order
                    let idx = result.indices.first().copied().unwrap_or(0);
                    if idx < blockers.len() {
                        let mut order = SmallVec::new();
                        order.push(blockers[idx]);
                        for (i, &blocker) in blockers.iter().enumerate() {
                            if i != idx {
                                order.push(blocker);
                            }
                        }
                        ChoiceResult::Ok(order)
                    } else {
                        ChoiceResult::Error("Invalid damage order index from network".to_string())
                    }
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_blocker_for_lethal_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        killable_blockers: &[(CardId, i32)],
        remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        // Build options for each killable blocker
        let options: Vec<String> = killable_blockers
            .iter()
            .map(|(card_id, lethal)| {
                format!(
                    "Kill {} first (needs {} damage)",
                    self.format_card(view, *card_id),
                    lethal
                )
            })
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::LethalDamageAssignment {
            attacker,
            killable_count: killable_blockers.len(),
            remaining_power,
        };

        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                let idx = result.indices.first().copied().unwrap_or(0);

                // Prefer target_card_ids if provided (more reliable than index)
                if let Some(ref card_ids) = result.target_card_ids {
                    if let Some(&blocker_id) = card_ids.first() {
                        // Validate the CardId exists in killable_blockers
                        if killable_blockers.iter().any(|(id, _)| *id == blocker_id) {
                            log::debug!(
                                "NetworkController: using blocker CardId {} from target_card_ids",
                                blocker_id.as_u32()
                            );
                            return ChoiceResult::Ok(blocker_id);
                        }
                        log::warn!(
                            "NetworkController: target_card_ids {:?} not in killable_blockers, falling back to index",
                            card_ids
                        );
                    }
                }

                // Fall back to index-based lookup
                if let Some((blocker_id, _)) = killable_blockers.get(idx) {
                    ChoiceResult::Ok(*blocker_id)
                } else {
                    ChoiceResult::Error(format!(
                        "FATAL DESYNC: Invalid lethal damage blocker index {} (only {} killable)",
                        idx,
                        killable_blockers.len()
                    ))
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_blocker_for_remaining_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        remaining_blockers: &[CardId],
        remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        // Build options for each remaining blocker
        let options: Vec<String> = remaining_blockers
            .iter()
            .map(|&card_id| {
                format!(
                    "Assign {} remaining damage to {}",
                    remaining_damage,
                    self.format_card(view, card_id)
                )
            })
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::RemainingDamageAssignment {
            attacker,
            blocker_count: remaining_blockers.len(),
            remaining_damage,
        };

        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                let idx = result.indices.first().copied().unwrap_or(0);

                // Prefer target_card_ids if provided (more reliable than index)
                if let Some(ref card_ids) = result.target_card_ids {
                    if let Some(&blocker_id) = card_ids.first() {
                        // Validate the CardId exists in remaining_blockers
                        if remaining_blockers.contains(&blocker_id) {
                            log::debug!(
                                "NetworkController: using remaining blocker CardId {} from target_card_ids",
                                blocker_id.as_u32()
                            );
                            return ChoiceResult::Ok(blocker_id);
                        }
                        log::warn!(
                            "NetworkController: target_card_ids {:?} not in remaining_blockers, falling back to index",
                            card_ids
                        );
                    }
                }

                // Fall back to index-based lookup
                if let Some(&blocker_id) = remaining_blockers.get(idx) {
                    ChoiceResult::Ok(blocker_id)
                } else {
                    ChoiceResult::Error(format!(
                        "FATAL DESYNC: Invalid remaining damage blocker index {} (only {} blockers)",
                        idx,
                        remaining_blockers.len()
                    ))
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Build options for each card in hand
        let options: Vec<String> = hand
            .iter()
            .map(|&card_id| format!("Discard: {}", self.format_card(view, card_id)))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Discard { count };

        // Multi-select for discarding multiple cards
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                let mut discards = SmallVec::new();
                for &idx in &result.indices {
                    if idx < hand.len() {
                        discards.push(hand[idx]);
                    } else {
                        return ChoiceResult::Error(format!("Invalid discard index {} from network", idx));
                    }
                }
                ChoiceResult::Ok(discards)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_scry_order(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::controller::ScryDecision> {
        // Build display options for each revealed card.
        let options: Vec<String> = revealed
            .iter()
            .map(|&card_id| format!("Scry: {}", self.format_card(view, card_id)))
            .collect();

        let state_hash = self.compute_view_hash(view);
        let choice_type = ChoiceType::Scry {
            count: revealed.len(),
            revealed_card_ids: revealed.to_vec(),
        };

        // Wire encoding: client returns indices of cards to put on BOTTOM.
        // The order of returned indices is the order the cards are placed
        // (first index → deepest bottom).
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                let bottom_positions: SmallVec<[usize; 4]> = result.indices.iter().copied().collect();
                // Validate every index is in range and unique.
                let mut seen = SmallVec::<[usize; 4]>::new();
                for &idx in &bottom_positions {
                    if idx >= revealed.len() {
                        return ChoiceResult::Error(format!(
                            "Invalid scry index {} (only {} revealed)",
                            idx,
                            revealed.len()
                        ));
                    }
                    if seen.contains(&idx) {
                        return ChoiceResult::Error(format!("Duplicate scry index {}", idx));
                    }
                    seen.push(idx);
                }

                // Build bottom pile in the order the client sent (first → deepest).
                let mut bottom: SmallVec<[CardId; 4]> = SmallVec::new();
                for &idx in &bottom_positions {
                    bottom.push(revealed[idx]);
                }

                // Build top pile: every revealed card NOT in bottom_positions,
                // preserving revealed (top-down) order, then converted to
                // bottom-up so library.cards.push() restores top-of-library.
                let mut top_top_down: SmallVec<[CardId; 4]> = SmallVec::new();
                for (i, &card_id) in revealed.iter().enumerate() {
                    if !bottom_positions.contains(&i) {
                        top_top_down.push(card_id);
                    }
                }
                let top: SmallVec<[CardId; 4]> = top_top_down.into_iter().rev().collect();

                ChoiceResult::Ok(crate::game::controller::ScryDecision { top, bottom })
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_surveil(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::controller::SurveilDecision> {
        // Build display options for each revealed card.
        let options: Vec<String> = revealed
            .iter()
            .map(|&card_id| format!("Surveil: {}", self.format_card(view, card_id)))
            .collect();

        let state_hash = self.compute_view_hash(view);
        let choice_type = ChoiceType::Surveil {
            count: revealed.len(),
            revealed_card_ids: revealed.to_vec(),
        };

        // Wire encoding: client returns indices of cards to put into the GRAVEYARD.
        // The order of returned indices is the order they are moved (first index
        // → deepest in graveyard pile).
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                let mill_positions: SmallVec<[usize; 4]> = result.indices.iter().copied().collect();
                let mut seen = SmallVec::<[usize; 4]>::new();
                for &idx in &mill_positions {
                    if idx >= revealed.len() {
                        return ChoiceResult::Error(format!(
                            "Invalid surveil index {} (only {} revealed)",
                            idx,
                            revealed.len()
                        ));
                    }
                    if seen.contains(&idx) {
                        return ChoiceResult::Error(format!("Duplicate surveil index {}", idx));
                    }
                    seen.push(idx);
                }

                let mut graveyard: SmallVec<[CardId; 4]> = SmallVec::new();
                for &idx in &mill_positions {
                    graveyard.push(revealed[idx]);
                }

                let mut top_top_down: SmallVec<[CardId; 4]> = SmallVec::new();
                for (i, &card_id) in revealed.iter().enumerate() {
                    if !mill_positions.contains(&i) {
                        top_top_down.push(card_id);
                    }
                }
                let top: SmallVec<[CardId; 4]> = top_top_down.into_iter().rev().collect();

                ChoiceResult::Ok(crate::game::controller::SurveilDecision { top, graveyard })
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn set_pending_library_search_card_ids(&mut self, card_ids: &[CardId]) {
        self.pending_library_search_card_ids = Some(card_ids.to_vec());
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Name-based library search protocol:
        // 1. Send unique names as options to client
        // 2. Client picks a name index
        // 3. Return the index (game loop maps back to CardId)

        // Build options for display: [0] = Decline, [1..] = card names
        let mut options = vec!["Decline to find".to_string()];
        options.extend(valid_cards.iter().map(|def| def.name.to_string()));

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request with LibrarySearchByName choice type
        // Note: name_counts is now simplified since the trait gives us deduplicated names
        let unique_names: Vec<String> = valid_cards.iter().map(|def| def.name.to_string()).collect();
        let name_counts: Vec<usize> = vec![1; unique_names.len()]; // Each name appears once in the list

        let choice_type = ChoiceType::LibrarySearchByName {
            unique_names,
            name_counts,
            filter_description: "matching cards".to_string(),
        };

        let library_search_cards = self.pending_library_search_card_ids.take();
        match self.request_choice(view, choice_type, options, state_hash, None, library_search_cards) {
            Ok(result) if result.indices.first() == Some(&0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(None) // Declined to find
            }
            Ok(result) => {
                self.increment_choice_seq();
                let name_idx_raw = result.indices.first().copied().unwrap_or(0);
                if name_idx_raw == 0 {
                    return ChoiceResult::Ok(None); // Declined
                }
                let name_idx = name_idx_raw - 1; // Adjust for "Decline" at index 0

                if name_idx < valid_cards.len() {
                    ChoiceResult::Ok(Some(name_idx))
                } else {
                    ChoiceResult::Error("Invalid library search index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Build options
        let options: Vec<String> = valid_permanents
            .iter()
            .map(|&card_id| format!("Sacrifice: {}", self.format_card(view, card_id)))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Sacrifice {
            valid_count: valid_permanents.len(),
            count,
            card_type_description: card_type_description.to_string(),
        };

        // Request choice (client sends all selections in one message for multi-select)
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                let mut sacrifices = SmallVec::new();
                for idx in result.indices {
                    if idx < valid_permanents.len() {
                        let card_id = valid_permanents[idx];
                        if !sacrifices.contains(&card_id) {
                            sacrifices.push(card_id);
                        }
                    } else {
                        return ChoiceResult::Error(format!(
                            "Invalid sacrifice index {} (max {})",
                            idx,
                            valid_permanents.len() - 1
                        ));
                    }
                }
                ChoiceResult::Ok(sacrifices)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // TODO: Add network protocol support for this choice
        // For now, auto-untap everything
        ChoiceResult::Ok(SmallVec::new())
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
        // Build options - each mode description becomes an option
        let options: Vec<String> = mode_descriptions.to_vec();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Modes {
            spell_id,
            mode_count,
            min_modes,
            can_repeat,
            available_modes: mode_descriptions.len(),
        };

        // Request choice from client
        match self.request_choice(view, choice_type, options, state_hash, None, None) {
            Ok(result) => {
                self.increment_choice_seq();
                // Validate and collect mode indices
                let mut modes = SmallVec::new();
                for idx in result.indices {
                    if idx < mode_descriptions.len() {
                        // Check for repeats if not allowed
                        if !can_repeat && modes.contains(&idx) {
                            return ChoiceResult::Error(format!(
                                "Duplicate mode {} selected (repeats not allowed)",
                                idx
                            ));
                        }
                        modes.push(idx);
                    } else {
                        return ChoiceResult::Error(format!(
                            "Invalid mode index {} (max {})",
                            idx,
                            mode_descriptions.len() - 1
                        ));
                    }
                }

                // Validate mode count
                if modes.len() < min_modes {
                    return ChoiceResult::Error(format!(
                        "Too few modes selected: {} (minimum {})",
                        modes.len(),
                        min_modes
                    ));
                }
                if modes.len() > mode_count {
                    return ChoiceResult::Error(format!(
                        "Too many modes selected: {} (maximum {})",
                        modes.len(),
                        mode_count
                    ));
                }

                ChoiceResult::Ok(modes)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No action needed for network controller
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // The WebSocket handler will send GameEnded message separately
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Network controller has no persistent state to snapshot
        None
    }

    fn has_more_choices(&self) -> bool {
        // Network controller always has more choices (until disconnected)
        true
    }

    fn get_controller_type(&self) -> ControllerType {
        // Network controller must not auto-pass - always go through ChoiceRequest flow
        // so clients are notified even when there are 0 available abilities
        ControllerType::Network
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameState;

    /// Create a minimal game state for testing
    fn create_test_game_state() -> GameState {
        GameState::new_two_player_with_capacity(
            "TestPlayer1".to_string(),
            "TestPlayer2".to_string(),
            20,
            60, // deck capacity
        )
    }

    #[test]
    fn test_network_controller_creation() {
        let (request_tx, _request_rx) = mpsc::channel();
        let (_response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        assert_eq!(controller.player_id(), PlayerId::new(1));
        assert_eq!(controller.choice_seq, 0);
    }

    #[test]
    fn test_choice_request_response() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let mut controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        // Spawn a thread to simulate the network handler
        std::thread::spawn(move || {
            let request: ChoiceRequest = request_rx.recv().unwrap();
            assert_eq!(request.choice_seq, 1);
            assert_eq!(request.options.len(), 3);
            // Verify reveals field exists (should be empty for this test)
            assert!(request.reveals.is_empty());

            response_tx
                .send(ChoiceResponse {
                    choice_seq: 1,
                    choice_indices: vec![2],
                    spell_ability: None,
                    target_card_ids: None,
                })
                .unwrap();
        });

        let options = vec!["Option A".to_string(), "Option B".to_string(), "Option C".to_string()];

        let result = controller.request_choice(
            &view,
            ChoiceType::Priority { available_count: 2 },
            options,
            0x12345678,
            None,
            None,
        );

        assert_eq!(result.unwrap().indices, vec![2]);
    }

    #[test]
    fn test_sequence_mismatch_error() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let mut controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        std::thread::spawn(move || {
            let _request: ChoiceRequest = request_rx.recv().unwrap();
            // Send response with wrong sequence number
            response_tx
                .send(ChoiceResponse {
                    choice_seq: 999, // Wrong sequence
                    choice_indices: vec![0],
                    spell_ability: None,
                    target_card_ids: None,
                })
                .unwrap();
        });

        let result = controller.request_choice(
            &view,
            ChoiceType::Priority { available_count: 0 },
            vec!["Pass".to_string()],
            0,
            None,
            None,
        );

        match result {
            Err(NetworkError::SequenceMismatch { expected: 1, got: 999 }) => {}
            _ => panic!("Expected SequenceMismatch error"),
        }
    }

    #[test]
    fn test_invalid_choice_returns_desync_error() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let mut controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        std::thread::spawn(move || {
            let _request: ChoiceRequest = request_rx.recv().unwrap();
            response_tx
                .send(ChoiceResponse {
                    choice_seq: 1,
                    choice_indices: vec![10], // Invalid - only 2 options
                    spell_ability: None,
                    target_card_ids: None,
                })
                .unwrap();
        });

        let result = controller.request_choice(
            &view,
            ChoiceType::Priority { available_count: 1 },
            vec!["Pass".to_string(), "Play land".to_string()],
            0,
            None,
            None,
        );

        // Invalid index should return a DesyncError, NOT be clamped
        // Per mtg-wsl8g: "Desync is ALWAYS a Fatal Error"
        match result {
            Err(NetworkError::DesyncError(msg)) => {
                assert!(msg.contains("DESYNC DETECTED"), "Error should mention desync: {}", msg);
                assert!(msg.contains("10"), "Error should mention invalid index 10: {}", msg);
                assert!(msg.contains("2"), "Error should mention only 2 options: {}", msg);
            }
            Ok(_) => panic!("Expected DesyncError, got Ok"),
            Err(e) => panic!("Expected DesyncError, got {:?}", e),
        }
    }

    #[test]
    fn test_network_error_display() {
        assert_eq!(NetworkError::Disconnected.to_string(), "Client disconnected");
        assert_eq!(NetworkError::Timeout.to_string(), "Request timed out");
        assert_eq!(
            NetworkError::SequenceMismatch { expected: 1, got: 2 }.to_string(),
            "Sequence mismatch: expected 1, got 2"
        );
        assert_eq!(
            NetworkError::InvalidChoice { max: 5, got: 10 }.to_string(),
            "Invalid choice index: 10 (max 5)"
        );
    }
}
