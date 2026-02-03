//! Network-aware choice helpers
//!
//! These helper methods wrap controller choice calls to integrate with the
//! pre-choice hook system for network mode. In non-network mode, they simply
//! forward to the controller. In network mode, they call the pre-choice hook
//! first and either use the returned choice directly (for remote players)
//! or proceed to call the controller (for local players).
//!
//! ## Design
//!
//! Each helper follows this pattern:
//! 1. Check if network mode is enabled (`is_network_mode()`)
//! 2. If not, call controller directly
//! 3. If yes, call pre-choice hook (this mutably borrows self.game)
//!    - `AskController`: assert NOT remote, call controller
//!    - `UseChoice(raw)`: assert IS remote, convert indices to choice
//!    - `Exit`: return ExitGame
//!
//! ## Borrow Management
//!
//! These helpers create `GameStateView` internally AFTER the hook returns,
//! avoiding borrow conflicts between `&mut self` (for hook) and `&GameStateView`.
//!
//! This maintains the invariant that:
//! - Local players receive `ChoiceRequest` → `AskController`
//! - Remote players receive `OpponentChoice` → `UseChoice`
//!
//! ## Note
//!
//! Some methods are currently unused after the MVar architecture migration but
//! are kept for potential future use or debugging.

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;

use super::{ChoiceKind, GameLoop, PreChoiceResult};

impl<'a> GameLoop<'a> {
    /// Choose a spell ability to play (network-aware)
    ///
    /// In network mode, calls the pre-choice hook first. For remote players,
    /// uses the hook's returned choice directly. For local players, proceeds
    /// to call the controller.
    ///
    /// # Arguments
    /// * `controller` - The player controller
    /// * `viewer_player` - The player ID for creating the game state view
    /// * `available` - The available spell abilities
    pub(super) fn choose_spell_ability_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        let player = controller.player_id();

        // Non-network mode: call controller directly
        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_spell_ability_to_play(&view, available);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;

