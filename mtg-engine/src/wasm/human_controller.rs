//! WASM Human Controller
//!
//! This controller implements human input for browser-based gameplay.
//! It uses the `ChoiceResult::NeedInput` pattern to signal when the game
//! should pause for user input.
//!
//! ## Design
//!
//! The controller maintains a pending choice that can be set from JavaScript
//! before resuming the game. When a choice is requested:
//!
//! 1. If a pending choice exists, return it immediately
//! 2. Otherwise, return `NeedInput` with context about what's needed
//!
//! The game loop will then pause, the UI displays options, and when the user
//! makes a selection, JavaScript calls back to set the pending choice before
//! resuming.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;

/// A choice made by the human player, ready to be consumed
#[derive(Debug, Clone)]
pub enum PendingChoice {
    /// Spell ability selection (index into available, or None for pass)
    SpellAbility(Option<usize>),
    /// Target selection (indices into valid_targets)
    Targets(Vec<usize>),
    /// Mana source selection (indices into available_sources)
    ManaSources(Vec<usize>),
    /// Attacker selection (indices into available_creatures)
    Attackers(Vec<usize>),
    /// Blocker selection (pairs of (blocker_idx, attacker_idx))
    Blockers(Vec<(usize, usize)>),
    /// Damage assignment order (indices into blockers, in order)
    DamageOrder(Vec<usize>),
    /// Discard selection (indices into hand)
    Discard(Vec<usize>),
    /// Library search result (index into valid_cards, or None to fail)
    LibrarySearch(Option<usize>),
}

/// Human controller for WASM/browser gameplay
///
/// This controller implements the event-driven input pattern:
/// - When choices are needed and no pending choice exists, returns `NeedInput`
/// - When a pending choice has been set, consumes and returns it
///
/// The pending choice is typically set by JavaScript event handlers after
/// the user makes a selection in the UI.
pub struct WasmHumanController {
    player_id: PlayerId,
    /// The next choice to return (set by UI before resuming game)
    pending_choice: Option<PendingChoice>,
}

