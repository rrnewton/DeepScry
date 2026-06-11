//! Stat-modification effect-family handlers (pump / debuff / mass-animate)
//! extracted from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that adjust a creature's power/toughness or keyword set
//! until end of turn (CR 613, layers 7c / 6):
//! - [`Effect::PumpCreature`] / [`Effect::PumpCreatureVariable`] — single-target
//!   +X/+Y (fixed or count-derived) plus until-EOT keyword grants,
//! - [`Effect::DebuffCreature`] — remove keywords,
//! - [`Effect::PumpAllCreatures`] — mass +X/+Y over a filter,
//! - [`Effect::AnimateAll`] — mass set-base-P/T + keyword grant.
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim,
//! including the existing undo-log bookkeeping that lets a rewind+replay
//! reproduce these until-EOT modifications bit-identically (mtg-610 / mtg-731).
//!
//! KNOWN SMELL (tracked by mtg-907): `execute_animate_all` filters candidates
//! with raw `filter.contains("YouCtrl")` / `filter.contains("Creature")`
//! substring checks instead of the structured `TargetRestriction` matcher that
//! `execute_pump_all_creatures` already uses. This is a "No Hacky String
//! Operations On Structured Data" violation and a divergent filter; it is
//! preserved verbatim here (behavior-preserving extraction) and slated for the
//! Valid$/target-filter consolidation in mtg-907.

