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
//! Both [`GameState::execute_pump_all_creatures`] and
//! [`GameState::execute_animate_all`] filter their `ValidCards$` candidates
//! through the canonical [`crate::core::TargetRestriction`] (mtg-907 — the raw
//! `filter.contains(...)` substring matching that `execute_animate_all` used was
//! removed, eliminating the "No Hacky String Operations On Structured Data"
//! violation and the divergence between the two mass-effects).

use crate::core::{CardId, CountExpression, Keyword, KeywordArgs, PlayerId};
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
        keyword_args_granted: &[KeywordArgs],
    ) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            log::warn!(target: "pump", "PumpCreature fizzled: unresolved target {}", target.as_u32());
            return Ok(());
        }
        log::debug!(target: "pump", "PumpCreature executing: target={}, power_bonus={}, toughness_bonus={}, keywords={:?}, keyword_args={:?}", target.as_u32(), power_bonus, toughness_bonus, keywords_granted, keyword_args_granted);
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
        // Grant complex (parameterized) keywords (e.g. Landwalk:Forest).
        // Vec used (not SmallVec) to match the undo log field type, which uses
        // Vec to keep GameAction::PumpCreature from inflating the enum's size.
        let mut newly_args_granted: Vec<KeywordArgs> = Vec::new();
        for kw_args in keyword_args_granted.iter() {
            if card.grant_keyword_args_until_eot(kw_args) {
                newly_args_granted.push(kw_args.clone());
            }
        }

        // Emit gamelog for pump effect (B23: keyword grants were silent).
        let card_name = self.cards.get(target)?.name.clone();
        if power_bonus != 0 || toughness_bonus != 0 {
            self.logger.gamelog(&format!(
                "{} gets +{}/+{} until end of turn",
                card_name, power_bonus, toughness_bonus
            ));
        }
        if !newly_granted.is_empty() {
            let kws: Vec<_> = newly_granted.iter().map(|k| format!("{:?}", k)).collect();
            self.logger.gamelog(&format!("{} gains {}", card_name, kws.join(", ")));
        }

        // Log the pump effect
        self.undo_log.log(
            crate::undo::GameAction::PumpCreature {
                card_id: target,
                power_delta: power_bonus,
                toughness_delta: toughness_bonus,
                keywords_granted: newly_granted,
                keyword_args_granted: newly_args_granted,
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
        keyword_args_granted: &[KeywordArgs],
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
        let mut newly_args_granted: Vec<KeywordArgs> = Vec::new();
        for kw_args in keyword_args_granted.iter() {
            if card.grant_keyword_args_until_eot(kw_args) {
                newly_args_granted.push(kw_args.clone());
            }
        }

        // Log for undo
        self.undo_log.log(
            crate::undo::GameAction::PumpCreature {
                card_id: target,
                power_delta: power_bonus,
                toughness_delta: toughness_bonus,
                keywords_granted: newly_granted,
                keyword_args_granted: newly_args_granted,
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
                    keyword_args_granted: Vec::new(),
                },
                prior_log_size,
            );
        }
        Ok(())
    }

    /// [`Effect::AnimateAll`]: set base P/T and/or grant keywords to all matching
    /// permanents (like `PumpAllCreatures` but sets base P/T instead of bonuses).
    ///
    /// mtg-907: candidates are filtered through the canonical
    /// [`crate::core::TargetRestriction`] (parsed from the `ValidCards$` string),
    /// matching the sibling `execute_pump_all_creatures` — no raw substring
    /// matching.
    pub(in crate::game::actions) fn execute_animate_all(
        &mut self,
        controller: PlayerId,
        filter: &str,
        power: Option<i32>,
        toughness: Option<i32>,
        keywords_granted: &[Keyword],
        keyword_args_granted: &[KeywordArgs],
    ) -> Result<()> {
        // AnimateAll: set base P/T and/or grant keywords to all matching permanents
        // Similar to PumpAllCreatures but sets base P/T instead of bonuses.
        //
        // mtg-907: `filter` is the card's `ValidCards$` string (the ValidTgts
        // grammar — e.g. `Creature.YouCtrl`, `Planeswalker.YouCtrl`,
        // `Permanent.OppCtrl`). Parse it once with the canonical
        // `TargetRestriction` and match via `matches_with_controller`, EXACTLY
        // like the sibling `execute_pump_all_creatures`. This replaces the old
        // raw `filter.contains("Creature")` / `.contains("YouCtrl")` substring
        // checks (a "No Hacky String Operations On Structured Data" violation
        // that silently ignored qualifiers like `.powerGE4` / `.nonArtifact` and
        // could false-match a subtype string against a type token). The result is
        // identical for every ValidCards$ string shipping today (verified:
        // Creature/Planeswalker/Permanent + YouCtrl/OppCtrl) and is now also
        // correct for qualified filters — see the commit's rules review.
        let restriction = crate::core::TargetRestriction::parse(filter);
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.try_get(card_id)?;
                if restriction.matches_with_controller(card, controller, card.controller) {
                    Some(card_id)
                } else {
                    None
                }
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
                for kw_args in keyword_args_granted.iter() {
                    card.grant_keyword_args_until_eot(kw_args);
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

    /// [`Effect::SetBasePowerToughness`]: an "animate" effect — set the target's
    /// temporary base P/T until end of turn, optionally grant keywords, add card
    /// types / subtypes, and (RemoveCreatureTypes$ True) strip existing creature
    /// subtypes. Used by Mishra's Factory-style manlands and Animate effects
    /// (Flexible Waterbender, Turtle-Duck, Soulstone Sanctuary).
    ///
    /// All mutations are undo-logged so a rewind+replay restores the exact prior
    /// typeline / keywords (mtg-610, mtg-614): the temp base-stat override via
    /// `set_temp_base_stats_logged`, the typeline+keyword grant via a single
    /// `AnimateTypeline` GameAction, and the ManaSourceCache invalidation when an
    /// animated mana source's classification changes (Mishra's Factory).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::actions) fn execute_set_base_power_toughness(
        &mut self,
        target: CardId,
        power: Option<i32>,
        toughness: Option<i32>,
        keywords_granted: &[Keyword],
        keyword_args_granted: &[KeywordArgs],
        types_added: &[crate::core::CardType],
        subtypes_added: &[crate::core::Subtype],
        remove_creature_subtypes: bool,
    ) -> Result<()> {
        // Skip if target is still placeholder (0) - no valid targets found
        if target.is_placeholder() {
            // Spell fizzles - no valid targets
            return Ok(());
        }
        // Set temporary base P/T override (until end of turn)
        // This is used by Animate effects like Flexible Waterbender,
        // Turtle-Duck, and manlands such as Mishra's Factory.
        //
        // Set the temp base P/T (if specified) via the logged helper so
        // the override is reversible by the undo log (mtg-614 hole (c)).
        if power.is_some() || toughness.is_some() {
            self.set_temp_base_stats_logged(target, power.map(|p| p as i8), toughness.map(|t| t as i8));
        }
        let card = self.cards.get_mut(target)?;
        let card_name = card.name.clone();
        let _old_power = card.current_power();
        let _old_toughness = card.current_toughness();

        // Grant temporary keywords (until end of turn).
        // Note: Uses same approach as PumpCreature - keywords added to permanent set.
        // Record ONLY the keywords this animate actually adds (those not
        // already present) so the `AnimateTypeline` undo entry can remove
        // exactly them on a rewind without stripping a printed/other-source
        // keyword (mtg-610: Soulstone Sanctuary's Vigilance was leaking
        // across rewind+replay because it was inserted but never undone).
        let mut granted_keywords: smallvec::SmallVec<[Keyword; 2]> = smallvec::SmallVec::new();
        for kw in keywords_granted {
            if !card.keywords.contains(*kw) {
                card.keywords.insert(*kw);
                granted_keywords.push(*kw);
            }
        }
        // Also grant complex (parameterized) keywords (e.g. Landwalk:Forest).
        // Vec used (not SmallVec) to match the undo log field type, which uses
        // Vec to keep GameAction::AnimateTypeline from inflating the enum's size.
        let mut granted_keyword_args_vec: Vec<KeywordArgs> = Vec::new();
        for kw_args in keyword_args_granted.iter() {
            if !card.keywords.contains(kw_args.keyword()) {
                card.keywords.insert_complex(kw_args.clone());
                granted_keyword_args_vec.push(kw_args.clone());
            }
        }

        // Snapshot the pre-animate typeline + tracking vectors so the
        // mutation can be logged as a reversible `AnimateTypeline`
        // GameAction (mtg-610). Captured BEFORE any push/drain below so
        // `undo()` restores the exact prior state and a rewind+replay
        // round-trips deterministically (the cleanup-step revert relies
        // on the tracking vectors, which can drift across rewinds).
        let prev_types = card.types.clone();
        let prev_subtypes = card.subtypes.clone();
        let prev_temp_animate_types = card.temp_animate_types.clone();
        let prev_temp_animate_subtypes = card.temp_animate_subtypes.clone();
        let prev_temp_removed_subtypes = card.temp_removed_subtypes.clone();

        // Animate: add card types (Mishra's Factory becomes
        // Land + Artifact + Creature). We track only the types we
        // actually push so cleanup_temporary_effects can remove them
        // without disturbing the printed type line.
        for ty in types_added {
            if !card.types.contains(ty) {
                card.types.push(*ty);
                card.temp_animate_types.push(*ty);
            }
        }

        // Animate: optionally strip pre-existing creature subtypes
        // (RemoveCreatureTypes$ True). For the common manland case
        // this is a no-op because the printed card has no subtypes,
        // but it matters for cards that animate into a *different*
        // creature type than their printed line.
        if remove_creature_subtypes && !card.subtypes.is_empty() {
            let removed: smallvec::SmallVec<[crate::core::Subtype; 2]> = card.subtypes.drain(..).collect();
            card.temp_removed_subtypes.extend(removed);
        }

        // Add subtypes (Assembly-Worker, etc.)
        for st in subtypes_added {
            if !card.subtypes.contains(st) {
                card.subtypes.push(st.clone());
                card.temp_animate_subtypes.push(st.clone());
            }
        }

        // If we touched types or subtypes, refresh the cache flags
        // (`is_creature`, `is_artifact`, etc.) so combat / mana / target
        // logic sees the new typeline immediately.
        let types_changed = !types_added.is_empty() || !subtypes_added.is_empty() || remove_creature_subtypes;
        if types_changed {
            let types = card.types.clone();
            let subtypes = card.subtypes.clone();
            let name = card.name.clone();
            card.definition.cache.update_from_types(&types);
            card.definition.cache.update_from_subtypes(&subtypes, name.as_str());

            // A card that just became a creature must record its
            // ETB turn so summoning-sickness logic works (CR 302.1).
            // Without this, animated lands could attack the same turn
            // they were played even without Haste — and conversely,
            // if `turn_entered_battlefield` is set to the current
            // turn the engine correctly demands Haste.
            //
            // The land itself entered the battlefield earlier (on a
            // prior turn for Mishra's Factory's typical use), so we
            // intentionally leave `turn_entered_battlefield` alone:
            // the land's existing entry timestamp already satisfies
            // summoning sickness once it's been on the battlefield
            // for a turn, mirroring Forge-Java's "becomes a creature"
            // not resetting summoning sickness.
        }

        // Snapshot post-animate state we need *after* dropping the
        // mutable card borrow (so we can re-borrow self below).
        let new_power = card.current_power();
        let new_toughness = card.current_toughness();
        let is_mana_source_now = card.definition.cache.is_mana_source;

        // Log the typeline mutation as a reversible GameAction so a
        // rewind+replay restores the exact prior typeline AND removes the
        // keywords this animate granted (mtg-610). Logged when we changed
        // the typeline OR newly granted a keyword (Soulstone Sanctuary
        // changes both, but a hypothetical keyword-only animate must still
        // be reversible).
        if types_changed || !granted_keywords.is_empty() || !granted_keyword_args_vec.is_empty() {
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::AnimateTypeline {
                    card_id: target,
                    prev_types,
                    prev_subtypes,
                    prev_temp_animate_types,
                    prev_temp_animate_subtypes,
                    prev_temp_removed_subtypes,
                    granted_keywords,
                    granted_keyword_args: granted_keyword_args_vec,
                },
                prior_log_size,
            );
        }

        // If a permanent's typeline changed AND it's a mana source,
        // the per-player ManaSourceCache classification may now be
        // wrong. Mishra's Factory is the canonical case: it was a
        // colorless *simple* source before animate, but post-animate
        // a from-scratch scan re-classifies it as a *complex* source
        // (because creatures with mana abilities go to
        // `complex_sources`). Mark all mana caches dirty so the next
        // ManaEngine update rebuilds, and bump the mana-state version
        // so memoized engine state is invalidated. Mirrors what the
        // undo path already does.
        if types_changed && is_mana_source_now {
            for (_, cache) in &mut self.mana_caches {
                cache.mark_dirty();
            }
            self.increment_mana_version();
        }

        // Log the effect
        if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
            let kw_str = if keywords_granted.is_empty() {
                String::new()
            } else {
                let kws: Vec<_> = keywords_granted.iter().map(|k| format!("{:?}", k)).collect();
                format!(" and gains {}", kws.join(", "))
            };

            if power.is_some() || toughness.is_some() {
                self.logger.gamelog(&format!(
                    "{} base P/T set to {}/{}{}",
                    card_name, new_power, new_toughness, kw_str
                ));
            } else if !keywords_granted.is_empty() {
                self.logger.gamelog(&format!(
                    "{} gains {}",
                    card_name,
                    keywords_granted
                        .iter()
                        .map(|k| format!("{:?}", k))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            // Surface "becomes a creature" so the gamelog reads
            // sensibly when a manland animates.
            if !types_added.is_empty() {
                let type_names: Vec<_> = types_added.iter().map(|t| format!("{:?}", t)).collect();
                self.logger
                    .gamelog(&format!("{} becomes {}", card_name, type_names.join(" + ")));
            }
        }
        Ok(())
    }
}
