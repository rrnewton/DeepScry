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
use crate::game::controller::{
    format_card_choices, format_spell_ability_choices, sort_spell_abilities, ChoiceContext, ChoiceResult,
    GameStateView, PlayerController,
};
use crate::game::replay_controller::ReplayChoice;
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
    /// Sacrifice selection (indices into valid_permanents)
    Sacrifice(Vec<usize>),
    /// Mode selection (indices into available modes)
    Modes(Vec<usize>),
}

impl PendingChoice {
    /// Convert a PendingChoice to a ReplayChoice using a ChoiceContext for index resolution.
    ///
    /// PendingChoice stores indices (from the UI), while ReplayChoice stores resolved
    /// CardIds/SpellAbilities. The ChoiceContext provides the mapping from indices to
    /// the actual game objects that were presented to the user.
    ///
    /// This is the single conversion point used by both local and network rewind/replay.
    pub fn to_replay_choice(&self, context: Option<&ChoiceContext>) -> ReplayChoice {
        match self {
            PendingChoice::SpellAbility(opt_idx) => {
                if let Some(ChoiceContext::SpellAbility { available, .. }) = context {
                    match opt_idx {
                        None | Some(0) => ReplayChoice::SpellAbility(None),
                        Some(idx) => {
                            let ability_idx = idx - 1;
                            if ability_idx < available.len() {
                                ReplayChoice::SpellAbility(Some(available[ability_idx].clone()))
                            } else {
                                ReplayChoice::SpellAbility(None)
                            }
                        }
                    }
                } else {
                    ReplayChoice::SpellAbility(None)
                }
            }
            PendingChoice::Targets(indices) => {
                if let Some(ChoiceContext::Targets { valid_targets, .. }) = context {
                    let targets: SmallVec<[CardId; 4]> =
                        indices.iter().filter_map(|i| valid_targets.get(*i).copied()).collect();
                    ReplayChoice::Targets(targets)
                } else {
                    ReplayChoice::Targets(SmallVec::new())
                }
            }
            PendingChoice::ManaSources(indices) => {
                if let Some(ChoiceContext::ManaSources { available_sources, .. }) = context {
                    let sources: SmallVec<[CardId; 8]> = indices
                        .iter()
                        .filter_map(|i| available_sources.get(*i).copied())
                        .collect();
                    ReplayChoice::ManaSources(sources)
                } else {
                    ReplayChoice::ManaSources(SmallVec::new())
                }
            }
            PendingChoice::Attackers(indices) => {
                if let Some(ChoiceContext::Attackers {
                    available_creatures, ..
                }) = context
                {
                    let attackers: SmallVec<[CardId; 8]> = indices
                        .iter()
                        .filter_map(|i| available_creatures.get(*i).copied())
                        .collect();
                    ReplayChoice::Attackers(attackers)
                } else {
                    ReplayChoice::Attackers(SmallVec::new())
                }
            }
            PendingChoice::Blockers(pairs) => {
                if let Some(ChoiceContext::Blockers {
                    available_blockers,
                    attackers,
                    ..
                }) = context
                {
                    let blockers: SmallVec<[(CardId, CardId); 8]> = pairs
                        .iter()
                        .filter_map(|(bi, ai)| {
                            let blocker = available_blockers.get(*bi).copied()?;
                            let attacker = attackers.get(*ai).copied()?;
                            Some((blocker, attacker))
                        })
                        .collect();
                    ReplayChoice::Blockers(blockers)
                } else {
                    ReplayChoice::Blockers(SmallVec::new())
                }
            }
            PendingChoice::DamageOrder(indices) => {
                if let Some(ChoiceContext::DamageOrder { blockers, .. }) = context {
                    let order: SmallVec<[CardId; 4]> =
                        indices.iter().filter_map(|i| blockers.get(*i).copied()).collect();
                    ReplayChoice::DamageOrder(order)
                } else {
                    ReplayChoice::DamageOrder(SmallVec::new())
                }
            }
            PendingChoice::Discard(indices) => {
                if let Some(ChoiceContext::Discard { hand, .. }) = context {
                    let cards: SmallVec<[CardId; 7]> = indices.iter().filter_map(|i| hand.get(*i).copied()).collect();
                    ReplayChoice::Discard(cards)
                } else {
                    ReplayChoice::Discard(SmallVec::new())
                }
            }
            PendingChoice::LibrarySearch(opt_idx) => match opt_idx {
                None => ReplayChoice::LibrarySearch(None),
                Some(idx) => ReplayChoice::LibrarySearch(Some(*idx)),
            },
            PendingChoice::Sacrifice(indices) => {
                if let Some(ChoiceContext::SacrificePermanents { valid_permanents, .. }) = context {
                    let permanents: SmallVec<[CardId; 8]> = indices
                        .iter()
                        .filter_map(|i| valid_permanents.get(*i).copied())
                        .collect();
                    ReplayChoice::Sacrifice(permanents)
                } else {
                    ReplayChoice::Sacrifice(SmallVec::new())
                }
            }
            PendingChoice::Modes(indices) => {
                let modes: SmallVec<[usize; 4]> = indices.iter().copied().collect();
                ReplayChoice::Modes(modes)
            }
        }
    }
}

