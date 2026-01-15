//! Remote controller for network clients
//!
//! This controller represents the opponent from the client's perspective.
//!
//! ## Architecture (Pre-Choice Hook Design)
//!
//! With the pre-choice hook architecture, this controller is now a simple marker:
//! - Returns `ControllerType::Remote` to identify this as a remote player
//! - All choice methods panic (they should never be called directly)
//!
//! The pre-choice hook in GameLoop handles:
//! - Blocking on the message channel
//! - Receiving OpponentChoice messages
//! - Returning UseChoice(RawChoice) which is converted to the appropriate type
//!
//! Since the hook intercepts remote player choices BEFORE calling the controller,
//! these methods should never actually be invoked.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;

/// A controller that represents the remote opponent.
///
/// This is a marker type that:
/// 1. Identifies itself as `ControllerType::Remote`
/// 2. Panics on any choice method (hook should intercept before calling)
///
/// ## Why Panic?
///
/// The pre-choice hook architecture guarantees that:
/// - For remote players, the hook receives `OpponentChoice` and returns `UseChoice`
/// - The helper functions use the `RawChoice` directly without calling the controller
/// - If these methods ARE called, it's a bug in the hook/helper logic
pub struct RemoteController {
    player_id: PlayerId,
}

impl RemoteController {
    /// Create a new remote controller for the given player
    pub fn new(player_id: PlayerId) -> Self {
        Self { player_id }
    }
}

impl PlayerController for RemoteController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        _available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        panic!(
            "RemoteController::choose_spell_ability_to_play called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        panic!(
            "RemoteController::choose_targets called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        _available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        panic!(
            "RemoteController::choose_mana_sources_to_pay called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        _available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        panic!(
            "RemoteController::choose_attackers called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        _available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        panic!(
            "RemoteController::choose_blockers called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        _blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        panic!(
            "RemoteController::choose_damage_assignment_order called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        _hand: &[CardId],
        _count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        panic!(
            "RemoteController::choose_cards_to_discard called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_from_library(&mut self, _view: &GameStateView, _valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        panic!(
            "RemoteController::choose_from_library called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        _valid_permanents: &[CardId],
        _count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        panic!(
            "RemoteController::choose_permanents_to_sacrifice called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        panic!(
            "RemoteController::choose_permanents_to_not_untap called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        _mode_descriptions: &[String],
        _mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        panic!(
            "RemoteController::choose_modes called! \
             This should never happen - the pre-choice hook should intercept remote player choices."
        );
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
