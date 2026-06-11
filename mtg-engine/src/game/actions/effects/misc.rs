//! Miscellaneous resource / turn-structure / control-flow effect handlers
//! extracted from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the small, self-contained effects that did not warrant a family of
//! their own:
//! - **mana:** [`Effect::AddMana`] (Dark Ritual / Su-Chi-style mana into the
//!   pool), [`Effect::ChooseColor`],
//! - **turn structure:** [`Effect::AddTurn`] (CR 500.7 extra turns),
//!   [`Effect::AddPhase`] (extra combat phases),
//! - **scratch / control-flow:** [`Effect::ClearRemembered`],
//!   [`Effect::Clone`] (routing-guard fallback), [`Effect::Unimplemented`],
//!   [`Effect::NoOp`].
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::{CardId, Color, ManaCost, PlayerId};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::AddMana`]: add the components of `mana` to the player's pool.
    /// Spell/triggered-ability path (Dark Ritual, Su-Chi); mana ABILITIES go
    /// through `tap_for_mana_for_cost` instead, where the source's chosen color
    /// and variable amount are available — so `produces_chosen_color` /
    /// `amount_var` reaching here is unexpected and warns.
    pub(in crate::game::actions) fn execute_add_mana(
        &mut self,
        player: PlayerId,
        mana: &ManaCost,
        produces_chosen_color: bool,
        amount_var: Option<&str>,
    ) -> Result<()> {
        // Capture log size before mana addition
        let prior_log_size = self.logger.log_count();

        // Add mana to player's mana pool
        // Note: For mana abilities, produces_chosen_color is handled in tap_for_mana_for_cost
        // where we have access to the source card's chosen_color.
        // This path is mainly for spell effects (Dark Ritual) and triggered abilities (Su-Chi).
        // Note: amount_var (for variable mana like Raucous Audience) is resolved in ManaEngine
        // during tap_for_mana_for_cost, not here.
        if produces_chosen_color {
            // This shouldn't happen in practice since mana abilities go through tap_for_mana_for_cost
            // but log a warning if it does
            self.logger
                .normal("Warning: produces_chosen_color in execute_effect - source card unknown");
        }
        if amount_var.is_some() {
            // Variable mana should be resolved before reaching execute_effect
            self.logger
                .normal("Warning: amount_var in execute_effect - should be resolved in ManaEngine");
        }
        let p = self.get_player_mut(player)?;

        // Add each component of the mana cost to the pool
        for _ in 0..mana.white {
            p.mana_pool.add_color(Color::White);
        }
        for _ in 0..mana.blue {
            p.mana_pool.add_color(Color::Blue);
        }
        for _ in 0..mana.black {
            p.mana_pool.add_color(Color::Black);
        }
        for _ in 0..mana.red {
            p.mana_pool.add_color(Color::Red);
        }
        for _ in 0..mana.green {
            p.mana_pool.add_color(Color::Green);
        }
        for _ in 0..mana.colorless {
            p.mana_pool.add_color(Color::Colorless);
        }

        // Log the mana addition
        self.undo_log.log(
            crate::undo::GameAction::AddMana {
                player_id: player,
                mana: *mana,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::ChooseColor`]: pick a color (AI heuristic: the most prominent
    /// color in the player's deck) and store it on the `source` card.
    pub(in crate::game::actions) fn execute_choose_color(&mut self, player: PlayerId, source: CardId) -> Result<()> {
        // Choose a color using AI heuristic (pick most prominent color in deck)
        let chosen = self.pick_prominent_color(player, &[]);

        // Store the chosen color on the source card
        if let Ok(card) = self.cards.get_mut(source) {
            let card_name = card.name.clone();
            card.chosen_color = Some(chosen);
            let player_name = self
                .get_player(player)
                .map(|p| p.name.to_string())
                .unwrap_or_else(|_| format!("Player {}", player.as_u32()));
            self.logger
                .normal(&format!("{} chooses color: {:?} ({})", player_name, chosen, card_name));
        } else {
            log::warn!("ChooseColor: source card {} not found", source.as_u32());
        }
        Ok(())
    }

    /// [`Effect::AddTurn`]: queue `num_turns` extra turns for the player
    /// (CR 500.7 — Time Walk etc.). Pushes onto `extra_turns` (the queue the
    /// turn-rotation code actually drains) and logs each for undo (mtg-551 /
    /// mtg-559 / mtg-610).
    pub(in crate::game::actions) fn execute_add_turn(&mut self, player: PlayerId, num_turns: u8) -> Result<()> {
        // Take extra turns (CR 500.7) - Time Walk, Temporal Manipulation, etc.
        // Add extra turns to the GameState extra-turn queue (consumed in
        // GameState::advance_step at end of turn, CR 500.7). NOTE: this
        // must push to `self.extra_turns` (the VecDeque actually drained
        // by the turn-rotation code), NOT `self.turn.extra_turns` (a
        // dead, write-only field) — otherwise the extra turn was queued
        // somewhere nothing reads, and never taken (mtg-551).
        for _ in 0..num_turns {
            let prior_log_size = self.logger.log_count();
            self.extra_turns.push_back(player);
            // Log for undo so a rewind+replay across the AddTurn
            // resolution doesn't leave a stale queued extra turn
            // (mtg-559/mtg-610).
            self.undo_log
                .log(crate::undo::GameAction::PushExtraTurn { player }, prior_log_size);
        }
        let player_name = self
            .get_player(player)
            .map(|p| p.name.as_str().to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        self.logger.gamelog(&format!(
            "{} takes {} extra turn(s) after this one",
            player_name, num_turns
        ));
        Ok(())
    }

    /// [`Effect::AddPhase`]: add `count` extra combat phase(s) after the current
    /// step (Relentless Assault-style).
    pub(in crate::game::actions) fn execute_add_phase(&mut self, count: u8) -> Result<()> {
        // Add extra combat phase(s) after the current step
        for _ in 0..count {
            self.extra_combat_phases += 1;
        }
        self.logger
            .gamelog(&format!("AddPhase: {} additional combat phase(s) this turn", count));
        Ok(())
    }

    /// [`Effect::ClearRemembered`]: clear the remembered-card / remembered-player
    /// / remembered-amount scratch. Any numeric value (Mana Drain) was already
    /// captured onto its delayed trigger by the preceding CreateDelayedTrigger,
    /// so clearing here is safe. The amount clear is logged for undo.
    pub(in crate::game::actions) fn execute_clear_remembered(&mut self) -> Result<()> {
        self.remembered_cards.clear();
        self.remembered_players.clear();
        if self.remembered_amount.is_some() {
            let prior_log_size = self.logger.log_count();
            let previous = self.remembered_amount;
            self.remembered_amount = None;
            self.undo_log.log(
                crate::undo::GameAction::SetRememberedAmount { previous },
                prior_log_size,
            );
        }
        Ok(())
    }

    /// [`Effect::Clone`] routing-guard fallback. Clone requires a controller
    /// decision ("you may" + which permanent to copy) and is resolved by the
    /// interactive path in `priority.rs::resolve_clone_effect`. Reaching
    /// execute_effect means that interception was bypassed — warn so the routing
    /// gap is visible rather than silently entering as a vanilla permanent.
    pub(in crate::game::actions) fn execute_clone_fallback(&mut self) -> Result<()> {
        log::warn!(
            target: "actions",
            "Effect::Clone reached execute_effect without controller interception; \
             permanent will not copy. This is a routing bug — Clone must go through \
             the interactive spell-resolution hook."
        );
        Ok(())
    }

    /// [`Effect::Unimplemented`]: an effect API not yet modeled — log a warning
    /// and a gamelog line, then resolve as a no-op (so the gap is visible).
    pub(in crate::game::actions) fn execute_unimplemented(&mut self, api_type: &str) -> Result<()> {
        // Log a warning instead of silently doing nothing
        log::warn!(target: "actions", "Unimplemented effect '{}' resolved as no-op", api_type);
        self.logger.gamelog(&format!(
            "WARNING: Effect '{}' is not yet implemented - resolving as no-op",
            api_type
        ));
        Ok(())
    }

    /// [`Effect::NoOp`]: an *intentional* no-op (e.g. StoreSVar, whose value is
    /// modeled directly elsewhere). Silent — no warning, no gamelog.
    pub(in crate::game::actions) fn execute_noop(&self, api_type: &str) -> Result<()> {
        log::debug!(target: "actions", "NoOp effect '{}' (intentional)", api_type);
        Ok(())
    }
}