/// Stage one card in a multi-card discard prompt and return the completed
/// `PendingChoice::Discard` once enough cards have been picked.
///
/// The engine's `choose_cards_to_discard` is a single call that expects ALL
/// `count` cards in one response (e.g. Bazaar of Baghdad's "discard 3"). The
/// fancy TUI naturally collects picks one at a time, so the UI accumulates
/// staged hand indices on the side and only commits when `count` are gathered.
///
/// `pick_idx` is the index of the card the user just picked *within the
/// currently displayed (filtered) choice list*. `displayed_to_hand_idx` maps
/// each visible row back to its original `ChoiceContext::Discard::hand` index;
/// the UI rebuilds this map on each render so already-staged cards can be
/// hidden and not re-picked.
///
/// Returns `Some(PendingChoice::Discard(indices))` if the user has now staged
/// `count` cards (and `staged` is left empty for the next prompt). Returns
/// `None` if more cards still need to be picked, or if `pick_idx` was out of
/// range for `displayed_to_hand_idx`.
pub fn stage_discard_pick(
    staged: &mut Vec<usize>,
    displayed_to_hand_idx: &[usize],
    pick_idx: usize,
    count: usize,
) -> Option<PendingChoice> {
    let &hand_idx = displayed_to_hand_idx.get(pick_idx)?;
    staged.push(hand_idx);
    if staged.len() < count {
        None
    } else {
        Some(PendingChoice::Discard(std::mem::take(staged)))
    }
}

/// Toggle one creature in the multi-attacker selection prompt.
///
/// The engine's `choose_attackers` is a single call that expects ALL chosen
/// attackers in one response (e.g. attacking with both Triskelion and Sengir
/// Vampire in the same combat). The tui_game.html UI naturally collects clicks
/// one creature at a time, so we accumulate `staged` indices client-side and
/// only commit when the user picks "Done".
///
/// `creature_idx` is the index into `ChoiceContext::Attackers::available_creatures`.
/// If the creature is not yet staged, it is added; if it was already staged, it
/// is removed (toggle semantics so the user can correct mis-clicks before
/// submitting). Native TUI's `choose_attackers` also displays an `[X]` marker
/// for staged creatures, but is add-only — tui_game.html improves on that by
/// allowing un-staging.
///
/// Returns `true` if `creature_idx` is now staged, `false` if it was un-staged.
pub fn toggle_staged_attacker(staged: &mut Vec<usize>, creature_idx: usize) -> bool {
    if let Some(pos) = staged.iter().position(|&i| i == creature_idx) {
        staged.remove(pos);
        false
    } else {
        staged.push(creature_idx);
        true
    }
}

