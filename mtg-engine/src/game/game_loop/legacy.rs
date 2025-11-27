//! Legacy v1 PlayerAction interface
//!
//! This module contains the original action-based player interface.
//! It is deprecated in favor of the v2 SpellAbility interface.

#![deprecated(note = "Legacy v1 PlayerAction interface. Use v2 SpellAbility interface instead.")]

use crate::core::{CardId, PlayerId};
use crate::Result;

use super::GameLoop;

// Legacy v1 action type (kept for compatibility with dead code)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) enum PlayerAction {
    PlayLand(CardId),
    CastSpell { card_id: CardId, targets: Vec<CardId> },
    TapForMana(CardId),
    DeclareAttacker(CardId),
    DeclareBlocker { blocker: CardId, attackers: Vec<CardId> },
    FinishDeclareAttackers,
    FinishDeclareBlockers,
    PassPriority,
}

impl<'a> GameLoop<'a> {
    /// Get available attackers for a player
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    pub(super) fn get_available_attackers(&self, player_id: PlayerId) -> Vec<PlayerAction> {
        let mut actions = Vec::new();

        // Add finish action
        actions.push(PlayerAction::FinishDeclareAttackers);

        // Find creatures that can attack
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_attacking(card_id)
                {
                    // TODO: Check for summoning sickness
                    actions.push(PlayerAction::DeclareAttacker(card_id));
                }
            }
        }

        actions
    }

    /// Get available blockers for a player
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    pub(super) fn get_available_blockers(&self, player_id: PlayerId) -> Vec<PlayerAction> {
        let mut actions = Vec::new();

        // Add finish action
        actions.push(PlayerAction::FinishDeclareBlockers);

        // Get all attacking creatures
        let attackers = self.game.combat.get_attackers();
        if attackers.is_empty() {
            return actions;
        }

        // Find creatures that can block
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_blocking(card_id)
                {
                    // For each potential blocker, offer to block each attacker
                    // (For simplicity, we only support blocking one attacker at a time)
                    for &attacker in &attackers {
                        actions.push(PlayerAction::DeclareBlocker {
                            blocker: card_id,
                            attackers: vec![attacker],
                        });
                    }
                }
            }
        }

        actions
    }

    /// Get available actions for a player at current game state
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    pub(super) fn get_available_actions(&self, player_id: PlayerId) -> Vec<PlayerAction> {
        let mut actions = Vec::new();

        // Always can pass priority
        actions.push(PlayerAction::PassPriority);

        let current_step = self.game.turn.current_step;

        // Can play lands in main phases if player hasn't played one this turn
        if current_step.can_play_lands() {
            if let Ok(player) = self.game.get_player(player_id) {
                if player.can_play_land() {
                    // Find lands in hand
                    if let Some(zones) = self.game.get_player_zones(player_id) {
                        for &card_id in &zones.hand.cards {
                            if let Ok(card) = self.game.cards.get(card_id) {
                                if card.is_land() {
                                    actions.push(PlayerAction::PlayLand(card_id));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Can tap lands for mana
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.owner == player_id && card.is_land() && !card.tapped {
                    actions.push(PlayerAction::TapForMana(card_id));
                }
            }
        }

        // Can cast spells from hand
        if let Some(zones) = self.game.get_player_zones(player_id) {
            for &card_id in &zones.hand.cards {
                if let Ok(card) = self.game.cards.get(card_id) {
                    // Check if card is castable (not a land)
                    if !card.is_land() {
                        // Check if player has enough mana
                        if let Ok(player) = self.game.get_player(player_id) {
                            if player.mana_pool.can_pay(&card.mana_cost) {
                                actions.push(PlayerAction::CastSpell {
                                    card_id,
                                    targets: vec![],
                                });
                            }
                        }
                    }
                }
            }
        }

        actions
    }

    /// Execute a player action
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    pub(super) fn execute_action(&mut self, player_id: PlayerId, action: &PlayerAction) -> Result<()> {
        if !matches!(action, PlayerAction::PassPriority) {
            let player_name = self.get_player_name(player_id);
            let action_desc = self.describe_action(action);
            self.log_verbose(&format!("{player_name} {action_desc}"));
        }

        match action {
            PlayerAction::PlayLand(card_id) => {
                self.game.play_land(player_id, *card_id)?;
            }
            PlayerAction::TapForMana(card_id) => {
                self.game.tap_for_mana(player_id, *card_id)?;
            }
            PlayerAction::CastSpell { card_id, targets } => {
                // Show spell being cast (added to stack)
                log_if_verbose!(
                    self,
                    "{} casts {}",
                    self.get_player_name(player_id),
                    self.game
                        .cards
                        .get(*card_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown")
                );

                self.game.cast_spell(player_id, *card_id, targets.clone())?;

                // Immediately resolve spell (simplified - no stack interaction yet)
                // Legacy v1 path - no targets chosen, rely on auto-targeting
                self.game.resolve_spell(*card_id, &[])?;

                log_if_verbose!(
                    self,
                    "{} resolves",
                    self.game
                        .cards
                        .get(*card_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown")
                );
            }
            PlayerAction::DeclareAttacker(card_id) => {
                self.game.declare_attacker(player_id, *card_id)?;
            }
            PlayerAction::DeclareBlocker { blocker, attackers } => {
                self.game.declare_blocker(player_id, *blocker, attackers.clone())?;
            }
            PlayerAction::FinishDeclareAttackers | PlayerAction::FinishDeclareBlockers => {
                // Handled by the combat step logic, not here
            }
            PlayerAction::PassPriority => {
                // Nothing to do
            }
        }
        Ok(())
    }

    /// Describe an action for verbose output
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    pub(super) fn describe_action(&self, action: &PlayerAction) -> String {
        match action {
            PlayerAction::PlayLand(card_id) => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("plays {card_name}")
            }
            PlayerAction::TapForMana(card_id) => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("taps {card_name} for mana")
            }
            PlayerAction::CastSpell { card_id, .. } => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("casts {card_name}")
            }
            PlayerAction::DeclareAttacker(card_id) => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("declares {card_name} as attacker")
            }
            PlayerAction::DeclareBlocker { blocker, attackers } => {
                let blocker_name = self
                    .game
                    .cards
                    .get(*blocker)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let attacker_names: Vec<_> = attackers
                    .iter()
                    .filter_map(|id| self.game.cards.get(*id).ok().map(|c| c.name.as_str()))
                    .collect();
                format!("blocks with {blocker_name} (blocking {attacker_names:?})")
            }
            PlayerAction::FinishDeclareAttackers => "finishes declaring attackers".to_string(),
            PlayerAction::FinishDeclareBlockers => "finishes declaring blockers".to_string(),
            PlayerAction::PassPriority => "passes priority".to_string(),
        }
    }
}
