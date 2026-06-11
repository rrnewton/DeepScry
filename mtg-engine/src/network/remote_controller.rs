//! Remote controller for network clients
//!
//! This controller represents the opponent from the client's perspective.
//!
//! ## Architecture (opponent-choice cursor buffer)
//!
//! - Returns `ControllerType::Remote` to identify this as a remote player.
//! - Replays the opponent's decisions from an append-only
//!   `ActionLog<ChoiceEntry>` cursor buffer in `SharedNetworkState`
//!   (`take_opponent_choice`), the log-as-source-of-truth model — NOT a
//!   destructive MVar.
//!
//! The WS reader appends each opponent choice to that buffer (keyed by the
//! server's monotonic `choice_seq`) via `push_opponent_choice`; this
//! controller advances a read cursor over it when a choice method is called.
//! Because the read is non-destructive, a rewind/replay can reset the cursor
//! and re-hand the same choices in order.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::network::client::SharedNetworkState;
use crate::network::{ChoiceEntry, ChoicePayload};
use smallvec::SmallVec;
use std::sync::Arc;

/// A controller that represents the remote opponent.
///
/// ## Network Sync Protocol (prepare_for_priority_choice)
///
/// Like NetworkLocalController, this controller implements a two-phase protocol:
/// 1. prepare_for_priority_choice() blocks on the opponent-choice cursor buffer
///    (`take_opponent_choice`) to receive the next opponent decision and caches it
/// 2. GameLoop calls sync_to_action() to process any buffered reveals
/// 3. Abilities are computed (now correct, includes opponent's drawn cards)
/// 4. choose_spell_ability_to_play() uses cached choice (doesn't block again)
pub struct RemoteController {
    player_id: PlayerId,
    /// Shared network state: the opponent-choice cursor buffer this controller replays from.
    shared_state: Arc<SharedNetworkState>,
    /// Last library search result from server, consumed by take_library_search_result.
    /// Stored here so the game loop can retrieve the authoritative CardId after
    /// choose_from_library returns an index into an empty valid_cards list.
    last_library_search_result: Option<CardId>,
    /// Cached opponent choice from prepare_for_priority_choice(). Holds the
    /// `ChoiceEntry` taken from the per-controller opponent-choice buffer until
    /// `get_opponent_choice_full` consumes it (mtg-787: was a field-renamed
    /// clone `CachedOpponentChoice`; now reuses the buffer's own entry type).
    pending_choice: Option<ChoiceEntry>,
}

impl RemoteController {
    /// Create a new remote controller with shared state.
    pub fn new_with_shared_state(player_id: PlayerId, shared_state: Arc<SharedNetworkState>) -> Self {
        Self {
            player_id,
            shared_state,
            last_library_search_result: None,
            pending_choice: None,
        }
    }

    /// Block on the opponent-choice cursor buffer to receive the next opponent
    /// decision and cache it.
    ///
    /// Called by GameLoop BEFORE computing abilities. This ensures:
    /// 1. We've received the opponent's choice from the server (via the buffer)
    /// 2. All CardRevealed messages preceding it are now buffered
    /// 3. GameLoop can call sync_to_action() to process those reveals
    /// 4. Abilities can be computed correctly (including opponent's drawn cards)
    ///
    /// Returns true if an opponent choice was received and cached.
    /// Returns false if game should exit (GameEnded/Error received).
    fn prepare_choice_info(&mut self) -> bool {
        // Already have cached info? Nothing to do.
        if self.pending_choice.is_some() {
            return true;
        }

        // Block (NO timeout) for the next unconsumed opponent choice in the
        // choice buffer (mtg-629 step 3b). Returns None only on terminal
        // disconnect (game ended / fatal error).
        match self.shared_state.take_opponent_choice() {
            Some(entry) => {
                log::debug!(
                    "RemoteController::prepare_choice_info: got OpponentChoice seq={} action={} indices={:?}",
                    entry.choice_seq,
                    entry.action_count,
                    entry.payload.choice_indices
                );
                self.pending_choice = Some(entry);
                true
            }
            None => {
                log::debug!("RemoteController::prepare_choice_info: opponent-choice buffer signaled exit");
                false
            }
        }
    }