/// Human controller for WASM/browser gameplay
///
/// This controller implements the event-driven input pattern:
/// - When choices are needed and no pending choice exists, returns `NeedInput`
/// - When a pending choice has been set, consumes and returns it
///
/// The pending choice is typically set by JavaScript event handlers after
/// the user makes a selection in the UI.
#[derive(Clone)]
pub struct WasmHumanController {
    player_id: PlayerId,
    /// The next choice to return (set by UI before resuming game)
    /// Made pub(crate) so fancy_tui can access it for replay pattern
    pub(crate) pending_choice: Option<PendingChoice>,
    /// The context from the last NeedInput response.
    /// This is crucial for target selection: when the user selects targets by INDEX,
    /// we need to map those indices to CardIds using the ORIGINAL valid_targets list
    /// that was shown to the user, NOT the current valid_targets which may have changed.
    pending_context: Option<ChoiceContext>,
}

impl WasmHumanController {
    /// Create a new WASM human controller
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            pending_choice: None,
            pending_context: None,
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
        // Sort abilities in canonical order: PlayLand, CastSpell, ActivateAbility
        let sorted = sort_spell_abilities(available);

        // Check for pending choice
        if let Some(PendingChoice::SpellAbility(choice_idx)) = self.pending_choice.take() {
            return match choice_idx {
                None => ChoiceResult::Ok(None),    // Pass
                Some(0) => ChoiceResult::Ok(None), // Index 0 is also pass
                Some(idx) => {
                    let ability_idx = idx - 1;
                    if ability_idx < sorted.len() {
                        ChoiceResult::Ok(Some(sorted[ability_idx].clone()))
                    } else {
                        ChoiceResult::Ok(None) // Invalid index, treat as pass
                    }
                }
            };
        }