        // Call hook (mutably borrows self.game)
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::SpellAbility);
        // Hook returned, mutable borrow ended

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(
                    !is_remote,
                    "AskController returned for remote controller (player {:?})",
                    player
                );
                // Create view AFTER hook returns (borrow is released)
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_spell_ability_to_play(&view, available)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(
                    is_remote,
                    "UseChoice returned for non-remote controller (player {:?})",
                    player
                );
                // Index 0 = pass, index N = available[N-1]
                let idx = raw.indices.first().copied().unwrap_or(0);
                if idx == 0 {
                    return ChoiceResult::Ok(None);
                }
                // If server sent the actual spell ability, use it directly
                if let Some(ability) = raw.spell_ability {
                    return ChoiceResult::Ok(Some(ability));
                }
                // Fall back to index-based lookup
                let ability_idx = idx - 1;
                if ability_idx < available.len() {
                    ChoiceResult::Ok(Some(available[ability_idx].clone()))
                } else {
                    log::warn!("Invalid ability index {} (available={})", ability_idx, available.len());
                    ChoiceResult::Ok(None)
                }
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => {
                // Hook not configured but is_network_mode() returned true - shouldn't happen
                unreachable!("is_network_mode() returned true but no hook configured");
            }
        }
    }

    /// Choose targets for a spell (network-aware)
    pub(super) fn choose_targets_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_targets(&view, spell, valid_targets);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::Targets);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_targets(&view, spell, valid_targets)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let mut targets = SmallVec::new();
                for idx in raw.indices {
                    if idx < valid_targets.len() {
                        targets.push(valid_targets[idx]);
                    }
                }
                ChoiceResult::Ok(targets)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose mana sources to pay a cost (network-aware)
    pub(super) fn choose_mana_sources_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_mana_sources_to_pay(&view, cost, available_sources);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::ManaSources);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_mana_sources_to_pay(&view, cost, available_sources)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let mut sources = SmallVec::new();
                for idx in raw.indices {
                    if idx < available_sources.len() {
                        sources.push(available_sources[idx]);
                    }
                }
                ChoiceResult::Ok(sources)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose attackers for combat (network-aware)
    pub(super) fn choose_attackers_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_attackers(&view, available_creatures);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::Attackers);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_attackers(&view, available_creatures)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                // Index 0 = pass, index N = available_creatures[N-1]
                let mut attackers = SmallVec::new();
                for idx in raw.indices {
                    if idx == 0 {
                        continue;
                    }
                    let creature_idx = idx - 1;
                    if creature_idx < available_creatures.len() {
                        attackers.push(available_creatures[creature_idx]);
                    }
                }
                ChoiceResult::Ok(attackers)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose blockers for combat (network-aware)
    pub(super) fn choose_blockers_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_blockers(&view, available_blockers, attackers);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::Blockers);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_blockers(&view, available_blockers, attackers)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                // Index 0 = pass, index N = (blocker_idx, attacker_idx) pair
                let mut blocks = SmallVec::new();
                for idx in raw.indices {
                    if idx == 0 {
                        continue;
                    }
                    let pair_idx = idx - 1;
                    let blocker_idx = pair_idx / attackers.len();
                    let attacker_idx = pair_idx % attackers.len();
                    if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                        blocks.push((available_blockers[blocker_idx], attackers[attacker_idx]));
                    }
                }
                ChoiceResult::Ok(blocks)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose damage assignment order (network-aware)
    pub(super) fn choose_damage_order_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_damage_assignment_order(&view, attacker, blockers);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::DamageOrder);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_damage_assignment_order(&view, attacker, blockers)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let mut result = SmallVec::new();
                for idx in raw.indices {
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
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose cards to discard (network-aware)
    pub(super) fn choose_discard_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_cards_to_discard(&view, hand, count);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::Discard);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_cards_to_discard(&view, hand, count)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let mut discards = SmallVec::new();
                for idx in raw.indices {
                    if idx < hand.len() {
                        discards.push(hand[idx]);
                    }
                }
                ChoiceResult::Ok(discards)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose a card from library (network-aware)
    ///
    /// This is the bridge between game loop (which has CardIds) and controllers
    /// (which work with CardDefinitions). The controller receives CardDefinitions
    /// and returns an index, which we map back to a CardId.
    pub(super) fn choose_from_library_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        valid_cards: &[CardId],
    ) -> ChoiceResult<Option<CardId>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            // Build CardDefinition references for the controller
            let valid_card_definitions: Vec<&crate::loader::CardDefinition> = valid_cards
                .iter()
                .filter_map(|&card_id| self.game.cards.get(card_id).ok().map(|c| &c.definition))
                .collect();
            // Provide CardIds to NetworkController so it can include them in
            // the ChoiceRequest for the coordinator to resolve back to CardId.
            // No-op for non-network controllers (trait default is empty).
            controller.set_pending_library_search_card_ids(valid_cards);
            // Call controller with CardDefinitions, get back index, map to CardId
            let view = GameStateView::new(self.game, viewer_player);
            let result = controller.choose_from_library(&view, &valid_card_definitions);
            let mapped = match result {
                ChoiceResult::Ok(Some(index)) if index < valid_cards.len() => {
                    ChoiceResult::Ok(Some(valid_cards[index]))
                }
                ChoiceResult::Ok(Some(_)) if valid_cards.is_empty() => {
                    // Client shadow game: valid_cards is empty because unrevealed
                    // library cards are not instantiated. The NLC communicated with
                    // the server and stored the authoritative CardId. Retrieve it.
                    let lib_result = controller.take_library_search_result();
                    ChoiceResult::Ok(lib_result)
                }
                ChoiceResult::Ok(_) => ChoiceResult::Ok(None),
                ChoiceResult::ExitGame => ChoiceResult::ExitGame,
                ChoiceResult::UndoRequest(n) => ChoiceResult::UndoRequest(n),
                ChoiceResult::Error(e) => ChoiceResult::Error(e),
                ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            };
            // After library search, sync pending reveals so the card entity
            // exists in the shadow game before move_card is attempted.
            if let ChoiceResult::Ok(Some(_)) = &mapped {
                self.sync_to_action();
            }
            return mapped;
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::FromLibrary);

        let result = match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                // Provide CardIds to NetworkController so it can include them in
                // the ChoiceRequest for the coordinator to resolve back to CardId.
                controller.set_pending_library_search_card_ids(valid_cards);
                // Build CardDefinition references after the hook call to avoid borrow conflict
                let valid_card_definitions: Vec<&crate::loader::CardDefinition> = valid_cards
                    .iter()
                    .filter_map(|&card_id| self.game.cards.get(card_id).ok().map(|c| &c.definition))
                    .collect();
                // Call controller with CardDefinitions, get back index, map to CardId
                let view = GameStateView::new(self.game, viewer_player);
                let choice = controller.choose_from_library(&view, &valid_card_definitions);
                match choice {
                    ChoiceResult::Ok(Some(index)) if index < valid_cards.len() => {
                        ChoiceResult::Ok(Some(valid_cards[index]))
                    }
                    ChoiceResult::Ok(Some(_)) if valid_cards.is_empty() => {
                        // Network mode: valid_cards is empty because unrevealed library cards
                        // are not instantiated in the shadow game. Use the server-authoritative
                        // CardId stored by NetworkLocalController from ChoiceAccepted.
                        ChoiceResult::Ok(controller.take_library_search_result())
                    }
                    ChoiceResult::Ok(_) => ChoiceResult::Ok(None),
                    ChoiceResult::ExitGame => ChoiceResult::ExitGame,
                    ChoiceResult::UndoRequest(n) => ChoiceResult::UndoRequest(n),
                    ChoiceResult::Error(e) => ChoiceResult::Error(e),
                    ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
                }
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                // For library searches, use the server's authoritative library_search_result.
                // The valid_cards list is empty on the client because unrevealed library cards
                // are not instantiated in the shadow game (card slots are `None`).
                ChoiceResult::Ok(raw.library_search_result)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        };

        // CRITICAL: After any library search in network mode, sync pending reveals.
        // The server sends CardRevealed for the library search result BEFORE
        // ChoiceAccepted, so it's in pending_reveals waiting to be processed.
        // We must process it before returning, otherwise move_card will fail
        // with "Entity not found" because the card isn't instantiated yet.
        if let ChoiceResult::Ok(Some(_)) = &result {
            self.sync_to_action();
        }

        result
    }

    /// Choose permanents to sacrifice (network-aware)
    pub(super) fn choose_sacrifice_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_permanents_to_sacrifice(&view, valid_permanents, count, card_type_description);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::Sacrifice);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_permanents_to_sacrifice(&view, valid_permanents, count, card_type_description)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let mut sacrifices = SmallVec::new();
                for idx in raw.indices {
                    if idx < valid_permanents.len() {
                        sacrifices.push(valid_permanents[idx]);
                    }
                }
                ChoiceResult::Ok(sacrifices)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose modes for a modal spell (network-aware)
    pub(super) fn choose_modes_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_modes(&view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::Modes);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_modes(&view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let modes: SmallVec<[usize; 4]> = raw.indices.into_iter().collect();
                ChoiceResult::Ok(modes)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }

    /// Choose permanents to not untap (network-aware)
    pub(super) fn choose_not_untap_with_hook(
        &mut self,
        controller: &mut dyn PlayerController,
        viewer_player: PlayerId,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let player = controller.player_id();

        if !self.is_network_mode() {
            let view = GameStateView::new(self.game, viewer_player);
            return controller.choose_permanents_to_not_untap(&view, may_not_untap_permanents);
        }

        let is_remote = controller.get_controller_type() == ControllerType::Remote;
        let hook_result = self.call_pre_choice_hook(player, ChoiceKind::NotUntap);

        match hook_result {
            Some(PreChoiceResult::AskController) => {
                debug_assert!(!is_remote, "AskController returned for remote controller");
                let view = GameStateView::new(self.game, viewer_player);
                controller.choose_permanents_to_not_untap(&view, may_not_untap_permanents)
            }
            Some(PreChoiceResult::UseChoice(raw)) => {
                debug_assert!(is_remote, "UseChoice returned for non-remote controller");
                let mut result = SmallVec::new();
                for idx in raw.indices {
                    if idx < may_not_untap_permanents.len() {
                        result.push(may_not_untap_permanents[idx]);
                    }
                }
                ChoiceResult::Ok(result)
            }
            Some(PreChoiceResult::Exit) => ChoiceResult::ExitGame,
            None => unreachable!("is_network_mode() returned true but no hook configured"),
        }
    }
}
