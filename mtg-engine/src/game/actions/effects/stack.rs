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
//!   condition is met,
//! - [`Effect::CreateDelayedTrigger`] — register a delayed trigger (Fatal
//!   Fissure / Mana Drain / Berserk's end-step destroy / Jeong Jeong),
//! - [`Effect::UnlessCostWrapper`] — "[effect] unless [payer] pays [cost]"
//!   (Chain Lightning, counter-unless-pay, may-discard-to-draw).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::effects::UnlessCost;
use crate::core::{
    CardId, DelayedTriggerCondition, DelayedTriggerExpiry, Effect, ImmediateTriggerCondition, PlayerId,
    SelfCounterCondition,
};
use crate::game::GameState;
use crate::zones::Zone;
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

            // Summoning Trap: if this spell is a creature spell cast by
            // its owner (not from an opponent's effect), and it is being
            // countered by an opponent's effect, set the
            // `had_creature_countered_this_turn` flag on the owner.
            // Condition: current_spell_controller is set (= the player
            // who controls the counter spell) and that player is NOT the
            // owner of the countered spell.
            if let Some(counter_controller) = self.current_spell_controller {
                if let Some(countered_card) = self.cards.try_get(target) {
                    let countered_owner = countered_card.owner;
                    let is_creature_spell = countered_card.is_creature();
                    if is_creature_spell && counter_controller != countered_owner {
                        if let Some(player) = self.players.iter_mut().find(|p| p.id == countered_owner) {
                            player.had_creature_countered_this_turn = true;
                        }
                    }
                }
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

    /// [`Effect::CreateDelayedTrigger`]: register a delayed trigger (SP$/DB$
    /// DelayedTrigger). Two shapes: a `ZoneChange` trigger watching a specific
    /// battlefield card (Fatal Fissure — fizzles if the tracked card is gone),
    /// or a phase / spell-cast trigger with no tracked card (Mana Drain, Jeong
    /// Jeong). The trigger's controller is the resolving spell's controller. The
    /// registration is undo-logged so rewind-to-turn-start removes it (mtg-519).
    pub(in crate::game::actions) fn execute_create_delayed_trigger(
        &mut self,
        tracked_card: CardId,
        condition: &DelayedTriggerCondition,
        delayed_effect: &Effect,
        expiry: &Option<DelayedTriggerExpiry>,
    ) -> Result<()> {
        // CreateDelayedTrigger effect: Register a delayed trigger that fires on a condition
        // Created by SP$/DB$ DelayedTrigger spells.
        //
        // Two shapes:
        // - ZoneChange (Fatal Fissure): tracks a battlefield card; the
        //   trigger fires when that card changes zones. Requires a valid
        //   tracked card on the battlefield or the spell fizzles.
        // - Phase / SpellCast (Mana Drain, Jeong Jeong): no tracked
        //   battlefield card; the trigger fires at a future phase / on a
        //   future spell cast. The controller is the resolving spell's
        //   controller, regardless of whose turn it is.
        let is_zone_change = matches!(condition, DelayedTriggerCondition::ZoneChange { .. });

        if is_zone_change {
            // Skip if tracked_card is still placeholder (0) - no valid targets found
            if tracked_card.is_placeholder() {
                log::debug!(target: "actions", "CreateDelayedTrigger: tracked_card is placeholder, spell fizzles");
                return Ok(());
            }
            // Verify the target is still on battlefield
            if !self.battlefield.contains(tracked_card) {
                log::debug!(target: "actions", "CreateDelayedTrigger: target no longer on battlefield, spell fizzles");
                return Ok(());
            }
        }

        // Get card name for logging (tracked card for zone triggers, else
        // the resolving spell's name).
        let card_name = self
            .cards
            .try_get(tracked_card)
            .or_else(|| self.current_damage_source.and_then(|src| self.cards.try_get(src)))
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        // The trigger controller is the resolving spell's controller
        // (current_damage_source is set to the resolving spell during
        // resolve_spell_execute_effects). Fall back to the active player
        // for the legacy zone-change path / direct execute_effect calls.
        let controller = self
            .current_damage_source
            .and_then(|src| self.cards.try_get(src))
            .map(|c| c.controller)
            .unwrap_or(self.turn.active_player);

        // Create the delayed trigger
        use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerId};

        // Check if the inner effect is CopySpellAbility - needs special handling
        // Wildcard is appropriate: all non-CopySpellAbility effects wrap in ExecuteEffect
        #[allow(clippy::wildcard_enum_match_arm)]
        let delayed_effect_type = match *delayed_effect {
            Effect::CopySpellAbility { may_choose_targets, .. } => {
                // For CopySpellAbility, use the specialized DelayedEffect variant
                // tracked_card will be repurposed to hold the spell being copied
                // (set at trigger fire time, not creation time)
                DelayedEffect::CopySpellAbility { may_choose_targets }
            }
            Effect::DestroyPermanent { .. } => {
                // Berserk: "At the beginning of the next end step, destroy
                // that creature if it attacked this turn." The delayed
                // Destroy targets the TRACKED card (the creature Berserk
                // targeted via RememberObjects$ Targeted), gated on its
                // attacked-this-turn flag (CR 603.4). Route to the
                // dedicated DestroyTracked variant so fire_delayed_trigger
                // can both bind the tracked card and check the gate.
                DelayedEffect::DestroyTracked {
                    require_attacked_this_turn: true,
                }
            }
            _ => {
                // For all other effects, wrap in ExecuteEffect
                DelayedEffect::ExecuteEffect {
                    effect: Box::new(delayed_effect.clone()),
                }
            }
        };

        let trigger = DelayedTrigger::new(
            DelayedTriggerId::new(0), // ID will be assigned by store
            tracked_card, // tracked_card - for zone triggers: the creature to watch; for spell triggers: will be set at fire time
            tracked_card, // source_card - same as tracked for spell-created triggers
            controller,
            condition.clone(),
            delayed_effect_type,
        );

        // Apply expiry if specified
        let trigger = match expiry {
            Some(exp) => trigger.with_expiry(exp.clone()),
            None => trigger,
        };

        // Capture any remembered numeric value (Mana Drain's countered
        // mana value) onto the trigger so it survives the chained
        // ClearRemembered and is part of the trigger's serialized state.
        let trigger = trigger.with_remembered_amount(self.remembered_amount);

        let prior_log_size = self.logger.log_count();
        let trigger_id = self.delayed_triggers.add(trigger);
        // Undo-log the registration so rewind-to-turn-start (snapshot/
        // resume, undo search) removes it; otherwise the replay would
        // double-register the trigger (mtg-519).
        self.undo_log.log(
            crate::undo::GameAction::RegisterDelayedTrigger { id: trigger_id },
            prior_log_size,
        );

        // Log the delayed trigger creation
        let what = if is_zone_change {
            format!("watching {} for death", card_name)
        } else {
            format!("from {}", card_name)
        };
        self.logger
            .gamelog(&format!("Delayed trigger {} created: {}", trigger_id.as_u32(), what));

        log::debug!(
            target: "actions",
            "CreateDelayedTrigger: trigger {} for {} with effect {:?}",
            trigger_id.as_u32(), card_name, delayed_effect
        );
        Ok(())
    }

    /// [`Effect::UnlessCostWrapper`]: "[effect] unless [payer] pays [cost]"
    /// (Chain Lightning, counter-unless-pay, may-discard-to-draw). Checks whether
    /// the resolved `payer` CAN pay, applies the AI pay/don't-pay heuristic,
    /// actually pays (mana tap / discard / life), then runs `inner_effect` iff
    /// the payment outcome (gated by `switched`) says it should.
    pub(in crate::game::actions) fn execute_unless_cost_wrapper(
        &mut self,
        inner_effect: &Effect,
        unless_cost: &UnlessCost,
    ) -> Result<()> {
        use crate::core::effects::UnlessCostType;

        // Parse the resolved payer ID (stored as numeric string after resolve_effect_target)
        let payer_id = unless_cost
            .payer
            .parse::<u32>()
            .map(PlayerId::new)
            .unwrap_or_else(|_| PlayerId::new(0));

        // Check if the cost can be paid
        let can_pay = match &unless_cost.cost {
            UnlessCostType::Discard { count, card_type: _ } => {
                // Check if player has enough cards in hand
                self.player_zones
                    .iter()
                    .find(|(pid, _)| *pid == payer_id)
                    .map(|(_, zones)| zones.hand.cards.len() >= *count as usize)
                    .unwrap_or(false)
            }
            UnlessCostType::Sacrifice { count, valid_type } => {
                // Check if player controls enough permanents of the type
                self.can_pay_sacrifice_pattern(valid_type, *count, CardId::new(0), payer_id)
            }
            UnlessCostType::PayLife(amount) => {
                // Check if player has enough life
                self.get_player(payer_id)
                    .map(|p| p.life > i32::from(*amount))
                    .unwrap_or(false)
            }
            UnlessCostType::Mana(mana_cost) => {
                // Check the payer can actually produce this cost, honouring
                // COLORED requirements (Chain Lightning's `UnlessCost$ R R`
                // is {R}{R}, generic=0 — the old "generic <= untapped lands"
                // check ignored color and treated 0 generic as trivially
                // payable). Reuse the ManaEngine affordability resolver,
                // built read-only from the current state (it reads the
                // per-player mana cache, falling back to a battlefield scan
                // when the cache is unbuilt). Pool mana (floating rituals)
                // is included via can_pay_with_pool. This is a pure read —
                // no borrow conflict with `&mut self`.
                let pool = self.try_get_player(payer_id).map(|p| p.mana_pool).unwrap_or_default();
                let mut mana_engine = crate::game::mana_engine::ManaEngine::new();
                mana_engine.update(self, payer_id);
                mana_engine.can_pay_with_pool(mana_cost, &pool)
            }
            UnlessCostType::Reveal { count, card_type: _ } => {
                // Check if player has enough cards in hand to reveal
                self.player_zones
                    .iter()
                    .find(|(pid, _)| *pid == payer_id)
                    .map(|(_, zones)| zones.hand.cards.len() >= *count as usize)
                    .unwrap_or(false)
            }
        };

        // AI heuristic: decide whether to pay
        // - For switched costs (pay → effect): AI pays if the effect benefits them
        // - For non-switched costs (effect if NOT paid): AI pays to prevent opponent's effect
        let should_pay = if can_pay {
            // Simple heuristic based on who benefits
            if unless_cost.switched {
                // "You may discard to draw" - controller benefits from effect
                // AI always takes beneficial effects
                true
            } else {
                // "Counter unless you pay" - opponent pays to prevent our spell
                // AI always tries to prevent the effect
                true
            }
        } else {
            false
        };

        // Execute payment if decided to pay
        let paid = if should_pay {
            match &unless_cost.cost {
                UnlessCostType::Discard { count, card_type: _ } => {
                    // Discard cards from hand (simple: discard from back)
                    let mut discarded = 0u8;
                    for _ in 0..*count {
                        // Get a card from hand to discard
                        let card_to_discard = self
                            .player_zones
                            .iter()
                            .find(|(pid, _)| *pid == payer_id)
                            .and_then(|(_, zones)| zones.hand.cards.last().copied());

                        if let Some(card_id) = card_to_discard {
                            // Move card to graveyard
                            let _ = self.move_card(card_id, Zone::Hand, Zone::Graveyard, payer_id);
                            discarded += 1;
                        }
                    }
                    discarded == *count
                }
                UnlessCostType::PayLife(amount) => {
                    // Pay life
                    if let Some(player) = self.players.iter_mut().find(|p| p.id == payer_id) {
                        player.life -= i32::from(*amount);
                        true
                    } else {
                        false
                    }
                }
                UnlessCostType::Sacrifice {
                    count: _,
                    valid_type: _,
                } => {
                    // TODO(mtg-884): Implement sacrifice payment for UnlessCost
                    // For now, return false (can't pay)
                    log::debug!("UnlessCost: Sacrifice payment not yet implemented");
                    false
                }
                UnlessCostType::Mana(mana_cost) => {
                    // Actually tap mana sources and deduct the cost. This
                    // replaces the old auto-success (which neither tapped
                    // nor deducted) — required for correctness AND for a
                    // recursive copy chain (Chain Lightning, mtg-152) to
                    // TERMINATE: each copy costs {R}{R} again, so the
                    // chain stops once a player runs out of red sources.
                    self.pay_mana_cost_by_tapping(payer_id, mana_cost)
                }
                UnlessCostType::Reveal { count: _, card_type: _ } => {
                    // Reveal doesn't consume cards, just show them
                    // For now, assume success
                    true
                }
            }
        } else {
            false
        };

        log::debug!(
            "UnlessCost: payer={}, can_pay={}, should_pay={}, paid={}, switched={}",
            payer_id.as_u32(),
            can_pay,
            should_pay,
            paid,
            unless_cost.switched
        );

        // Execute inner effect based on payment result and switched flag
        // - switched=true: execute if paid (e.g., "you may discard, if you do, draw")
        // - switched=false: execute if NOT paid (e.g., "counter unless you pay")
        let should_execute = if unless_cost.switched {
            paid // Execute effect only if cost was paid
        } else {
            !paid // Execute effect only if cost was NOT paid
        };

        if should_execute {
            self.execute_effect(inner_effect)?;
        } else {
            log::debug!(
                "UnlessCost: inner effect skipped (paid={}, switched={})",
                paid,
                unless_cost.switched
            );
        }
        Ok(())
    }
}
