//! Stack / counter-magic / conditional-dispatch effect-family handlers
//! extracted from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that interact with the stack or conditionally dispatch
//! sub-effects:
//! - [`Effect::CounterSpell`] — counter a spell on the stack (CR 701.5),
//!   optionally remembering its mana value for a chained rider (Mana Drain),
//! - [`Effect::ConditionalSelfCounter`] — run an inner effect only if the
//!   source's counter state satisfies an intervening-if gate (CR 603.4),
//! - [`Effect::ModalChoice`] — routing-guard fallback (modes are picked at
//!   cast time; this should never reach execution),
//! - [`Effect::ImmediateTrigger`] — run sub-effects if a remembered-cards
//!   condition is met.
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::{CardId, Effect, ImmediateTriggerCondition, SelfCounterCondition};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::CounterSpell`]: counter the target spell on the stack
    /// (CR 701.5). Fizzles if the target is a placeholder or no longer on the
    /// stack (e.g. a triggered counter firing with nothing to counter). When
    /// `remember_mana_value` is set (Mana Drain's `RememberCounteredCMC$`), the
    /// countered spell's mana value (incl. any X paid) is captured BEFORE it
    /// leaves the stack so a chained delayed trigger can add that much {C}.
    pub(in crate::game::actions) fn execute_counter_spell(
        &mut self,
        target: CardId,
        remember_mana_value: bool,
    ) -> Result<()> {
        // Counter a spell on the stack
        // Fizzle if target is placeholder (no valid target found) or not on stack
        // This happens when triggered counter effects (e.g., Ulamog's Nullifier ETB)
        // fire when no spell is on the stack to target
        if target.is_placeholder() || !self.stack.contains(target) {
            log::debug!("CounterSpell fizzles - target {} not on stack", target.as_u32());
        } else {
            // Mana Drain (RememberCounteredCMC$ True): record the
            // countered spell's mana value (including any X paid) BEFORE
            // it leaves the stack, so the chained delayed trigger can
            // add that much {C} at the controller's next main phase.
            if remember_mana_value {
                let mana_value = self
                    .cards
                    .try_get(target)
                    .map(|c| u32::from(c.mana_cost.cmc()) + u32::from(c.x_paid))
                    .unwrap_or(0);
                let prior_log_size = self.logger.log_count();
                let previous = self.remembered_amount;
                self.remembered_amount = Some(mana_value);
                self.undo_log.log(
                    crate::undo::GameAction::SetRememberedAmount { previous },
                    prior_log_size,
                );
            }
            self.counter_spell(target)?;
        }
        Ok(())
    }

    /// [`Effect::ConditionalSelfCounter`]: run `inner` only if `source`
    /// currently satisfies the counter condition (CR 603.4 intervening-if gate
    /// for a mid-chain sub-ability). E.g. All Hallow's Eve's exile→graveyard +
    /// mass resurrection only fires on the upkeep where the final scream counter
    /// was removed.
    pub(in crate::game::actions) fn execute_conditional_self_counter(
        &mut self,
        source: CardId,
        condition: &SelfCounterCondition,
        inner: &Effect,
    ) -> Result<()> {
        if source.is_placeholder() || source.is_self_target() {
            log::debug!(
                target: "self_exile",
                "ConditionalSelfCounter: source still placeholder/sentinel, skipping"
            );
            return Ok(());
        }
        let satisfied = self
            .cards
            .try_get(source)
            .map(|c| condition.evaluate(c.get_counter(condition.counter_type)))
            .unwrap_or(false);
        if satisfied {
            self.execute_effect(inner)?;
        }
        Ok(())
    }

    /// [`Effect::ModalChoice`] routing-guard fallback. Modal spells pick their
    /// mode at cast time and resolve only the selected mode's effect, so this
    /// variant should never reach execute_effect — warn if it does.
    pub(in crate::game::actions) fn execute_modal_choice_fallback(&self, num_modes: usize) -> Result<()> {
        // Modal spells are handled during casting, not execution.
        // When the spell resolves, only the selected mode's effect is executed.
        // This variant should not be encountered during execute_effect.
        //
        // If we get here, it means the modal choice wasn't processed during casting.
        // Log a warning and skip execution.
        log::warn!(
            target: "actions",
            "ModalChoice effect reached execute_effect - should have been resolved during casting. {} modes available.",
            num_modes
        );
        Ok(())
    }

    /// [`Effect::ImmediateTrigger`]: run `sub_effects` if the remembered-cards
    /// `condition` is met (e.g. "if a nonland card was revealed/remembered").
    pub(in crate::game::actions) fn execute_immediate_trigger(
        &mut self,
        condition: &ImmediateTriggerCondition,
        sub_effects: &[Effect],
    ) -> Result<()> {
        // Check if remembered cards match the condition
        let condition_met = match condition {
            ImmediateTriggerCondition::RememberedNonLand => {
                // Check if any remembered card is a nonland
                self.remembered_cards.iter().any(|&card_id| {
                    if let Some(card) = self.cards.try_get(card_id) {
                        !card.is_land()
                    } else {
                        false
                    }
                })
            }
            ImmediateTriggerCondition::AnyRemembered => !self.remembered_cards.is_empty(),
        };

        if condition_met {
            // Execute sub-effects
            for sub_effect in sub_effects {
                self.execute_effect(sub_effect)?;
            }
        }
        Ok(())
    }
}
