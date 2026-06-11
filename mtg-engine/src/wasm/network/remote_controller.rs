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
//! ## Code Sharing Note (mtg-788 C1)
//!
//! The pure index→CardId DECODE logic (attackers / blockers / multi-select
//! subsets / the lethal+remaining-damage CardId-vs-index resolver) is now shared
//! with the native `RemoteController` via `crate::network::choice_decode`. Both
//! controllers must decode the server's `choice_indices` IDENTICALLY or the two
//! shadows desync, so the decode lives in ONE place. Only the divergent glue
//! stays here: the non-blocking `try_get_choice` fetch + `NeedInput` yield, and
//! the genuinely-different bodies (`choose_damage_assignment_order` appends the
//! unlisted blockers; `choose_from_library` carries the server-CardId fallback;
//! `choose_spell_ability_to_play` has the server-ability fallback).

use super::client::SharedNetworkClient;
use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::loader::CardDefinition;
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
    /// Last received library search result CardId (from OpponentChoice).
    /// Stored so choose_from_library can trigger the game loop's
    /// take_library_search_result() fallback path for shadow games where
    /// valid_cards is empty (opponent's unrevealed library cards).
    last_library_search_result: Option<CardId>,
    /// Whether the game has ended
    game_ended: bool,
    /// Saved opponent-choice cursor for the multi-step combat damage
    /// assignment checkpoint/restore (mtg-sfihb). See
    /// `PlayerController::mark_choice_checkpoint`.
    choice_checkpoint: Option<u64>,
}

impl WasmRemoteController {
    /// Create a new remote controller
    pub fn new(player_id: PlayerId, network_client: SharedNetworkClient) -> Self {
        Self {
            player_id,
            network_client,
            last_spell_ability: None,
            last_library_search_result: None,
            game_ended: false,
            choice_checkpoint: None,
        }
    }

    /// Try to get the next opponent choice
    ///
    /// Returns the choice indices, or NeedInput if none available.
    fn try_get_choice(&mut self) -> ChoiceResult<Vec<usize>> {
        // Check if game has ended
        let client = self.network_client.borrow();
        if client.state() == super::client::NetworkState::GameEnded {
            drop(client);
            self.game_ended = true;
            log::debug!("WasmRemoteController: Game ended, returning ExitGame");
            return ChoiceResult::ExitGame;
        }
        drop(client);

        // Try to pop an opponent choice
        let mut client = self.network_client.borrow_mut();
        if let Some(choice) = client.pop_opponent_choice() {
            log::debug!(
                "WasmRemoteController: Opponent chose indices {:?} (seq={}, {})",
                choice.choice_indices,
                choice.choice_seq,
                choice.description
            );
            // Store spell_ability for choose_spell_ability_to_play to use
            self.last_spell_ability = choice.spell_ability;
            // Store library_search_result for choose_from_library / take_library_search_result
            self.last_library_search_result = choice.library_search_result;
            ChoiceResult::Ok(choice.choice_indices)
        } else {
            log::debug!("WasmRemoteController: No opponent choice available, returning NeedInput");
            ChoiceResult::NeedInput(waiting_for_opponent_context())
        }
    }

