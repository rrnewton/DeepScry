//! Master spell/ability/land selection dispatcher (`choose_best_spell`)
//!
//! Part of the heuristic AI controller, split out of the former monolithic
//! `heuristic_controller.rs`. See `heuristic_controller/README.md` for the
//! submodule map. This is a pure structural refactor of the Java-Forge AI
//! port — no decision logic changed.

use super::*;

impl HeuristicController {
    /// Choose the best spell to cast from available options
    ///
    /// This implements the core decision logic from AiController.chooseSpellAbilityToPlay()
    /// Reference: AiController.java:1415-1449
    ///
    /// Priority order (like Java):
    /// 1. Check for "PlayBeforeLandDrop" cards (special timing requirements)
    /// 2. Play land (if available and should play)
    /// 3. Cast creatures (best evaluation first)
    /// 4. Cast other spells (removal, pump, etc.)
    /// 5. Pass priority
    pub(crate) fn choose_best_spell(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        if available.is_empty() {
            return None;
        }

        // Phase 1: Check for "PlayBeforeLandDrop" cards
        // TODO(mtg-XX): Implement PlayBeforeLandDrop check
        // Java: CardLists.filter(player.getCardsIn(ZoneType.Hand),
        //                        CardPredicates.hasSVar("PlayBeforeLandDrop"))

        // Phase 2: Cast spells (creatures, pumps, etc.)
        // IMPORTANT: Cast spells BEFORE playing lands to ensure aggressive gameplay

        // 2a: Evaluate pump spells first (they can enable attacks)
        // Reference: PumpAi.checkPhaseRestrictions() lines 98-103
        // Instant-speed pumps should NOT be cast outside of combat (with exceptions)
        for ability in available {
            if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
                if let Some(spell_card) = view.get_card(*card_id) {
                    // Check if this is a pump spell (has PumpCreature effect)
                    for effect in &spell_card.effects {
                        if let crate::core::Effect::PumpCreature {
                            power_bonus,
                            toughness_bonus,
                            ..
                        } = effect
                        {
                            // Check phase restrictions for instant pumps
                            // Reference: PumpAi.java:98-103
                            let current_step = view.current_step();
                            let is_instant = spell_card.is_instant();

                            // Instant pumps should only be cast during combat (or with good reason pre-combat)
                            // Don't cast instant pumps if:
                            // - We're before combat begins, OR
                            // - We're after declare blockers
                            // Exception: If the pump makes a non-attacker into an attacker (pre-combat only)
                            let should_hold_for_combat = is_instant
                                && (current_step < crate::game::phase::Step::BeginCombat
                                    || current_step > crate::game::phase::Step::DeclareBlockers);

                            if should_hold_for_combat && current_step < crate::game::phase::Step::BeginCombat {
                                // Pre-combat: Only cast if it makes a non-attacker into an attacker
                                // AND it's not a combat trick we should hold
                                // This is evaluated in should_cast_pump with combat trick detection
                            }

                            // This is a pump spell - evaluate whether we should cast it
                            // For pump spells, we need to determine the target
                            // For now, evaluate if pumping our best creature would be good

                            // Get our creatures (typically 2-8)
                            let our_creatures: SmallVec<[&Card; 8]> = view
                                .battlefield()
                                .iter()
                                .filter_map(|&id| view.get_card(id))
                                .filter(|c| c.owner == self.player_id && c.is_creature())
                                .collect();

                            // Try each potential target
                            for creature in &our_creatures {
                                // Extract keywords that would be granted
                                // TODO: Parse keywords from effect or spell text
                                let keywords_granted: Vec<String> = vec![];

                                if self.should_cast_pump(
                                    creature,
                                    *power_bonus,
                                    *toughness_bonus,
                                    &keywords_granted,
                                    view,
                                ) {
                                    // This pump spell would be valuable - cast it
                                    return Some(ability.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2a2: Cast mana-producing artifacts early (Sol Ring, Arcane Signet, etc.)
        // In the early game (turns 1-5), mana rocks are extremely valuable for ramping.
        // Cast them before creatures to accelerate future turns.
        let turn_number = view.turn_number();
        if turn_number <= 5 {
            for ability in available {
                if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
                    if let Some(card) = view.get_card(*card_id) {
                        // Check if this is a mana-producing artifact (not a creature)
                        // Check both cache flag AND activated abilities for mana production
                        if card.is_artifact() && !card.is_creature() {
                            let has_mana_ability = card.definition.cache.is_mana_source
                                || card.activated_abilities.iter().any(|ab| ab.is_mana_ability);
                            if has_mana_ability {
                                return Some(ability.clone());
                            }
                        }
                    }
                }
            }
        }

        // 2b: Cast creatures (best evaluation first, with mana efficiency)
        // Evaluate all castable creatures considering both raw value and mana efficiency
        // This prioritizes curving out in early game while still preferring high-value threats
        let mut best_creature_ability: Option<SpellAbility> = None;
        let mut best_creature_value = i32::MIN;

        // Get game state for mana efficiency calculation
        let available_mana = self.count_available_mana(view);

        for ability in available {
            if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
                if let Some(card) = view.get_card(*card_id) {
                    if card.is_creature() {
                        // Use mana-efficient evaluation in early game (turns 1-5)
                        // In late game, just use raw creature value
                        let value = if turn_number <= 5 {
                            self.evaluate_creature_for_casting(view, *card_id, available_mana, turn_number)
                        } else {
                            self.evaluate_creature(view, *card_id)
                        };
                        if value > best_creature_value {
                            best_creature_value = value;
                            best_creature_ability = Some(ability.clone());
                        }
                    }
                }
            }
        }

        if best_creature_ability.is_some() {
            return best_creature_ability;
        }

        // Phase 2b: Activated abilities (especially removal during combat)
        // Evaluate and use activated abilities intelligently
        // Reference: Java Forge's ability AI in forge-ai/src/main/java/forge/ai/ability/
        for ability in available {
            if let SpellAbility::ActivateAbility { card_id, ability_index } = ability {
                if let Some(source_card) = view.get_card(*card_id) {
                    // Skip mana abilities (let mana system handle those)
                    // Check the SPECIFIC ability being activated, not all abilities on the card
                    let is_mana_ability = source_card
                        .activated_abilities
                        .get(*ability_index)
                        .is_some_and(|ab| ab.is_mana_ability);
                    if is_mana_ability {
                        continue;
                    }

                    // Evaluate if we should use this ability now
                    if self.should_activate_ability(source_card, view) {
                        return Some(ability.clone());
                    }
                }
            }
        }

        // Phase 3: Land play logic (only if we can't cast creatures)
        // Collect land play abilities (typically 1-3 lands in hand)
        let land_plays: SmallVec<[&SpellAbility; 4]> = available
            .iter()
            .filter(|sa| matches!(sa, SpellAbility::PlayLand { .. }))
            .collect();

        if !land_plays.is_empty() {
            // Extract land card IDs
            let land_ids: SmallVec<[CardId; 4]> = land_plays
                .iter()
                .filter_map(|sa| {
                    if let SpellAbility::PlayLand { card_id, .. } = sa {
                        Some(*card_id)
                    } else {
                        None
                    }
                })
                .collect();

            // Choose best land
            if let Some(best_land_id) = self.choose_best_land(view, &land_ids) {
                // Check if we should play this land (may hold for Main 2 bluffing)
                if self.should_play_land(best_land_id, view) {
                    // Find and return the corresponding land play ability
                    for ability in land_plays {
                        if let SpellAbility::PlayLand { card_id, .. } = ability {
                            if *card_id == best_land_id {
                                return Some((*ability).clone());
                            }
                        }
                    }
                }
            }
        }

        // Phase 4: Cast other spells (removal, damage, etc.)
        for ability in available {
            if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
                if let Some(spell_card) = view.get_card(*card_id) {
                    // Skip creatures and pumps (already handled above)
                    if spell_card.is_creature() {
                        continue;
                    }

                    // Check if this is a pump spell (skip, already handled)
                    let is_pump = spell_card
                        .effects
                        .iter()
                        .any(|e| matches!(e, crate::core::Effect::PumpCreature { .. }));
                    if is_pump {
                        continue;
                    }

                    // Evaluate other spells (removal, damage, etc.)
                    if self.should_cast_spell(spell_card, view) {
                        return Some(ability.clone());
                    }
                }
            }
        }

        // Pass priority if nothing good to do
        None
    }
}