        // Auto-pass when there are no playable options
        // This is important for network mode to avoid unnecessary waiting
        if sorted.is_empty() {
            return ChoiceResult::Ok(None);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
            available: sorted.clone(),
            formatted_choices: format_spell_ability_choices(view, &sorted),
        })
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        _min_targets: usize,
        _max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // The pending-choice path already maps a VECTOR of indices, so a variable
        // target count (Fireball) round-trips; the JS multi-select UX for picking
        // how many is a best-effort follow-up (mtg-tyvcn). min/max are advisory.
        // Check for pending choice
        if let Some(PendingChoice::Targets(indices)) = self.pending_choice.take() {
            // CRITICAL: Use the ORIGINAL valid_targets from pending_context, NOT the current
            // valid_targets passed in. Between showing the UI and processing the choice,
            // the valid_targets list could have changed (e.g., a creature gained counters,
            // making it invalid for "destroy target creature with no counters").
            // The user selected indices into the ORIGINAL list, so we must use that list.
            let original_targets = if let Some(ChoiceContext::Targets {
                valid_targets: stored_targets,
                ..
            }) = &self.pending_context
            {
                stored_targets.as_slice()
            } else {
                // Fallback to current list if no context stored (shouldn't happen)
                log::warn!(
                    target: "human_controller",
                    "choose_targets: No pending_context found, falling back to current valid_targets"
                );
                valid_targets
            };

            let indices_len = indices.len();
            log::debug!(
                target: "human_controller",
                "choose_targets: indices={:?}, original_targets={:?}, current_targets={:?}",
                &indices,
                original_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>(),
                valid_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
            );

            let targets: SmallVec<[CardId; 4]> = indices
                .into_iter()
                .filter_map(|i| original_targets.get(i).copied())
                .collect();

            log::debug!(
                target: "human_controller",
                "choose_targets: resolved targets={:?}",
                targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
            );

            if targets.is_empty() && indices_len > 0 {
                log::warn!(
                    target: "human_controller",
                    "choose_targets: User selection resulted in empty targets! indices={:?} original_targets={:?}",
                    indices_len,
                    original_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
                );
            }

            // Defensive validation: verify resolved targets are still valid
            // With deterministic replay (B1 fix), this should always pass.
            // If it doesn't, it indicates a replay divergence bug.
            for &target_id in &targets {
                if !valid_targets.contains(&target_id) {
                    log::warn!(
                        target: "human_controller",
                        "choose_targets: Resolved target {:?} from pending_context is NOT in current valid_targets! \
                         This may indicate a replay divergence. original_targets={:?}, current_targets={:?}",
                        target_id.as_u32(),
                        original_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>(),
                        valid_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
                    );
                }
            }

            // Clear the context after use
            self.pending_context = None;

            return ChoiceResult::Ok(targets);
        }

        // No pending choice - store context and request input
        let context = ChoiceContext::Targets {
            spell_id: spell,
            valid_targets: valid_targets.to_vec(),
            formatted_targets: format_card_choices(view, valid_targets, self.player_id),
        };
        self.pending_context = Some(context.clone());
        ChoiceResult::NeedInput(context)
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
            formatted_sources: format_card_choices(view, available_sources, self.player_id),
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
            formatted_creatures: format_card_choices(view, available_creatures, self.player_id),
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
            formatted_blockers: format_card_choices(view, available_blockers, self.player_id),
            formatted_attackers: format_card_choices(view, attackers, self.player_id),
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
            formatted_blockers: format_card_choices(view, blockers, self.player_id),
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
            formatted_hand: format_card_choices(view, hand, self.player_id),
        })
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Check for pending choice
        if let Some(PendingChoice::LibrarySearch(choice)) = self.pending_choice.take() {
            return match choice {
                None => ChoiceResult::Ok(None), // Fail to find
                Some(idx) => {
                    if idx < valid_cards.len() {
                        ChoiceResult::Ok(Some(idx))
                    } else {
                        ChoiceResult::Ok(None)
                    }
                }
            };
        }

        // No pending choice - request input
        // Note: We no longer have CardIds, so we provide indices and formatted names
        let formatted_cards: Vec<String> = valid_cards.iter().map(|def| def.name.to_string()).collect();
        ChoiceResult::NeedInput(ChoiceContext::LibrarySearch {
            valid_cards: vec![], // CardIds not available in new architecture
            formatted_cards,
        })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::Sacrifice(indices)) = self.pending_choice.take() {
            let sacrifices: SmallVec<[CardId; 8]> = indices
                .into_iter()
                .filter_map(|i| valid_permanents.get(i).copied())
                .collect();
            return ChoiceResult::Ok(sacrifices);
        }

        // No pending choice - request input
        ChoiceResult::NeedInput(ChoiceContext::SacrificePermanents {
            valid_permanents: valid_permanents.to_vec(),
            count,
            card_type_description: card_type_description.to_string(),
            formatted_permanents: format_card_choices(view, valid_permanents, self.player_id),
        })
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // WASM human controller: auto-untap everything for now
        // TODO: Implement interactive UI for this choice
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Check for pending choice
        if let Some(PendingChoice::Modes(indices)) = self.pending_choice.take() {
            // Use original mode count from pending_context for validation
            let original_count = if let Some(ChoiceContext::Modes { formatted_modes, .. }) = &self.pending_context {
                formatted_modes.len()
            } else {
                mode_descriptions.len()
            };

            let modes: SmallVec<[usize; 4]> = indices.into_iter().filter(|&i| i < original_count).collect();

            // Clear context after use
            self.pending_context = None;

            return ChoiceResult::Ok(modes);
        }

        // No pending choice - store context and request input
        let context = ChoiceContext::Modes {
            spell_id,
            mode_count,
            min_modes,
            can_repeat,
            formatted_modes: mode_descriptions.to_vec(),
        };
        self.pending_context = Some(context.clone());
        ChoiceResult::NeedInput(context)
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

    fn wants_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)]
mod tests {
    use super::*;
    use crate::core::EntityId;