    /// Get opponent's choice from the opponent-choice buffer with action count validation
    ///
    /// Uses cached value from prepare_choice_info() if available.
    ///
    /// The `expected_action` parameter is used for validation in network mode.
    /// If the choice's action_count doesn't match, this indicates a sync issue.
    fn get_opponent_choice(&mut self, expected_action: u64) -> ChoiceResult<(Vec<usize>, Option<SpellAbility>)> {
        match self.get_opponent_choice_full(expected_action) {
            ChoiceResult::Ok((indices, spell_ability, _library_search_result, _target_card_ids)) => {
                ChoiceResult::Ok((indices, spell_ability))
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::UndoRequest(n) => ChoiceResult::UndoRequest(n),
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::NeedInput(i) => ChoiceResult::NeedInput(i),
        }
    }

    /// Get opponent's choice with target CardIds (for target selections)
    fn get_opponent_choice_with_targets(
        &mut self,
        expected_action: u64,
    ) -> ChoiceResult<(Vec<usize>, Option<Vec<CardId>>)> {
        match self.get_opponent_choice_full(expected_action) {
            ChoiceResult::Ok((indices, _spell_ability, _library_search_result, target_card_ids)) => {
                ChoiceResult::Ok((indices, target_card_ids))
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::UndoRequest(n) => ChoiceResult::UndoRequest(n),
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::NeedInput(i) => ChoiceResult::NeedInput(i),
        }
    }

    /// Get opponent's choice from the opponent-choice buffer with full info including library_search_result
    ///
    /// Uses cached value from prepare_choice_info() if available.
    /// This is the underlying implementation that returns all choice info.
    /// Used by choose_from_library to get the authoritative CardId for library searches.
    #[allow(clippy::type_complexity)]
    fn get_opponent_choice_full(
        &mut self,
        expected_action: u64,
    ) -> ChoiceResult<(Vec<usize>, Option<SpellAbility>, Option<CardId>, Option<Vec<CardId>>)> {
        // Check for cached choice from prepare_choice_info()
        if let Some(cached) = self.pending_choice.take() {
            // Validate action count ordering
            if cached.action_count != expected_action {
                log::warn!(
                    "RemoteController: action count mismatch! expected={}, got={}, indices={:?}",
                    expected_action,
                    cached.action_count,
                    cached.payload.choice_indices
                );
                // Continue anyway - server is authoritative, but log the discrepancy
            }
            log::debug!(
                "RemoteController: using cached OpponentChoice indices={:?} action={}",
                cached.payload.choice_indices,
                cached.action_count
            );
            let ChoicePayload {
                choice_indices,
                spell_ability,
                library_search_result,
                target_card_ids,
            } = cached.payload;
            return ChoiceResult::Ok((choice_indices, spell_ability, library_search_result, target_card_ids));
        }

        // Read the next unconsumed opponent choice from the buffer (mtg-629
        // step 3b), keyed by choice_seq, non-destructive.
        match self.shared_state.take_opponent_choice() {
            Some(entry) => {
                // Validate action count ordering
                if entry.action_count != expected_action {
                    log::warn!(
                        "RemoteController: action count mismatch! expected={}, got={}, indices={:?}",
                        expected_action,
                        entry.action_count,
                        entry.payload.choice_indices
                    );
                    // Continue anyway - server is authoritative, but log the discrepancy
                }
                log::debug!(
                    "RemoteController: got OpponentChoice seq={} indices={:?} action={} lib_search={:?} targets={:?}",
                    entry.choice_seq,
                    entry.payload.choice_indices,
                    entry.action_count,
                    entry.payload.library_search_result,
                    entry.payload.target_card_ids
                );
                let ChoicePayload {
                    choice_indices,
                    spell_ability,
                    library_search_result,
                    target_card_ids,
                } = entry.payload;
                ChoiceResult::Ok((choice_indices, spell_ability, library_search_result, target_card_ids))
            }
            None => {
                log::debug!("RemoteController: opponent-choice buffer signaled exit");
                ChoiceResult::ExitGame
            }
        }
    }
}

impl PlayerController for RemoteController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn prepare_for_priority_choice(&mut self) -> bool {
        // Block on the opponent-choice buffer to receive the opponent's choice, cache it for later
        // This ensures CardRevealed messages are buffered before abilities are computed
        self.prepare_choice_info()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        let (indices, spell_ability) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        // If server sent the actual spell ability, use it directly
        if let Some(ability) = spell_ability {
            return ChoiceResult::Ok(Some(ability));
        }

        // Otherwise convert index to ability
        // Index 0 = pass, index N = available[N-1]
        let idx = indices.first().copied().unwrap_or(0);
        if idx == 0 {
            ChoiceResult::Ok(None)
        } else if idx - 1 < available.len() {
            ChoiceResult::Ok(Some(available[idx - 1].clone()))
        } else {
            // FATAL: Invalid index indicates client/server desync
            let error_msg = format!(
                "DESYNC: RemoteController received invalid ability index {} (only {} available). \
                 This indicates client/server state divergence - a bug that must be fixed.",
                idx,
                available.len()
            );
            log::error!("{}", error_msg);
            ChoiceResult::Error(error_msg)
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
        _min_targets: usize,
        _max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // The server already decided the full target set (any count); we replay
        // its indices / CardIds verbatim, so variable target counts round-trip.
        let (indices, target_card_ids) = match self.get_opponent_choice_with_targets(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        // Use server-provided target CardIds directly if available
        // This is more reliable than index-based lookup which can fail
        // if the client's valid_targets list differs from the server's
        if let Some(card_ids) = target_card_ids {
            log::debug!(
                "RemoteController::choose_targets: using server-provided targets {:?}",
                card_ids
            );
            return ChoiceResult::Ok(card_ids.into_iter().collect());
        }

        // Fallback to index-based lookup (legacy compatibility)
        let targets: SmallVec<[CardId; 4]> = indices
            .into_iter()
            .filter_map(|idx| valid_targets.get(idx).copied())
            .collect();
        ChoiceResult::Ok(targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        ChoiceResult::Ok(crate::network::decode_subset(&indices, available_sources))
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        ChoiceResult::Ok(crate::network::decode_attackers(&indices, available_creatures))
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        ChoiceResult::Ok(crate::network::decode_blockers(&indices, available_blockers, attackers))
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        let order: SmallVec<[CardId; 4]> = indices
            .into_iter()
            .filter_map(|idx| blockers.get(idx).copied())
            .collect();
        ChoiceResult::Ok(order)
    }

    fn choose_blocker_for_lethal_damage(
        &mut self,
        view: &GameStateView,
        _attacker: CardId,
        killable_blockers: &[(CardId, i32)],
        _remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        // Use get_opponent_choice_with_targets to get the actual CardId
        // (index-based protocol fails when blocker lists differ between server/client)
        let (indices, blocker_card_ids) = match self.get_opponent_choice_with_targets(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        log::info!(
            "RemoteController::choose_blocker_for_lethal_damage: indices={:?}, blocker_card_ids={:?}, killable_blockers={:?}",
            indices,
            blocker_card_ids,
            killable_blockers.iter().map(|(id, _)| id.as_u32()).collect::<Vec<_>>()
        );

        let valid: SmallVec<[CardId; 8]> = killable_blockers.iter().map(|(id, _)| *id).collect();
        match crate::network::resolve_combat_blocker(&indices, blocker_card_ids.as_deref(), &valid, "lethal-damage") {
            Ok(id) => ChoiceResult::Ok(id),
            Err(msg) => {
                log::error!("{}", msg);
                ChoiceResult::Error(msg)
            }
        }
    }

    fn choose_blocker_for_remaining_damage(
        &mut self,
        view: &GameStateView,
        _attacker: CardId,
        remaining_blockers: &[CardId],
        _remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        // Use get_opponent_choice_with_targets to get the actual CardId
        // (index-based protocol fails when blocker lists differ between server/client)
        let (indices, blocker_card_ids) = match self.get_opponent_choice_with_targets(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        match crate::network::resolve_combat_blocker(
            &indices,
            blocker_card_ids.as_deref(),
            remaining_blockers,
            "remaining-damage",
        ) {
            Ok(id) => ChoiceResult::Ok(id),
            Err(msg) => {
                log::error!("{}", msg);
                ChoiceResult::Error(msg)
            }
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        _count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        ChoiceResult::Ok(crate::network::decode_subset(&indices, hand))
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        _valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Get the opponent's choice indices from the network
        // Protocol: index 0 = decline, index 1+ = name indices (1-based)
        let (indices, _spell_ability, library_search_result, _target_card_ids) =
            match self.get_opponent_choice_full(view.action_count() as u64) {
                ChoiceResult::Ok(choice) => choice,
                ChoiceResult::UndoRequest(_)
                | ChoiceResult::ExitGame
                | ChoiceResult::Error(_)
                | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
            };

        // Store the server-authoritative CardId so game loop can retrieve it
        // via take_library_search_result() after this method returns.
        self.last_library_search_result = library_search_result;

        // Convert from protocol format (1-based with 0=decline) to trait format (0-based Option)
        let name_idx_raw = indices.first().copied().unwrap_or(0);
        let result = if name_idx_raw == 0 {
            None // Declined
        } else {
            Some(name_idx_raw - 1) // Convert to 0-based index
        };

        log::debug!(
            "RemoteController::choose_from_library: indices={:?}, result={:?}",
            indices,
            result
        );
        ChoiceResult::Ok(result)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        _count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        ChoiceResult::Ok(crate::network::decode_subset(&indices, valid_permanents))
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        ChoiceResult::Ok(crate::network::decode_subset(&indices, may_not_untap_permanents))
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        _spell_id: CardId,
        _mode_descriptions: &[String],
        _mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

        let modes: SmallVec<[usize; 4]> = indices.into_iter().collect();
        ChoiceResult::Ok(modes)
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No-op for remote controller
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // No-op for remote controller
    }

    fn take_library_search_result(&mut self) -> Option<CardId> {
        self.last_library_search_result.take()
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Remote
    }
}
