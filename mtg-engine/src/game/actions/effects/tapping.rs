//! Tap / untap effect-family handlers extracted from the `execute_effect`
//! dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that tap or untap permanents (CR 701.21 / 701.22):
//! - [`Effect::TapPermanent`] / [`Effect::UntapPermanent`] / single-target,
//! - [`Effect::TapOrUntapPermanent`] (AI chooses),
//! - [`Effect::TapAll`] / [`Effect::UntapAll`] (mass, filtered).
//!
//! All handlers route tap/untap through the `tap_permanent` / `untap_permanent`
//! helpers (NOT direct `card.tapped` writes) so the undo log, the
//! `ManaSourceCache` untapped counts, and `mana_state_version` stay consistent
//! — a direct write would desync server vs client shadow state (see
//! `docs/NETWORK_ARCHITECTURE.md`). Behavior-preserving: bodies moved verbatim.

use crate::core::{CardId, TriggerEvent};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::TapPermanent`]: tap the target and fire `Taps` triggers
    /// (CR 701.21). Fizzles on an unresolved/placeholder target.
    pub(in crate::game::actions) fn execute_tap_permanent(&mut self, target: CardId) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            // Spell fizzles - no valid targets
            return Ok(());
        }
        // Use helper that handles tap + undo log + mana version
        self.tap_permanent(target)?;
        // Check for Taps triggers
        self.check_triggers(TriggerEvent::Taps, target)?;
        Ok(())
    }

    /// [`Effect::UntapPermanent`]: untap the target (CR 701.22). Fizzles on an
    /// unresolved/placeholder target.
    pub(in crate::game::actions) fn execute_untap_permanent(&mut self, target: CardId) -> Result<()> {
        // Skip if target is placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            return Ok(());
        }
        // Use helper that handles untap + undo log + mana version
        self.untap_permanent(target)?;
        Ok(())
    }

    /// [`Effect::TapOrUntapPermanent`]: tap OR untap the target, with the AI
    /// choosing — heuristic: untap our own permanents (free mana / ready to
    /// block), tap the opponent's (remove blocker / deny mana). Taps fire
    /// `Taps` triggers.
    pub(in crate::game::actions) fn execute_tap_or_untap_permanent(&mut self, target: CardId) -> Result<()> {
        // Tap or untap target permanent (AI chooses)
        // Heuristic: untap our own creatures, tap opponent's
        if target.is_placeholder() {
            return Ok(());
        }
        if let Some(card) = self.cards.try_get(target) {
            let is_ours = card.controller == self.turn.active_player;
            if is_ours {
                // Untap our own permanent (free mana, ready to block)
                self.untap_permanent(target)?;
            } else {
                // Tap opponent's permanent (remove blocker, deny mana)
                self.tap_permanent(target)?;
                self.check_triggers(TriggerEvent::Taps, target)?;
            }
        }
        Ok(())
    }

    /// [`Effect::TapAll`]: tap every untapped permanent matching `restriction`.
    pub(in crate::game::actions) fn execute_tap_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
    ) -> Result<()> {
        // Tap all permanents matching the restriction
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .get(card_id)
                    .map(|card| !card.tapped && restriction.matches(card))
                    .unwrap_or(false)
            })
            .collect();

        for card_id in targets {
            // Route through tap_permanent so the undo log, ManaSourceCache
            // untapped counts, and mana_state_version all stay consistent.
            // Setting `card.tapped` directly would leave the mana cache
            // reporting these sources as still untapped, which can offer an
            // unaffordable spell as a legal play and diverge server vs client
            // shadow state (network desync). See docs/NETWORK_ARCHITECTURE.md.
            let card_name = self.cards.get(card_id)?.name.clone();
            self.tap_permanent(card_id)?;
            self.logger.gamelog(&format!("{} ({}) is tapped", card_name, card_id));
        }
        Ok(())
    }

    /// [`Effect::UntapAll`]: untap every tapped permanent matching `restriction`
    /// (controller-aware).
    pub(in crate::game::actions) fn execute_untap_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
    ) -> Result<()> {
        // Untap all permanents matching the restriction
        let spell_controller = self.turn.active_player;
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .get(card_id)
                    .map(|card| {
                        card.tapped && restriction.matches_with_controller(card, spell_controller, card.controller)
                    })
                    .unwrap_or(false)
            })
            .collect();

        for card_id in targets {
            // Route through untap_permanent so the undo log, ManaSourceCache
            // untapped counts, and mana_state_version stay consistent (see
            // the matching note in Effect::TapAll above).
            let card_name = self.cards.get(card_id)?.name.clone();
            self.untap_permanent(card_id)?;
            self.logger.gamelog(&format!("{} ({}) is untapped", card_name, card_id));
        }
        Ok(())
    }
}
