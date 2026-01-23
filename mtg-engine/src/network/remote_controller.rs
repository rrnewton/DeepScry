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
use crate::network::client::{RemoteChoiceInfo, SharedNetworkState};
use smallvec::SmallVec;
use std::sync::Arc;

/// A controller that represents the remote opponent.
///
/// Supports two modes:
/// - MVar: Reads OpponentChoice from SharedNetworkState
/// - Legacy: Panics (pre-choice hook should intercept)
pub struct RemoteController {
    player_id: PlayerId,
    /// Shared network state (MVar architecture) - if set, reads choices from MVar
    shared_state: Option<Arc<SharedNetworkState>>,
}

impl RemoteController {
    /// Create a new remote controller for the given player (legacy mode)
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            shared_state: None,
        }
    }

    /// Create a new remote controller with shared state (MVar mode)
    pub fn new_with_shared_state(player_id: PlayerId, shared_state: Arc<SharedNetworkState>) -> Self {
        Self {
            player_id,
            shared_state: Some(shared_state),
        }
    }

    /// Get opponent's choice from MVar with action count validation
    ///
    /// In MVar mode: Takes OpponentChoice from remote_choice_mvar (blocking if needed)
    /// In legacy mode: Panics (this shouldn't be called)
    ///
    /// The `expected_action` parameter is used for validation in network mode.
    /// If the choice's action_count doesn't match, this indicates a sync issue.
    fn get_opponent_choice(&self, expected_action: u64) -> ChoiceResult<(Vec<usize>, Option<SpellAbility>)> {
        match self.get_opponent_choice_full(expected_action) {
            ChoiceResult::Ok((indices, spell_ability, _library_search_result)) => {
                ChoiceResult::Ok((indices, spell_ability))
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::UndoRequest(n) => ChoiceResult::UndoRequest(n),
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::NeedInput(i) => ChoiceResult::NeedInput(i),
        }
    }

    /// Get opponent's choice from MVar with full info including library_search_result
    ///
    /// This is the underlying implementation that returns all choice info.
    /// Used by choose_from_library to get the authoritative CardId for library searches.
    fn get_opponent_choice_full(
        &self,
        expected_action: u64,
    ) -> ChoiceResult<(Vec<usize>, Option<SpellAbility>, Option<CardId>)> {
        if let Some(ref state) = self.shared_state {
            // MVar mode: take from REMOTE choice MVar (dedicated for this controller)
            match state.take_remote_choice() {
                Some(RemoteChoiceInfo::Opponent {
                    action_count,
                    indices,
                    spell_ability,
                    library_search_result,
                }) => {
                    // Validate action count ordering
                    if action_count != expected_action {
                        log::warn!(
                            "RemoteController: action count mismatch! expected={}, got={}, indices={:?}",
                            expected_action,
                            action_count,
                            indices
                        );
                        // Continue anyway - server is authoritative, but log the discrepancy
                    }
                    log::debug!(
                        "RemoteController: got OpponentChoice indices={:?} action={} lib_search={:?}",
                        indices,
                        action_count,
                        library_search_result
                    );
                    ChoiceResult::Ok((indices, spell_ability, library_search_result))
                }
                Some(RemoteChoiceInfo::Exit { winner }) => {
                    log::info!("RemoteController: game ended, winner={:?}", winner);
                    ChoiceResult::ExitGame
                }
                Some(RemoteChoiceInfo::Error { message }) => {
                    log::error!("RemoteController: error from server: {}", message);
                    ChoiceResult::ExitGame
                }
                None => {
                    log::debug!("RemoteController: MVar returned None (exit signaled)");
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
            log::warn!(
                "RemoteController: invalid ability index {} (available={})",
                idx,
                available.len()
            );
            ChoiceResult::Ok(None)
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let (indices, _) = match self.get_opponent_choice(view.action_count() as u64) {
            ChoiceResult::Ok(choice) => choice,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
        };

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

    fn choose_from_library(&mut self, view: &GameStateView, _valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // Use get_opponent_choice_full to get the authoritative library_search_result.
        // The local valid_cards list is unavailable in network mode because unrevealed library
        // cards are not instantiated in the shadow game (card slots are `None`).
        // We trust the server's library_search_result which contains the exact CardId chosen.
        let (_indices, _spell_ability, library_search_result) =
            match self.get_opponent_choice_full(view.action_count() as u64) {
                ChoiceResult::Ok(choice) => choice,
                ChoiceResult::UndoRequest(_)
                | ChoiceResult::ExitGame
                | ChoiceResult::Error(_)
                | ChoiceResult::NeedInput(_) => return ChoiceResult::ExitGame,
            };

        log::debug!(
            "RemoteController::choose_from_library: using library_search_result={:?}",
            library_search_result
        );
        ChoiceResult::Ok(library_search_result)
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

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Remote
    }
}
