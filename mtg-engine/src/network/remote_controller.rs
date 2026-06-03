//! Remote controller for network clients
//!
//! This controller represents the opponent from the client's perspective.
//!
//! ## Architecture (MVar Design)
//!
//! With the MVar architecture:
//! - Returns `ControllerType::Remote` to identify this as a remote player
//! - In MVar mode: Reads OpponentChoice from SharedNetworkState
//! - In legacy mode: Panics (pre-choice hook should intercept)
//!
//! The network reader task populates the MVar with OpponentChoice,
//! and this controller reads from it when a choice method is called.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::network::client::SharedNetworkState;
use smallvec::SmallVec;
use std::sync::Arc;

/// Cached choice info from prepare_for_priority_choice()
#[derive(Debug, Clone)]
struct CachedOpponentChoice {
    action_count: u64,
    indices: Vec<usize>,
    spell_ability: Option<SpellAbility>,
    library_search_result: Option<CardId>,
    target_card_ids: Option<Vec<CardId>>,
}

/// A controller that represents the remote opponent.
///
/// Supports two modes:
/// - MVar: Reads OpponentChoice from SharedNetworkState
/// - Legacy: Panics (pre-choice hook should intercept)
///
/// ## Network Sync Protocol (prepare_for_priority_choice)
///
/// Like NetworkLocalController, this controller implements a two-phase protocol:
/// 1. prepare_for_priority_choice() blocks on MVar to receive OpponentChoice
/// 2. GameLoop calls sync_to_action() to process any buffered reveals
/// 3. Abilities are computed (now correct, includes opponent's drawn cards)
/// 4. choose_spell_ability_to_play() uses cached choice (doesn't block again)
pub struct RemoteController {
    player_id: PlayerId,
    /// Shared network state (MVar architecture) - if set, reads choices from MVar
    shared_state: Option<Arc<SharedNetworkState>>,
    /// Last library search result from server, consumed by take_library_search_result.
    /// Stored here so the game loop can retrieve the authoritative CardId after
    /// choose_from_library returns an index into an empty valid_cards list.
    last_library_search_result: Option<CardId>,
    /// Cached OpponentChoice from prepare_for_priority_choice()
    pending_choice: Option<CachedOpponentChoice>,
}