    #[test]
    fn test_human_controller_auto_passes_with_no_available() {
        let player_id = EntityId::new(1);
        let mut controller = WasmHumanController::new(player_id);

        // Create a minimal game state for testing
        let game = crate::game::GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
        let view = crate::game::GameStateView::new(&game, player_id);

        // When no abilities are available, should auto-pass (network optimization)
        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::Ok(None)));
    }

    #[test]
    fn test_human_controller_needs_input_with_available() {
        use crate::core::SpellAbility;

        let player_id = EntityId::new(1);
        let mut controller = WasmHumanController::new(player_id);

        // Create a minimal game state for testing
        let game = crate::game::GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
        let view = crate::game::GameStateView::new(&game, player_id);

        // With available abilities but no pending choice, should return NeedInput
        let available = vec![SpellAbility::PlayLand {
            card_id: EntityId::new(42),
        }];
        let result = controller.choose_spell_ability_to_play(&view, &available);
        assert!(matches!(result, ChoiceResult::NeedInput(_)));
    }

    #[test]
    fn test_stage_discard_pick_count_one_commits_immediately() {
        // count=1 (e.g. a single discard cost): the very first pick commits.
        let mut staged = Vec::new();
        let displayed_to_hand_idx = vec![0_usize, 1, 2];
        let result = stage_discard_pick(&mut staged, &displayed_to_hand_idx, 1, 1);
        match result {
            Some(PendingChoice::Discard(idxs)) => assert_eq!(idxs, vec![1]),
            _ => panic!("expected committed PendingChoice::Discard, got {:?}", result),
        }
        assert!(staged.is_empty(), "staged should be drained on commit");
    }

    #[test]
    fn test_stage_discard_pick_three_cards_stages_then_commits() {
        // Regression for Bazaar of Baghdad ("draw 2, discard 3"): the engine
        // asks for ALL 3 cards in one call, so the UI must stage the first 2
        // picks and only submit `PendingChoice::Discard` on the 3rd.
        let mut staged: Vec<usize> = Vec::new();

        // Initial display: 5-card hand (indices 0..=4 in `formatted_hand`).
        let displayed = vec![0_usize, 1, 2, 3, 4];

        // Pick 1: user selects displayed row 2 (hand_idx 2). Should stage, not commit.
        assert!(stage_discard_pick(&mut staged, &displayed, 2, 3).is_none());
        assert_eq!(staged, vec![2]);

        // After re-render the UI hides the staged card. The new map skips hand_idx 2.
        let displayed = vec![0_usize, 1, 3, 4];

        // Pick 2: user selects displayed row 0 (hand_idx 0). Still staging.
        assert!(stage_discard_pick(&mut staged, &displayed, 0, 3).is_none());
        assert_eq!(staged, vec![2, 0]);

        let displayed = vec![1_usize, 3, 4];

        // Pick 3: user selects displayed row 2 (hand_idx 4). NOW it commits.
        let result = stage_discard_pick(&mut staged, &displayed, 2, 3);
        match result {
            Some(PendingChoice::Discard(idxs)) => assert_eq!(idxs, vec![2, 0, 4]),
            _ => panic!("expected committed PendingChoice::Discard, got {:?}", result),
        }
        assert!(staged.is_empty(), "staged should be drained on commit");
    }

    #[test]
    fn test_toggle_staged_attacker_adds_then_removes() {
        let mut staged: Vec<usize> = Vec::new();
        // First pick: stage attacker at idx 0.
        assert!(toggle_staged_attacker(&mut staged, 0));
        assert_eq!(staged, vec![0]);
        // Second pick: stage attacker at idx 2.
        assert!(toggle_staged_attacker(&mut staged, 2));
        assert_eq!(staged, vec![0, 2]);
        // Third pick: re-pick idx 0 — should un-stage it.
        assert!(!toggle_staged_attacker(&mut staged, 0));
        assert_eq!(staged, vec![2]);
        // Fourth pick: re-pick idx 0 — re-stages it (toggle).
        assert!(toggle_staged_attacker(&mut staged, 0));
        assert_eq!(staged, vec![2, 0]);
    }

    #[test]
    fn test_toggle_staged_attacker_multiple_attackers_regression() {
        // Regression for bug-multi-attacker-selection: in tui_game.html, picking
        // Triskelion then Sengir Vampire used to overwrite the first selection
        // because the UI committed `vec![idx]` immediately on each click.
        // After the fix, both stay staged until the user picks "Done".
        let mut staged: Vec<usize> = Vec::new();
        // Triskelion at available_creatures idx 0.
        toggle_staged_attacker(&mut staged, 0);
        // Sengir Vampire at available_creatures idx 1.
        toggle_staged_attacker(&mut staged, 1);
        // Both creatures are staged for attack — this used to be impossible.
        assert_eq!(staged, vec![0, 1], "both attackers must remain staged");
    }

    #[test]
    fn test_stage_discard_pick_out_of_range_is_noop() {
        let mut staged = vec![0];
        let displayed = vec![1_usize, 2];
        // pick_idx 5 is out of range for a 2-row display.
        let result = stage_discard_pick(&mut staged, &displayed, 5, 3);
        assert!(result.is_none());
        // staged is unchanged when the pick is invalid.
        assert_eq!(staged, vec![0]);
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

    /// Regression test for `bug-chaos-orb-no-destroy` / `bug-strip-mine-no-destroy`:
    /// `PendingChoice::Targets(indices)` carries RAW indices into the `valid_targets`
    /// list. Callers that present a UI with "No target" prepended at index 0 (such
    /// as `wasm/fancy_tui.rs::select_current_choice`) MUST subtract 1 before
    /// storing the user's pick — otherwise every target selection silently drops
    /// the chosen card and the destroy/exile/tap/etc. effect fizzles.
    ///
    /// This test pins down the contract so a future refactor of
    /// `select_current_choice` cannot silently regress it.
    #[test]
    fn test_choose_targets_uses_raw_valid_target_indices() {
        use crate::core::SpellAbility;
        use crate::game::controller::PlayerController;

        let player_id = EntityId::new(1);
        let mut controller = WasmHumanController::new(player_id);

        let game = crate::game::GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
        let view = crate::game::GameStateView::new(&game, player_id);

        // Two valid targets — exactly the situation where the engine prompts
        // the user (single-target lists are auto-selected by the priority loop).
        let target_a = EntityId::new(100);
        let target_b = EntityId::new(200);
        let valid_targets = [target_a, target_b];

        // First call: no pending choice → controller asks for input and stores
        // the context (mirroring what the WASM TUI sees on first prompt).
        let spell = EntityId::new(50);
        let _ = controller.choose_targets(&view, spell, &valid_targets, 1, 1);
        assert!(controller.pending_context.is_some(), "pending_context must be stored");

        // Caller picks index 0 in the engine's `valid_targets` (i.e. the first
        // real target). In the WASM UI this corresponds to clicking the second
        // visible row, since "No target" occupies row 0; `select_current_choice`
        // is responsible for the -1 conversion.
        controller.set_pending_choice(PendingChoice::Targets(vec![0]));
        let result = controller.choose_targets(&view, spell, &valid_targets, 1, 1);
        match result {
            ChoiceResult::Ok(targets) => {
                assert_eq!(
                    targets.as_slice(),
                    &[target_a],
                    "raw index 0 must map to first valid target"
                );
            }
            _ => panic!("expected Ok with first target, got {:?}", result),
        }

        // Re-prime context, then verify raw index 1 → target_b.
        let _ = controller.choose_targets(&view, spell, &valid_targets, 1, 1);
        controller.set_pending_choice(PendingChoice::Targets(vec![1]));
        let result = controller.choose_targets(&view, spell, &valid_targets, 1, 1);
        match result {
            ChoiceResult::Ok(targets) => {
                assert_eq!(
                    targets.as_slice(),
                    &[target_b],
                    "raw index 1 must map to second valid target"
                );
            }
            _ => panic!("expected Ok with second target, got {:?}", result),
        }

        // And the empty-vec convention means "no target chosen" (fizzle).
        let _ = controller.choose_targets(&view, spell, &valid_targets, 1, 1);
        controller.set_pending_choice(PendingChoice::Targets(vec![]));
        let result = controller.choose_targets(&view, spell, &valid_targets, 1, 1);
        match result {
            ChoiceResult::Ok(targets) => {
                assert!(targets.is_empty(), "empty Targets vec must mean 'no target'");
            }
            _ => panic!("expected Ok with empty targets, got {:?}", result),
        }

        // Suppress unused-import warnings on builds without the wasm-network
        // feature flag (`SpellAbility` is only used to keep the imports parallel
        // to the surrounding tests).
        let _ = SpellAbility::PlayLand { card_id: spell };
    }
}
