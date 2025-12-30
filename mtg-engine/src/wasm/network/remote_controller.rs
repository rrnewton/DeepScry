//! WASM Remote Controller
//!
//! Handles opponent choices received from the server.
//! Returns `NeedInput` when no OpponentChoice message is available.
//!
//! ## Design
//!
//! This is the WASM equivalent of the native `RemoteController`, but instead
//! of blocking on a channel, it checks the network client's queue and returns
//! `NeedInput` if empty.
//!
//! ## Code Sharing Note
//!
//! The choice processing logic (extracting spell_ability, falling back to index)
//! mirrors `crate::network::remote_controller::RemoteController`. Consider
//! extracting common helpers if this logic grows more complex.

use super::client::SharedNetworkClient;
use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;

/// Context returned when waiting for opponent
fn waiting_for_opponent_context() -> ChoiceContext {
    ChoiceContext::SpellAbility {
        available: vec![],
        formatted_choices: vec!["Waiting for opponent...".to_string()],
    }
}

/// WASM Remote Controller
///
/// Represents the opponent from the client's perspective. When the GameLoop
/// asks for a choice, this controller checks if an OpponentChoice message
/// is available from the server. If not, it returns `NeedInput`.
pub struct WasmRemoteController {
    player_id: PlayerId,
    /// Shared reference to the network client
    network_client: SharedNetworkClient,
    /// Last received spell ability (from OpponentChoice)
    last_spell_ability: Option<SpellAbility>,
    /// Whether the game has ended
    game_ended: bool,
}

impl WasmRemoteController {
    /// Create a new remote controller
    pub fn new(player_id: PlayerId, network_client: SharedNetworkClient) -> Self {
        Self {
            player_id,
            network_client,
            last_spell_ability: None,
            game_ended: false,
        }
    }

    /// Try to get the next opponent choice
    ///
    /// Returns the choice index, or NeedInput if none available.
    fn try_get_choice(&mut self) -> ChoiceResult<usize> {
        // Check if game has ended
        let client = self.network_client.borrow();
        if client.state() == super::client::NetworkState::GameEnded {
            drop(client);
            self.game_ended = true;
            return ChoiceResult::ExitGame;
        }
        drop(client);

        // Try to pop an opponent choice
        let mut client = self.network_client.borrow_mut();
        if let Some(choice) = client.pop_opponent_choice() {
            log::debug!(
                "WasmRemoteController: Opponent chose index {} ({})",
                choice.choice_index,
                choice.description
            );
            // Store spell_ability for choose_spell_ability_to_play to use
            self.last_spell_ability = choice.spell_ability;
            ChoiceResult::Ok(choice.choice_index)
        } else {
            ChoiceResult::NeedInput(waiting_for_opponent_context())
        }
    }

    /// Helper to select from a slice based on choice index
    fn select_from_slice<T: Clone>(&mut self, items: &[T]) -> ChoiceResult<Option<T>> {
        match self.try_get_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < items.len() {
                    ChoiceResult::Ok(Some(items[idx].clone()))
                } else {
                    // Index >= len typically means "none" or "pass"
                    ChoiceResult::Ok(None)
                }
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }
}

impl PlayerController for WasmRemoteController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        match self.try_get_choice() {
            ChoiceResult::Ok(0) => ChoiceResult::Ok(None), // Pass
            ChoiceResult::Ok(idx) => {
                // If server sent the actual spell ability, use it directly
                // This handles the case where client doesn't know opponent's hand
                if let Some(ability) = self.last_spell_ability.take() {
                    log::debug!(
                        "WasmRemoteController: Using server-provided spell ability: {:?}",
                        ability
                    );
                    return ChoiceResult::Ok(Some(ability));
                }

                // Fall back to index-based lookup
                let ability_idx = idx - 1;
                if ability_idx < available.len() {
                    ChoiceResult::Ok(Some(available[ability_idx].clone()))
                } else {
                    log::warn!(
                        "WasmRemoteController: Invalid ability index {} (available={}, spell_ability was None)",
                        ability_idx,
                        available.len()
                    );
                    ChoiceResult::Ok(None)
                }
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        match self.select_from_slice(valid_targets) {
            ChoiceResult::Ok(Some(target)) => ChoiceResult::Ok(smallvec::smallvec![target]),
            ChoiceResult::Ok(None) => ChoiceResult::Ok(SmallVec::new()),
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // For mana sources, we receive a bitmask or index
        // For simplicity, treat as single source selection
        match self.select_from_slice(available_sources) {
            ChoiceResult::Ok(Some(source)) => ChoiceResult::Ok(smallvec::smallvec![source]),
            ChoiceResult::Ok(None) => ChoiceResult::Ok(SmallVec::new()),
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Choice index encodes number of attackers (simplified)
        match self.try_get_choice() {
            ChoiceResult::Ok(count) => {
                let attackers: SmallVec<[CardId; 8]> = available_creatures.iter().take(count).copied().collect();
                ChoiceResult::Ok(attackers)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Choice index encodes number of blocks (simplified)
        match self.try_get_choice() {
            ChoiceResult::Ok(count) => {
                // Simplified: first N blockers block first N attackers
                let blocks: SmallVec<[(CardId, CardId); 8]> = available_blockers
                    .iter()
                    .zip(attackers.iter())
                    .take(count)
                    .map(|(&b, &a)| (b, a))
                    .collect();
                ChoiceResult::Ok(blocks)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Default order
        match self.try_get_choice() {
            ChoiceResult::Ok(_) => {
                let order: SmallVec<[CardId; 4]> = blockers.iter().copied().collect();
                ChoiceResult::Ok(order)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        match self.try_get_choice() {
            ChoiceResult::Ok(start_idx) => {
                // Discard 'count' cards starting from index
                let discards: SmallVec<[CardId; 7]> = hand.iter().skip(start_idx).take(count).copied().collect();
                ChoiceResult::Ok(discards)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_from_library(&mut self, _view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        self.select_from_slice(valid_cards)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        match self.try_get_choice() {
            ChoiceResult::Ok(start_idx) => {
                let sacrifices: SmallVec<[CardId; 8]> =
                    valid_permanents.iter().skip(start_idx).take(count).copied().collect();
                ChoiceResult::Ok(sacrifices)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Nothing to do
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        self.game_ended = true;
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Remote
    }
}