impl RemoteController {
    /// Create a new remote controller for the given player (legacy mode)
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            shared_state: None,
            last_library_search_result: None,
            pending_choice: None,
        }
    }

    /// Create a new remote controller with shared state (MVar mode)
    pub fn new_with_shared_state(player_id: PlayerId, shared_state: Arc<SharedNetworkState>) -> Self {
        Self {
            player_id,
            shared_state: Some(shared_state),
            last_library_search_result: None,
            pending_choice: None,
        }
    }

    /// Block on MVar to receive OpponentChoice and cache it
    ///
    /// Called by GameLoop BEFORE computing abilities. This ensures:
    /// 1. We've received the OpponentChoice from server
    /// 2. All CardRevealed messages preceding it are now buffered
    /// 3. GameLoop can call sync_to_action() to process those reveals
    /// 4. Abilities can be computed correctly (including opponent's drawn cards)
    ///
    /// Returns true if an OpponentChoice was received and cached.
    /// Returns false if game should exit (GameEnded/Error received).
    fn prepare_choice_info(&mut self) -> bool {
        // Already have cached info? Nothing to do.
        if self.pending_choice.is_some() {
            return true;
        }

        // Only relevant for shared-state (network) mode
        let Some(ref state) = self.shared_state else {
            return true; // Legacy mode always proceeds
        };

        // Block (NO timeout) for the next unconsumed opponent choice in the
        // choice buffer (Phase 2 step 3b). Returns None only on terminal
        // disconnect (game ended / fatal error).
        match state.take_opponent_choice() {
            Some(entry) => {
                log::debug!(
                    "RemoteController::prepare_choice_info: got OpponentChoice seq={} action={} indices={:?}",
                    entry.choice_seq,
                    entry.action_count,
                    entry.choice_indices
                );
                self.pending_choice = Some(CachedOpponentChoice {
                    action_count: entry.action_count,
                    indices: entry.choice_indices,
                    spell_ability: entry.spell_ability,
                    library_search_result: entry.library_search_result,
                    target_card_ids: entry.target_card_ids,
                });
                true
            }
            None => {
                log::debug!("RemoteController::prepare_choice_info: opponent-choice buffer signaled exit");
                false
            }
        }
    }

    /// Get opponent's choice from MVar with action count validation
    ///
    /// Uses cached value from prepare_choice_info() if available.
    /// In MVar mode: Takes OpponentChoice from remote_choice_mvar (blocking if needed)
    /// In legacy mode: Panics (this shouldn't be called)
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

    /// Get opponent's choice from MVar with full info including library_search_result
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
                    cached.indices
                );
                // Continue anyway - server is authoritative, but log the discrepancy
            }
            log::debug!(
                "RemoteController: using cached OpponentChoice indices={:?} action={}",
                cached.indices,
                cached.action_count
            );
            return ChoiceResult::Ok((
                cached.indices,
                cached.spell_ability,
                cached.library_search_result,
                cached.target_card_ids,
            ));
        }

        if let Some(ref state) = self.shared_state {
            // Network mode: read the next unconsumed opponent choice from the
            // buffer (Phase 2 step 3b), keyed by choice_seq, non-destructive.
            match state.take_opponent_choice() {
                Some(entry) => {
                    // Validate action count ordering
                    if entry.action_count != expected_action {
                        log::warn!(
                            "RemoteController: action count mismatch! expected={}, got={}, indices={:?}",
                            expected_action,
                            entry.action_count,
                            entry.choice_indices
                        );
                        // Continue anyway - server is authoritative, but log the discrepancy
                    }
                    log::debug!(
                        "RemoteController: got OpponentChoice seq={} indices={:?} action={} lib_search={:?} targets={:?}",
                        entry.choice_seq,
                        entry.choice_indices,
                        entry.action_count,
                        entry.library_search_result,
                        entry.target_card_ids
                    );
                    ChoiceResult::Ok((
                        entry.choice_indices,
                        entry.spell_ability,
                        entry.library_search_result,
                        entry.target_card_ids,
                    ))
                }
                None => {
                    log::debug!("RemoteController: opponent-choice buffer signaled exit");
                    ChoiceResult::ExitGame
                }
            }
        } else {
            panic!(
                "RemoteController choice method called in legacy mode! \
                 This should never happen - the pre-choice hook should intercept remote player choices."
            );
        }
    }
}

impl PlayerController for RemoteController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn prepare_for_priority_choice(&mut self) -> bool {
        // Block on MVar to receive OpponentChoice, cache it for later
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

        let sources: SmallVec<[CardId; 8]> = indices
            .into_iter()
            .filter_map(|idx| available_sources.get(idx).copied())
            .collect();
        ChoiceResult::Ok(sources)
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