use crate::core::{CardId, CountExpression, Keyword, PlayerId};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::PumpCreature`]: give the target a fixed +`power_bonus`/
    /// +`toughness_bonus` and grant `keywords_granted` until end of turn. Only
    /// the keywords this pump *newly* adds are recorded for undo, so a rewind
    /// never strips a printed/other-source keyword (mtg-731).
    pub(in crate::game::actions) fn execute_pump_creature(
        &mut self,
        target: CardId,
        power_bonus: i32,
        toughness_bonus: i32,
        keywords_granted: &[Keyword],
    ) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            log::warn!(target: "pump", "PumpCreature fizzled: unresolved target {}", target.as_u32());
            return Ok(());
        }
        log::debug!(target: "pump", "PumpCreature executing: target={}, power_bonus={}, toughness_bonus={}, keywords={:?}", target.as_u32(), power_bonus, toughness_bonus, keywords_granted);
        // Capture log size before pump
        let prior_log_size = self.logger.log_count();

        let card = self.cards.get_mut(target)?;
        card.power_bonus += power_bonus;
        card.toughness_bonus += toughness_bonus;
        // Grant keywords until end of turn (tracked so forward cleanup
        // + rewind sweep can remove them deterministically; mtg-610).
        // Record ONLY the keywords this pump *newly* added so the undo
        // entry removes exactly those, never a printed/other-source
        // keyword (mtg-731: Rockface Village pumping a printed-haste
        // creature was stripping its Haste).
        let mut newly_granted: smallvec::SmallVec<[Keyword; 2]> = smallvec::SmallVec::new();
        for keyword in keywords_granted.iter() {
            if card.grant_keyword_until_eot(*keyword) {
                newly_granted.push(*keyword);
            }
        }

        // Log the pump effect
        self.undo_log.log(
            crate::undo::GameAction::PumpCreature {
                card_id: target,
                power_delta: power_bonus,
                toughness_delta: toughness_bonus,
                keywords_granted: newly_granted,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::PumpCreatureVariable`]: pump where the +X/+Y bonus is derived
    /// from counting game state (e.g. Berserk's `Targeted$CardPower`
    /// power-doubling, or "+X/+X where X = artifacts opponents control"). The
    /// target's own power is read BEFORE the pump mutates it, so a
    /// `TargetedCardPower` X locks to the pre-pump value (CR 613.4).
    pub(in crate::game::actions) fn execute_pump_creature_variable(
        &mut self,
        target: CardId,
        power_count: &CountExpression,
        toughness_count: &CountExpression,
        keywords_granted: &[Keyword],
    ) -> Result<()> {
        // Variable pump: bonus depends on counting game state
        // Example: Elephant-Mandrill gets +X/+X where X is artifacts opponents control

        // Skip if target is still placeholder
        if target.is_placeholder() {
            log::warn!(target: "pump", "PumpCreatureVariable fizzled: target is still placeholder");
            return Ok(());
        }

        // Get target's controller for filter resolution
        let target_controller = self.cards.get(target)?.controller;

        // Evaluate the count expressions. `Targeted$CardPower` (Berserk's
        // power-doubling +X/+0) resolves against the target itself and is
        // not visible to `evaluate_count_expression` (controller-only), so
        // resolve it HERE from the target's CURRENT power — read BEFORE the
        // pump mutates power_bonus below, so X locks to the pre-pump value
        // (CR 613.4: the +X/+0 layer applies once, X = power at resolution).
        let target_power = i32::from(self.cards.get(target)?.current_power());
        let resolve = |this: &Self, count: &CountExpression| -> Result<i32> {
            if matches!(count, CountExpression::TargetedCardPower) {
                Ok(target_power)
            } else {
                this.evaluate_count_expression(count, target_controller)
            }
        };
        let power_bonus = resolve(self, power_count)?;
        let toughness_bonus = resolve(self, toughness_count)?;

        log::debug!(
            target: "pump",
            "PumpCreatureVariable executing: target={}, power_bonus={}, toughness_bonus={}, keywords={:?}",
            target.as_u32(),
            power_bonus,
            toughness_bonus,
            keywords_granted
        );

        // Apply the pump
        let prior_log_size = self.logger.log_count();
        let card = self.cards.get_mut(target)?;
        card.power_bonus += power_bonus;
        card.toughness_bonus += toughness_bonus;
        // Record ONLY newly-added keywords so undo never strips a
        // printed/other-source keyword (mtg-731).
        let mut newly_granted: smallvec::SmallVec<[Keyword; 2]> = smallvec::SmallVec::new();
        for keyword in keywords_granted.iter() {
            if card.grant_keyword_until_eot(*keyword) {
                newly_granted.push(*keyword);
            }
        }

        // Log for undo
        self.undo_log.log(
            crate::undo::GameAction::PumpCreature {
                card_id: target,
                power_delta: power_bonus,
                toughness_delta: toughness_bonus,
                keywords_granted: newly_granted,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::DebuffCreature`]: remove `keywords_removed` from the target
    /// (e.g. losing flying), logged for undo.
    pub(in crate::game::actions) fn execute_debuff_creature(
        &mut self,
        target: CardId,
        keywords_removed: &smallvec::SmallVec<[Keyword; 2]>,
    ) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            log::warn!(target: "debuff", "DebuffCreature fizzled: unresolved target {}", target.as_u32());
            return Ok(());
        }
        log::debug!(target: "debuff", "DebuffCreature executing: target={}, keywords_removed={:?}", target.as_u32(), keywords_removed);

        let prior_log_size = self.logger.log_count();
        let card = self.cards.get_mut(target)?;
        // Remove keywords
        for keyword in keywords_removed.iter() {
            card.keywords.remove(*keyword);
        }

        // Log the debuff effect for undo
        self.undo_log.log(
            crate::undo::GameAction::DebuffCreature {
                card_id: target,
                keywords_removed: keywords_removed.clone(),
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::PumpAllCreatures`]: "creatures you control get +X/+Y until end
    /// of turn" — pump every creature matching the structured `filter`.
    pub(in crate::game::actions) fn execute_pump_all_creatures(
        &mut self,
        controller: PlayerId,
        filter: &str,
        power_bonus: i32,
        toughness_bonus: i32,
    ) -> Result<()> {
        // Mass pump: "Creatures you control get +X/+Y until end of turn"
        // Find all creatures matching the filter and pump them
        let restriction = crate::core::TargetRestriction::parse(filter);
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Some(card) = self.cards.try_get(card_id) {
                    if card.is_creature() && restriction.matches_with_controller(card, controller, card.controller) {
                        Some(card_id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // Apply pump to all matching creatures
        for target in targets {
            let prior_log_size = self.logger.log_count();
            if let Ok(card) = self.cards.get_mut(target) {
                card.power_bonus += power_bonus;
                card.toughness_bonus += toughness_bonus;
                log::debug!(
                    "PumpAllCreatures: {} gets +{}/+{}",
                    card.name,
                    power_bonus,
                    toughness_bonus
                );
                self.logger.normal(&format!(
                    "{} gets +{}/+{} until end of turn",
                    card.name, power_bonus, toughness_bonus
                ));
            }
            self.undo_log.log(
                crate::undo::GameAction::PumpCreature {
                    card_id: target,
                    power_delta: power_bonus,
                    toughness_delta: toughness_bonus,
                    keywords_granted: smallvec::SmallVec::new(),
                },
                prior_log_size,
            );
        }
        Ok(())
    }

    /// [`Effect::AnimateAll`]: set base P/T and/or grant keywords to all matching
    /// permanents (like `PumpAllCreatures` but sets base P/T instead of bonuses).
    ///
    /// NOTE (mtg-907): the candidate filter here uses raw `filter.contains(...)`
    /// substring checks rather than the structured `TargetRestriction` matcher.
    /// Preserved verbatim from the original inline arm; slated for consolidation.
    pub(in crate::game::actions) fn execute_animate_all(
        &mut self,
        controller: PlayerId,
        filter: &str,
        power: Option<i32>,
        toughness: Option<i32>,
        keywords_granted: &[Keyword],
    ) -> Result<()> {
        // AnimateAll: set base P/T and/or grant keywords to all matching permanents
        // Similar to PumpAllCreatures but sets base P/T instead of bonuses
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.try_get(card_id)?;
                // Check controller filters
                if filter.contains("YouCtrl") && card.controller != controller {
                    return None;
                }
                if filter.contains("OppCtrl") && card.controller == controller {
                    return None;
                }
                // Check type filters
                if filter.contains("Creature") && !card.is_creature() {
                    return None;
                }
                if filter.contains("Planeswalker") && !card.is_planeswalker() {
                    return None;
                }
                if filter.contains("Land") && !card.is_land() {
                    return None;
                }
                Some(card_id)
            })
            .collect();

        for target in targets {
            // Set base P/T (if specified) via the logged helper so the
            // override is reversible by the undo log (mtg-614 hole (c)).
            if power.is_some() || toughness.is_some() {
                self.set_temp_base_stats_logged(target, power.map(|p| p as i8), toughness.map(|t| t as i8));
            }
            if let Ok(card) = self.cards.get_mut(target) {
                let card_name = card.name.clone();

                // Grant keywords until end of turn (AnimateAll is an
                // until-EOT mass animate; track them so forward cleanup
                // + rewind sweep remove them deterministically; mtg-610).
                for kw in keywords_granted {
                    card.grant_keyword_until_eot(*kw);
                }

                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let kws: Vec<_> = keywords_granted.iter().map(|k| format!("{:?}", k)).collect();
                    let kw_str = if kws.is_empty() {
                        String::new()
                    } else {
                        format!(" and gains {}", kws.join(", "))
                    };

                    if power.is_some() || toughness.is_some() {
                        self.logger.gamelog(&format!(
                            "{} becomes {}/{}{}",
                            card_name,
                            card.current_power(),
                            card.current_toughness(),
                            kw_str
                        ));
                    } else if !kws.is_empty() {
                        self.logger.gamelog(&format!("{} gains {}", card_name, kws.join(", ")));
                    }
                }
            }
        }
        Ok(())
    }
}