impl WasmHumanController {
    /// Create a new WASM human controller
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            pending_choice: None,
        }
    }

    /// Set the pending choice (called from JavaScript after user makes selection)
    pub fn set_pending_choice(&mut self, choice: PendingChoice) {
        self.pending_choice = Some(choice);
    }

    /// Check if a pending choice is available
    pub fn has_pending_choice(&self) -> bool {
        self.pending_choice.is_some()
    }

    /// Clear any pending choice
    pub fn clear_pending_choice(&mut self) {
        self.pending_choice = None;
    }

    /// Helper to format spell abilities for display
    fn format_spell_abilities(available: &[SpellAbility], view: &GameStateView) -> Vec<String> {
        let mut formatted = vec!["Pass (do nothing)".to_string()];
        for ability in available {
            let s = match ability {
                SpellAbility::PlayLand { card_id } => {
                    let name = view.get_card(*card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                    format!("Play land: {}", name)
                }
                SpellAbility::CastSpell { card_id } => {
                    let name = view.get_card(*card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let cost = view
                        .get_card(*card_id)
                        .map(|c| c.mana_cost.to_string())
                        .unwrap_or_default();
                    format!("Cast: {} ({})", name, cost)
                }
                SpellAbility::ActivateAbility { card_id, .. } => {
                    let name = view.get_card(*card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                    format!("Activate ability of {}", name)
                }
            };
            formatted.push(s);
        }
        formatted
    }

    /// Helper to format cards for display
    fn format_cards(cards: &[CardId], view: &GameStateView) -> Vec<String> {
        cards
            .iter()
            .map(|id| {
                view.get_card(*id)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|| format!("Unknown ({})", id))
            })
            .collect()
    }
}

impl PlayerController for WasmHumanController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Check for pending choice
        if let Some(PendingChoice::SpellAbility(choice_idx)) = self.pending_choice.take() {
            return match choice_idx {
                None => ChoiceResult::Ok(None),    // Pass
                Some(0) => ChoiceResult::Ok(None), // Index 0 is also pass
                Some(idx) => {
                    let ability_idx = idx - 1;
                    if ability_idx < available.len() {
                        ChoiceResult::Ok(Some(available[ability_idx].clone()))
                    } else {
                        ChoiceResult::Ok(None) // Invalid index, treat as pass
                    }
                }
            };
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
            available: available.to_vec(),
            formatted_choices: Self::format_spell_abilities(available, view),
        })
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Check for pending choice
        if let Some(PendingChoice::Targets(indices)) = self.pending_choice.take() {
            let targets: SmallVec<[CardId; 4]> = indices
                .into_iter()
                .filter_map(|i| valid_targets.get(i).copied())
                .collect();
            return ChoiceResult::Ok(targets);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::Targets {
            spell_id: spell,
            valid_targets: valid_targets.to_vec(),
            formatted_targets: Self::format_cards(valid_targets, view),
        })
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::ManaSources(indices)) = self.pending_choice.take() {
            let sources: SmallVec<[CardId; 8]> = indices
                .into_iter()
                .filter_map(|i| available_sources.get(i).copied())
                .collect();
            return ChoiceResult::Ok(sources);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::ManaSources {
            cost: *cost,
            available_sources: available_sources.to_vec(),
            formatted_sources: Self::format_cards(available_sources, view),
        })
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::Attackers(indices)) = self.pending_choice.take() {
            let attackers: SmallVec<[CardId; 8]> = indices
                .into_iter()
                .filter_map(|i| available_creatures.get(i).copied())
                .collect();
            return ChoiceResult::Ok(attackers);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::Attackers {
            available_creatures: available_creatures.to_vec(),
            formatted_creatures: Self::format_cards(available_creatures, view),
        })
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::Blockers(pairs)) = self.pending_choice.take() {
            let blocks: SmallVec<[(CardId, CardId); 8]> = pairs
                .into_iter()
                .filter_map(|(blocker_idx, attacker_idx)| {
                    let blocker = available_blockers.get(blocker_idx).copied()?;
                    let attacker = attackers.get(attacker_idx).copied()?;
                    Some((blocker, attacker))
                })
                .collect();
            return ChoiceResult::Ok(blocks);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::Blockers {
            available_blockers: available_blockers.to_vec(),
            attackers: attackers.to_vec(),
            formatted_blockers: Self::format_cards(available_blockers, view),
            formatted_attackers: Self::format_cards(attackers, view),
        })
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Check for pending choice
        if let Some(PendingChoice::DamageOrder(indices)) = self.pending_choice.take() {
            let order: SmallVec<[CardId; 4]> = indices.into_iter().filter_map(|i| blockers.get(i).copied()).collect();
            return ChoiceResult::Ok(order);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::DamageOrder {
            attacker,
            blockers: blockers.to_vec(),
            formatted_blockers: Self::format_cards(blockers, view),
        })
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Check for pending choice
        if let Some(PendingChoice::Discard(indices)) = self.pending_choice.take() {
            let discards: SmallVec<[CardId; 7]> = indices.into_iter().filter_map(|i| hand.get(i).copied()).collect();
            return ChoiceResult::Ok(discards);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::Discard {
            hand: hand.to_vec(),
            count,
            formatted_hand: Self::format_cards(hand, view),
        })
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // Check for pending choice
        if let Some(PendingChoice::LibrarySearch(choice)) = self.pending_choice.take() {
            return match choice {
                None => ChoiceResult::Ok(None), // Fail to find
                Some(idx) => {
                    if idx < valid_cards.len() {
                        ChoiceResult::Ok(Some(valid_cards[idx]))
                    } else {
                        ChoiceResult::Ok(None)
                    }
                }
            };
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::LibrarySearch {
            valid_cards: valid_cards.to_vec(),
            formatted_cards: Self::format_cards(valid_cards, view),
        })
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Nothing to do
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Nothing to do
    }

    fn get_controller_type(&self) -> ControllerType {
        // Use Tui as the closest match for human player
        ControllerType::Tui
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;

    #[test]
    fn test_human_controller_needs_input_without_pending() {
        let player_id = EntityId::new(1);
        let mut controller = WasmHumanController::new(player_id);

        // Create a minimal game state for testing
        let game = crate::game::GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
        let view = crate::game::GameStateView::new(&game, player_id);

        // Without pending choice, should return NeedInput
        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::NeedInput(_)));
    }

    #[test]
    fn test_human_controller_returns_pending_choice() {
        let player_id = EntityId::new(1);
        let mut controller = WasmHumanController::new(player_id);

        // Create a minimal game state for testing
        let game = crate::game::GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
        let view = crate::game::GameStateView::new(&game, player_id);

        // Set pending choice to pass
        controller.set_pending_choice(PendingChoice::SpellAbility(None));

        // Should return the pending choice
        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::Ok(None)));

        // Pending choice should be consumed
        assert!(!controller.has_pending_choice());
    }
}