        // Index 0 = pass (no attackers), index N = creature N-1
        if indices.first().copied() == Some(0) {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let attackers: SmallVec<[CardId; 8]> = indices
            .into_iter()
            .filter_map(|idx| {
                if idx > 0 {
                    available_creatures.get(idx - 1).copied()
                } else {
                    None
                }
            })
            .collect();
        ChoiceResult::Ok(attackers)
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

        // Index 0 = pass (no blockers), index N = (blocker_idx, attacker_idx) + 1
        if indices.first().copied() == Some(0) {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let blocks: SmallVec<[(CardId, CardId); 8]> = indices
            .into_iter()
            .filter_map(|idx| {
                if idx > 0 {
                    let idx = idx - 1;
                    let blocker_idx = idx / attackers.len();
                    let attacker_idx = idx % attackers.len();
                    let blocker = available_blockers.get(blocker_idx).copied()?;
                    let attacker = attackers.get(attacker_idx).copied()?;
                    Some((blocker, attacker))
                } else {
                    None
                }
            })
            .collect();
        ChoiceResult::Ok(blocks)
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

        // Prefer the actual CardId if provided (more reliable than index)
        if let Some(ref card_ids) = blocker_card_ids {
            if let Some(&blocker_id) = card_ids.first() {
                log::info!(
                    "RemoteController: using blocker_card_id {} from target_card_ids",
                    blocker_id.as_u32()
                );
                // Validate the CardId exists in killable_blockers
                if killable_blockers.iter().any(|(id, _)| *id == blocker_id) {
                    return ChoiceResult::Ok(blocker_id);
                }
                // HARD ERROR (mtg-w5sa2): a CardId WAS submitted but is NOT in
                // our authoritative killable_blockers — the two sides' combat
                // state has diverged. The old index fallback MASKED this by
                // silently picking a different, order-dependent blocker, which
                // cascades into a later view-hash desync. Desync is ALWAYS fatal:
                // surface it at the exact divergence point instead of recovering
                // with the wrong blocker (the recovery hack the rewind vision
                // forbids).
                let error_msg = format!(
                    "FATAL DESYNC: RemoteController lethal-damage blocker {:?} not in killable_blockers {:?} \
                     (combat-state divergence; index fallback removed — mtg-w5sa2)",
                    card_ids,
                    killable_blockers.iter().map(|(id, _)| id.as_u32()).collect::<Vec<_>>()
                );
                log::error!("{}", error_msg);
                return ChoiceResult::Error(error_msg);
            }
        }

        // Index-based selection ONLY when no CardId was provided (legacy/no-id
        // peers). The CardId path above is authoritative for the rewind+replay
        // network protocol.
        let idx = indices.first().copied().unwrap_or(0);
        if let Some((blocker_id, _)) = killable_blockers.get(idx) {
            ChoiceResult::Ok(*blocker_id)
        } else {
            // FATAL: Invalid index indicates client/server desync
            let error_msg = format!(
                "DESYNC: RemoteController received invalid blocker index {} (only {} killable blockers). \
                 This indicates client/server state divergence - a bug that must be fixed.",
                idx,
                killable_blockers.len()
            );
            log::error!("{}", error_msg);
            ChoiceResult::Error(error_msg)
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

        // Prefer the actual CardId if provided (more reliable than index)
        if let Some(card_ids) = blocker_card_ids {
            if let Some(&blocker_id) = card_ids.first() {
                // Validate the CardId exists in remaining_blockers
                if remaining_blockers.contains(&blocker_id) {
                    return ChoiceResult::Ok(blocker_id);
                }
                // HARD ERROR (mtg-w5sa2): submitted CardId not in our
                // authoritative remaining_blockers → combat-state divergence.
                // Index fallback removed (it masked the desync by picking a
                // different order-dependent blocker). Desync is ALWAYS fatal.
                let error_msg = format!(
                    "FATAL DESYNC: RemoteController remaining-damage blocker {:?} not in remaining_blockers {:?} \
                     (combat-state divergence; index fallback removed — mtg-w5sa2)",
                    card_ids, remaining_blockers
                );
                log::error!("{}", error_msg);
                return ChoiceResult::Error(error_msg);
            }
        }

        // Index-based selection ONLY when no CardId was provided (legacy/no-id
        // peers). The CardId path above is authoritative.
        let idx = indices.first().copied().unwrap_or(0);
        if let Some(&blocker_id) = remaining_blockers.get(idx) {
            ChoiceResult::Ok(blocker_id)
        } else {
            // FATAL: Invalid index indicates client/server desync
            let error_msg = format!(
                "DESYNC: RemoteController received invalid remaining blocker index {} (only {} remaining). \
                 This indicates client/server state divergence - a bug that must be fixed.",
                idx,
                remaining_blockers.len()
            );
            log::error!("{}", error_msg);
            ChoiceResult::Error(error_msg)
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

        let discards: SmallVec<[CardId; 7]> = indices.into_iter().filter_map(|idx| hand.get(idx).copied()).collect();
        ChoiceResult::Ok(discards)
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

        let sacrifices: SmallVec<[CardId; 8]> = indices
            .into_iter()
            .filter_map(|idx| valid_permanents.get(idx).copied())
            .collect();
        ChoiceResult::Ok(sacrifices)
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

        let stay_tapped: SmallVec<[CardId; 8]> = indices
            .into_iter()
            .filter_map(|idx| may_not_untap_permanents.get(idx).copied())
            .collect();
        ChoiceResult::Ok(stay_tapped)
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