    /// Helper to select from a slice based on choice indices (uses first index)
    fn select_from_slice<T: Clone>(&mut self, items: &[T]) -> ChoiceResult<Option<T>> {
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let idx = indices.first().copied().unwrap_or(items.len());
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

    fn mark_choice_checkpoint(&mut self) {
        // Save the current opponent-choice consumption cursor so a mid-pass
        // NeedInput during combat damage assignment can rewind to here
        // (mtg-sfihb).
        self.choice_checkpoint = Some(self.network_client.borrow().opponent_choice_cursor());
    }

    fn restore_choice_checkpoint(&mut self) {
        if let Some(saved) = self.choice_checkpoint.take() {
            self.network_client.borrow_mut().set_opponent_choice_cursor(saved);
        }
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let idx = indices.first().copied().unwrap_or(0);
                if idx == 0 {
                    return ChoiceResult::Ok(None); // Pass
                }

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
        _min_targets: usize,
        _max_targets: usize,
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
        // Server sends ALL source positions (0-indexed) in a single OpponentChoice.
        // Multi-mana costs (e.g. {R}{R}) send multiple indices (e.g. [0, 1]).
        // We must return ALL selected sources, not just the first.
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => ChoiceResult::Ok(crate::network::decode_subset(&indices, available_sources)),
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
        // Server sends indices: [0] = no attackers, [N, M, ...] = creature indices (1-based)
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                ChoiceResult::Ok(crate::network::decode_attackers(&indices, available_creatures))
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
        // Server sends indices: [0] = no blockers, [N, M, ...] = encoded blocker-attacker pairs (1-based)
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                ChoiceResult::Ok(crate::network::decode_blockers(&indices, available_blockers, attackers))
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
        // Server sends indices specifying the damage assignment order
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let mut result = SmallVec::new();
                for idx in indices {
                    if idx < blockers.len() {
                        result.push(blockers[idx]);
                    }
                }
                // Add remaining blockers
                if result.len() < blockers.len() {
                    for &blocker in blockers {
                        if !result.contains(&blocker) {
                            result.push(blocker);
                        }
                    }
                }
                ChoiceResult::Ok(result)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_blocker_for_lethal_damage(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        killable_blockers: &[(CardId, i32)],
        _remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        // SMART damage assignment (mtg-418): the server sends the index AND the
        // authoritative CardId via target_card_ids. Prefer the CardId — index-based
        // lookup is unreliable in shadow state where blocker ordering may differ.
        //
        // CRITICAL: We MUST consume an OpponentChoice from the queue here, even
        // though we have a default impl on the trait. Failing to consume the message
        // would leave it in the queue and shift every subsequent OpponentChoice by
        // one, causing immediate state-hash desync (the bug described in mtg-418).
        // Peek at the next queued OpponentChoice (which try_get_choice will pop)
        // and capture its target_card_ids before consuming the choice.
        let target_card_ids = self
            .network_client
            .borrow()
            .peek_opponent_choice()
            .and_then(|c| c.target_card_ids.clone());

        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let valid: SmallVec<[CardId; 8]> = killable_blockers.iter().map(|(id, _)| *id).collect();
                match crate::network::resolve_combat_blocker(
                    &indices,
                    target_card_ids.as_deref(),
                    &valid,
                    "lethal-damage",
                ) {
                    Ok(id) => ChoiceResult::Ok(id),
                    Err(msg) => {
                        log::error!("{}", msg);
                        ChoiceResult::Error(msg)
                    }
                }
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_blocker_for_remaining_damage(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        remaining_blockers: &[CardId],
        _remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        // Mirror of choose_blocker_for_lethal_damage above (mtg-418).
        let target_card_ids = self
            .network_client
            .borrow()
            .peek_opponent_choice()
            .and_then(|c| c.target_card_ids.clone());

        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                match crate::network::resolve_combat_blocker(
                    &indices,
                    target_card_ids.as_deref(),
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
        _count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Server sends indices of cards to discard (multi-select)
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => ChoiceResult::Ok(crate::network::decode_subset(&indices, hand)),
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_scry_order(
        &mut self,
        _view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::controller::ScryDecision> {
        // Server sends indices of cards to put on BOTTOM (in placement order).
        // Cards not in indices stay on top in revealed order.
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let mut bottom = SmallVec::<[CardId; 4]>::new();
                let mut bottom_positions = SmallVec::<[usize; 4]>::new();
                for idx in indices {
                    if idx < revealed.len() {
                        bottom.push(revealed[idx]);
                        bottom_positions.push(idx);
                    } else {
                        let msg = format!(
                            "FATAL DESYNC: WasmRemoteController received invalid scry index {} (only {} revealed)",
                            idx,
                            revealed.len()
                        );
                        log::error!("{}", msg);
                        return ChoiceResult::Error(msg);
                    }
                }
                let mut top_top_down = SmallVec::<[CardId; 4]>::new();
                for (i, &card_id) in revealed.iter().enumerate() {
                    if !bottom_positions.contains(&i) {
                        top_top_down.push(card_id);
                    }
                }
                let top: SmallVec<[CardId; 4]> = top_top_down.into_iter().rev().collect();
                ChoiceResult::Ok(crate::game::controller::ScryDecision { top, bottom })
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_surveil(
        &mut self,
        _view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::controller::SurveilDecision> {
        // Server sends indices of cards to mill to GRAVEYARD (in placement order).
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let mut graveyard = SmallVec::<[CardId; 4]>::new();
                let mut mill_positions = SmallVec::<[usize; 4]>::new();
                for idx in indices {
                    if idx < revealed.len() {
                        graveyard.push(revealed[idx]);
                        mill_positions.push(idx);
                    } else {
                        let msg = format!(
                            "FATAL DESYNC: WasmRemoteController received invalid surveil index {} (only {} revealed)",
                            idx,
                            revealed.len()
                        );
                        log::error!("{}", msg);
                        return ChoiceResult::Error(msg);
                    }
                }
                let mut top_top_down = SmallVec::<[CardId; 4]>::new();
                for (i, &card_id) in revealed.iter().enumerate() {
                    if !mill_positions.contains(&i) {
                        top_top_down.push(card_id);
                    }
                }
                let top: SmallVec<[CardId; 4]> = top_top_down.into_iter().rev().collect();
                ChoiceResult::Ok(crate::game::controller::SurveilDecision { top, graveyard })
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Server sends 1-based name index (0 = decline) in choice_indices,
        // and stores the authoritative CardId in library_search_result.
        // try_get_choice() already captured library_search_result in last_library_search_result.
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let name_idx_raw = indices.first().copied().unwrap_or(0);
                if name_idx_raw == 0 {
                    // Opponent declined the search
                    ChoiceResult::Ok(None)
                } else if self.last_library_search_result.is_some() {
                    // Server provided authoritative CardId. Return Some(_) (non-None) to
                    // trigger the game loop's take_library_search_result() fallback path,
                    // which handles the case where valid_cards is empty (shadow game).
                    log::debug!(
                        "WasmRemoteController::choose_from_library: using server CardId {:?} (indices={:?})",
                        self.last_library_search_result,
                        indices
                    );
                    ChoiceResult::Ok(Some(0)) // Placeholder; game loop will use take_library_search_result()
                } else if !valid_cards.is_empty() {
                    // Fallback: use index into local valid_cards list
                    let card_idx = (name_idx_raw - 1).min(valid_cards.len() - 1);
                    ChoiceResult::Ok(Some(card_idx))
                } else {
                    log::warn!(
                        "WasmRemoteController::choose_from_library: no library_search_result and valid_cards empty (indices={:?})",
                        indices
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

    fn take_library_search_result(&mut self) -> Option<CardId> {
        self.last_library_search_result.take()
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        _count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Server sends indices of permanents to sacrifice (multi-select)
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => ChoiceResult::Ok(crate::network::decode_subset(&indices, valid_permanents)),
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
        }
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Network: auto-untap everything for now
        // TODO: Add network protocol support for this choice
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Server sends mode indices
        match self.try_get_choice() {
            ChoiceResult::Ok(indices) => {
                let mut modes = SmallVec::new();
                for idx in indices.into_iter().take(mode_count) {
                    if idx < mode_descriptions.len() {
                        modes.push(idx);
                    }
                }
                ChoiceResult::Ok(modes)
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
