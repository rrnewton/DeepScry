//! Spell-casting evaluation: pump tricks, removal, and the should_cast_* family
//!
//! Part of the heuristic AI controller, split out of the former monolithic
//! `heuristic_controller.rs`. See `heuristic_controller/README.md` for the
//! submodule map. This is a pure structural refactor of the Java-Forge AI
//! port — no decision logic changed.

use super::*;

impl HeuristicController {
    /// Evaluate whether we should cast a pump spell on a creature
    ///
    /// Reference: ComputerUtilCard.shouldPumpCard() (lines 1291-1600+)
    ///
    /// This is a faithful port of Java's pump spell evaluation logic.
    /// Currently implements pre-combat evaluation for Main Phase 1.
    ///
    /// Parameters:
    /// - target: The creature we're considering pumping
    /// - power_bonus: +P from the pump spell
    /// - toughness_bonus: +T from the pump spell
    /// - keywords_granted: Keywords granted by the pump (e.g., ["Trample", "Haste"])
    /// - view: Current game state
    ///
    /// Returns true if we should cast the pump spell now.
    ///
    /// TODO: Implement combat trick timing (holding instant-speed pumps until declare blockers)
    /// TODO: Implement during-combat evaluation (save creatures, kill blockers, lethal damage)
    pub(crate) fn should_cast_pump(
        &self,
        target: &Card,
        power_bonus: i32,
        toughness_bonus: i32,
        keywords_granted: &[String],
        view: &GameStateView,
    ) -> bool {
        // Basic validity checks

        // Can't pump if new toughness would be <= 0 (creature dies)
        // Java: if (c.getNetToughness() + toughness <= 0) { return false; }
        let current_toughness = i32::from(target.current_toughness()) + target.power_bonus;
        if current_toughness + toughness_bonus <= 0 {
            return false;
        }

        let current_step = view.current_step();
        let current_power = i32::from(target.current_power());

        // Create a hypothetical pumped creature to evaluate
        let pumped_power = current_power + power_bonus;
        let _pumped_toughness = current_toughness + toughness_bonus;

        // Combat trick detection (Reference: ComputerUtilCard.java:1416-1431)
        // A spell is a "combat trick" if:
        // 1. Target creature has power > 0 (not obvious)
        // 2. Keywords are empty OR only contain Trample/FirstStrike/DoubleStrike
        // 3. We're in pre-combat main phase
        let is_combat_trick_candidate = current_power > 0
            && keywords_granted
                .iter()
                .all(|kw| kw == "Trample" || kw == "First Strike" || kw == "Double Strike")
            && current_step == crate::game::phase::Step::Main1;

        // Phase-based evaluation

        // Get opponent info
        let opponent_life = view.opponent_life();

        // Collect opponent creatures (potential blockers, typically 2-8)
        let opponent_creatures: SmallVec<[&Card; 8]> = view
            .battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| c.owner != self.player_id && c.is_creature())
            .collect();

        // PHASE 1: Pre-combat evaluation (Main1)
        // Reference: ComputerUtilCard.java:1345-1431
        if current_step == crate::game::phase::Step::Main1 {
            // Case 1: Will this pump make a non-attacker into an attacker?
            // Java: if (!doesCreatureAttackAI(ai, c) && doesSpecifiedCreatureAttackAI(ai, pumped))
            // This is the most important case for pre-combat pumps
            let would_attack_unpumped = self.should_attack(target, view);

            if !would_attack_unpumped
                && self.would_attack_if_pumped(target, power_bonus, toughness_bonus, keywords_granted, view)
            {
                // Calculate threat level if it attacked unblocked
                // Java: float threat = 1.0f * ComputerUtilCombat.damageIfUnblocked(pumped, opp, combat, true) / opp.getLife();
                let threat = pumped_power as f32 / opponent_life as f32;

                // Check if creature would be unblockable
                // Java: if (oppCreatures.stream().noneMatch(CardPredicates.possibleBlockers(pumped)))
                let has_blockers = opponent_creatures.iter().any(|blocker| {
                    // Simplified blocking check (would need to account for keywords granted)
                    self.can_block_simple(target, blocker, keywords_granted)
                });

                let mut chance = threat;
                if !has_blockers {
                    // Unblockable = 2x more valuable
                    chance *= 2.0;
                }

                // If 0-power creature self-pumps to get power, it's very valuable
                // Java: if (c.getNetPower() == 0 && c == sa.getHostCard() && power > 0) { threat *= 4; }
                if current_power == 0 && power_bonus > 0 {
                    chance *= 4.0;
                }

                // Combat trick detection: if this is a combat trick, DON'T cast it now
                // Wait until Declare Blockers to get more value
                // Reference: ComputerUtilCard.java:1416-1431
                if is_combat_trick_candidate && chance < 0.3 {
                    // Hold the combat trick for later unless the threat is very high
                    return false;
                }

                // Cast if threat is significant (>= 10% of opponent's life in damage)
                if chance >= 0.1 {
                    return true;
                }
            }
        }

        // PHASE 2: During combat evaluation (Declare Blockers)
        // Reference: ComputerUtilCard.java:1468-1600
        if current_step == crate::game::phase::Step::DeclareBlockers {
            let combat = view.combat();

            // Check if target creature is in combat
            let is_attacking = combat.is_attacking(target.id);
            let is_blocking = combat.is_blocking(target.id);

            if !is_attacking && !is_blocking {
                // Target not in combat - don't pump during declare blockers
                return false;
            }

            // Get effective stats for damage calculations
            let target_power = view
                .get_effective_power(target.id)
                .unwrap_or_else(|| i32::from(target.current_power()));
            let target_toughness = view
                .get_effective_toughness(target.id)
                .unwrap_or_else(|| i32::from(target.current_toughness()));
            let pumped_effective_power = target_power + power_bonus;
            let pumped_effective_toughness = target_toughness + toughness_bonus;

            if is_attacking {
                // Case: Our creature is attacking
                let blockers = combat.get_blockers(target.id);

                if blockers.is_empty() {
                    // Unblocked attacker - pump to deal lethal damage
                    if pumped_power >= opponent_life {
                        return true;
                    }

                    // Calculate total damage from all attackers to check for lethal
                    let mut total_damage = 0i32;
                    for &attacker_id in combat.attackers.keys() {
                        if attacker_id == target.id {
                            total_damage += pumped_effective_power;
                        } else if !combat.is_blocked(attacker_id) {
                            if let Some(atk_card) = view.get_card(attacker_id) {
                                let atk_power = view
                                    .get_effective_power(attacker_id)
                                    .unwrap_or_else(|| i32::from(atk_card.current_power()));
                                total_damage += atk_power;
                            }
                        } else {
                            // Blocked attacker - only counts trample damage
                            if let Some(atk_card) = view.get_card(attacker_id) {
                                if atk_card.has_trample() {
                                    let atk_power = view
                                        .get_effective_power(attacker_id)
                                        .unwrap_or_else(|| i32::from(atk_card.current_power()));
                                    let blocker_toughness: i32 = combat
                                        .get_blockers(attacker_id)
                                        .iter()
                                        .filter_map(|&b| view.get_card(b))
                                        .map(|b| {
                                            view.get_effective_toughness(b.id)
                                                .unwrap_or_else(|| i32::from(b.current_toughness()))
                                        })
                                        .sum();
                                    let trample_damage = (atk_power - blocker_toughness).max(0);
                                    total_damage += trample_damage;
                                }
                            }
                        }
                    }

                    // Pump if it would be lethal
                    if total_damage >= opponent_life {
                        return true;
                    }
                } else {
                    // Blocked attacker - evaluate combat outcome
                    let total_blocker_power: i32 = blockers
                        .iter()
                        .filter_map(|&b| view.get_card(b))
                        .map(|b| {
                            view.get_effective_power(b.id)
                                .unwrap_or_else(|| i32::from(b.current_power()))
                        })
                        .sum();

                    let total_blocker_toughness: i32 = blockers
                        .iter()
                        .filter_map(|&b| view.get_card(b))
                        .map(|b| {
                            view.get_effective_toughness(b.id)
                                .unwrap_or_else(|| i32::from(b.current_toughness()))
                        })
                        .sum();

                    // Check for first strike on either side (for future damage race logic)
                    let _attacker_has_first_strike = target.has_first_strike() || target.has_double_strike();
                    let _blocker_has_first_strike = blockers
                        .iter()
                        .filter_map(|&b| view.get_card(b))
                        .any(|b| b.has_first_strike() || b.has_double_strike());

                    // 1. Save our creature: Would we die without pump but survive with it?
                    // Note: First strike matters for damage race timing, but simplify for now
                    // as lethal damage is still lethal regardless of timing
                    let would_die_without_pump = total_blocker_power >= target_toughness;

                    let would_survive_with_pump =
                        pumped_effective_toughness > total_blocker_power || target.has_indestructible();

                    if would_die_without_pump && would_survive_with_pump {
                        return true;
                    }

                    // 2. Kill blockers: Can pumping let us kill blockers that would survive?
                    for &blocker_id in &blockers {
                        if let Some(blocker) = view.get_card(blocker_id) {
                            let blocker_toughness = view
                                .get_effective_toughness(blocker_id)
                                .unwrap_or_else(|| i32::from(blocker.current_toughness()));

                            // Would this blocker die without pump?
                            let blocker_dies_without_pump =
                                target_power >= blocker_toughness || target.has_deathtouch();

                            // Would this blocker die with pump?
                            let blocker_dies_with_pump =
                                pumped_effective_power >= blocker_toughness || target.has_deathtouch();

                            // Pump if it would kill a blocker that wouldn't die otherwise
                            if !blocker_dies_without_pump && blocker_dies_with_pump && !blocker.has_indestructible() {
                                return true;
                            }
                        }
                    }

                    // 3. Trample damage: If we have trample, pump to deal more damage
                    if target.has_trample() || keywords_granted.iter().any(|k| k == "Trample") {
                        let damage_without_pump = (target_power - total_blocker_toughness).max(0);
                        let damage_with_pump = (pumped_effective_power - total_blocker_toughness).max(0);

                        if damage_with_pump > damage_without_pump && damage_with_pump >= opponent_life {
                            return true;
                        }
                    }
                }
            } else if is_blocking {
                // Case: Our creature is blocking
                let attackers_blocked = combat.blockers.get(&target.id).cloned().unwrap_or_default();

                if attackers_blocked.is_empty() {
                    return false;
                }

                // Calculate total attacking power
                let total_attacker_power: i32 = attackers_blocked
                    .iter()
                    .filter_map(|&a| view.get_card(a))
                    .map(|a| {
                        view.get_effective_power(a.id)
                            .unwrap_or_else(|| i32::from(a.current_power()))
                    })
                    .sum();

                // Check for first strike (for future damage race logic)
                let _attacker_has_first_strike = attackers_blocked
                    .iter()
                    .filter_map(|&a| view.get_card(a))
                    .any(|a| a.has_first_strike() || a.has_double_strike());
                let _blocker_has_first_strike = target.has_first_strike() || target.has_double_strike();

                // 1. Save our blocker
                // Note: First strike timing could matter but simplify for now
                let would_die_without_pump = total_attacker_power >= target_toughness;

                let would_survive_with_pump =
                    pumped_effective_toughness > total_attacker_power || target.has_indestructible();

                if would_die_without_pump && would_survive_with_pump {
                    return true;
                }

                // 2. Kill attackers with pump
                for &attacker_id in &attackers_blocked {
                    if let Some(attacker) = view.get_card(attacker_id) {
                        let attacker_toughness = view
                            .get_effective_toughness(attacker_id)
                            .unwrap_or_else(|| i32::from(attacker.current_toughness()));

                        let attacker_dies_without_pump = target_power >= attacker_toughness || target.has_deathtouch();
                        let attacker_dies_with_pump =
                            pumped_effective_power >= attacker_toughness || target.has_deathtouch();

                        if !attacker_dies_without_pump && attacker_dies_with_pump && !attacker.has_indestructible() {
                            return true;
                        }
                    }
                }

                // 3. Reduce trample damage by pumping toughness
                let any_trampler = attackers_blocked
                    .iter()
                    .filter_map(|&a| view.get_card(a))
                    .any(|a| a.has_trample());

                if any_trampler && toughness_bonus > 0 {
                    // Pumping toughness reduces trample damage to us
                    return true;
                }
            }

            // No good combat reason to pump
            return false;
        }

        // PHASE 3: Post-combat or other phases
        // Generally don't cast pump spells outside of Main1 or Declare Blockers
        if current_step != crate::game::phase::Step::Main1 && current_step != crate::game::phase::Step::DeclareBlockers
        {
            return false;
        }

        // Legacy evaluation for other cases (will be removed once combat logic is complete)
        let would_attack_unpumped = self.should_attack(target, view);

        if !would_attack_unpumped {
            // Creature doesn't attack normally - would it attack if pumped?
            // Simplified check: creature with power > 0 after pump might attack
            if pumped_power > 0 {
                // Calculate threat level if it attacked unblocked
                // Java: float threat = 1.0f * ComputerUtilCombat.damageIfUnblocked(pumped, opp, combat, true) / opp.getLife();
                let threat = pumped_power as f32 / opponent_life as f32;

                // Check if creature would be unblockable
                // Java: if (oppCreatures.stream().noneMatch(CardPredicates.possibleBlockers(pumped)))
                let has_blockers = opponent_creatures.iter().any(|blocker| {
                    // Simplified blocking check (would need to account for keywords granted)
                    self.can_block_simple(target, blocker, keywords_granted)
                });

                let mut chance = threat;
                if !has_blockers {
                    // Unblockable = 2x more valuable
                    chance *= 2.0;
                }

                // If 0-power creature self-pumps to get power, it's very valuable
                // Java: if (c.getNetPower() == 0 && c == sa.getHostCard() && power > 0) { threat *= 4; }
                let base_power = i32::from(target.current_power());
                if base_power == 0 && power_bonus > 0 {
                    chance *= 4.0;
                }

                // Cast if threat is significant (>= 10% of opponent's life in damage)
                if chance >= 0.1 {
                    return true;
                }
            }
        }

        // Case 2: Grant haste to enable attacking this turn
        // Java: if (keywords.contains("Haste") && c.hasSickness() && !c.isTapped())
        if keywords_granted.iter().any(|k| k == "Haste") {
            // Check if creature has summoning sickness
            // TODO: We need to check turn_entered_battlefield vs current turn
            // For now, simple heuristic: if the creature would attack when pumped
            if pumped_power > 0 {
                // Haste is worth about 0.5 + damage threat
                let threat = 0.5 + (0.5 * pumped_power as f32 / opponent_life as f32);
                if threat >= 0.3 {
                    return true;
                }
            }
        }

        // Case 3: Grant evasion (Flying, Unblockable, etc.)
        // Java: if (oppCreatures.stream().anyMatch(CardPredicates.possibleBlockers(c)))
        // Check if creature is currently blockable but would become unblockable
        let currently_blockable = opponent_creatures
            .iter()
            .any(|blocker| self.can_block_simple(target, blocker, &[]));

        if currently_blockable {
            // Would the pumped creature be unblockable?
            let would_be_blockable = opponent_creatures
                .iter()
                .any(|blocker| self.can_block_simple(target, blocker, keywords_granted));

            if !would_be_blockable && pumped_power > 0 {
                // Granting evasion is valuable - worth ~0.5 * damage potential
                let threat = 0.5 * pumped_power as f32 / opponent_life as f32;
                if threat >= 0.2 {
                    return true;
                }
            }
        }

        // Default: don't cast
        false
    }

    /// Evaluate whether to cast a non-creature, non-pump spell
    ///
    /// Reference: Various spell AI classes in forge-ai/src/main/java/forge/ai/ability/
    pub(crate) fn should_cast_spell(&self, spell: &Card, view: &GameStateView) -> bool {
        // Check for card draw spells
        // Reference: DrawAi.java:30-120 (checkApiLogic)
        // Cast draw spells when hand is getting low (4 or fewer cards)
        // More aggressive than before (was 2) to keep card advantage flowing
        let has_draw = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DrawCards { .. } | crate::core::Effect::DrawCardsXPaid { .. }
            )
        });
        if has_draw {
            let hand_size = view.hand().len();
            // Draw if we have 4 or fewer cards in hand
            // Reference: DrawAi.java - Java Forge draws when hand < 4-5
            if hand_size <= 4 {
                // Check timing for instant-speed draw - prefer opponent's end step for bluffing
                if spell.is_instant() && !self.should_cast_instant_now(view, spell) {
                    return false;
                }
                return true;
            }
        }

        // Check for removal spells (destroy or damage effects)
        // Reference: DestroyAi.java:106-303 (checkApiLogic)
        let has_destroy = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));
        let has_damage = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DealDamage { .. } | crate::core::Effect::DealDamageXPaid { .. }
            )
        });

        if has_destroy || has_damage {
            // Check if there's a valid removal target AND if timing is right
            // Reference: DestroyAi.java:246 calls useRemovalNow() before committing
            if let Some(target) = self.choose_best_removal_target(spell, view) {
                if self.use_removal_now(spell, target, view) {
                    return true;
                }
            }
        }

        // Check for counterspells
        // Reference: CounterAi.java:32-226 (checkApiLogic)
        let has_counter = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::CounterSpell { .. }));
        if has_counter && self.should_counter_spell(view) {
            return true;
        }

        // Check for enchantments with static abilities
        // Reference: AttachAi.java:47-91 (checkApiLogic) for Auras
        // Reference: PumpAllAi.java:29-240 (checkApiLogic) for global enchantments
        if spell.definition.cache.is_enchantment {
            // Handle Auras (require targeting)
            if spell.definition.cache.is_aura {
                if self.should_cast_aura(spell, view) {
                    return true;
                }
            } else {
                // Handle global enchantments (no targeting)
                if self.should_cast_global_enchantment(spell, view) {
                    return true;
                }
            }
        }

        // Check for board wipes (DestroyAll, DamageAll, SacrificeAll)
        // Reference: DestroyAllAi.java:52-175 (doMassRemovalLogic)
        let has_mass_removal = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DestroyAll { .. }
                    | crate::core::Effect::DamageAll { .. }
                    | crate::core::Effect::SacrificeAll { .. }
            )
        });
        if has_mass_removal && self.should_cast_board_wipe(spell, view) {
            return true;
        }

        // Check for sacrifice effects (ForceSacrifice)
        let has_force_sac = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::ForceSacrifice { .. }));
        if has_force_sac && self.should_cast_force_sacrifice(view) {
            return true;
        }

        // Check for TapAll effects
        let has_tap_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapAll { .. }));
        if has_tap_all && self.should_cast_tap_all(view) {
            return true;
        }

        // Check for UntapAll effects
        let has_untap_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::UntapAll { .. }));
        if has_untap_all && self.should_cast_untap_all(view) {
            return true;
        }

        // Check for SetLife effects
        let has_set_life = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::SetLife { .. }));
        if has_set_life && self.should_cast_set_life(spell, view) {
            return true;
        }

        // Check for LoseLife effects (targeting opponent)
        let has_lose_life = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::LoseLife { .. }));
        if has_lose_life {
            // LoseLife targeting opponent is almost always worth casting
            return true;
        }

        // Check for Fight effects (creature mutual damage)
        // Reference: FightAi.java:27-108 (checkApiLogic)
        let has_fight = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::Fight { .. }));
        if has_fight && self.should_cast_fight(view) {
            return true;
        }

        // Check for GainControl effects (steal creature)
        // Reference: ControlGainAi.java (checkApiLogic)
        let has_gain_control = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::GainControl { .. }));
        if has_gain_control && self.should_cast_gain_control(view) {
            return true;
        }

        // Check for PutCounterAll effects (mass counter placement)
        // Reference: CountersPutAllAi.java:25-115 (checkApiLogic)
        let has_put_counter_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::PutCounterAll { .. }));
        if has_put_counter_all && self.should_cast_put_counter_all(spell, view) {
            return true;
        }

        // Check for ChangeZoneAll effects (mass zone changes: bounce, exile, etc.)
        // Reference: ChangeZoneAllAi.java:20-200 (canPlay)
        let has_change_zone_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::ChangeZoneAll { .. }));
        if has_change_zone_all && self.should_cast_change_zone_all(spell, view) {
            return true;
        }

        // Check for Discard effects (Hymn to Tourach, Mind Rot, etc.)
        // Reference: DiscardAi.java:27-120 (checkApiLogic)
        // Discard is almost always good when opponent has cards in hand
        let has_discard = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DiscardCards { .. } | crate::core::Effect::DiscardCardsXPaid { .. }
            )
        });
        if has_discard && self.should_cast_discard(view) {
            return true;
        }

        // Check for single-target Tap effects (Icy Manipulator spell mode, etc.)
        // Reference: TapAi.java:26-100 (checkApiLogic)
        let has_tap = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapPermanent { .. }));
        if has_tap && self.should_cast_tap_permanent(view) {
            return true;
        }

        // Check for single-target Untap effects
        let has_untap = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::UntapPermanent { .. }));
        if has_untap {
            // Untapping our own permanents is almost always good
            return true;
        }

        // Check for TapOrUntap effects (Bounding Krasis ETB, etc.)
        let has_tap_or_untap = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapOrUntapPermanent { .. }));
        if has_tap_or_untap {
            // Flexible effect - always worth casting
            return true;
        }

        // Check for DebuffCreature effects (removing keywords from opponent creatures)
        let has_debuff = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DebuffCreature { .. }));
        if has_debuff && self.should_cast_debuff(view) {
            return true;
        }

        // Check for Regenerate spell effects (cast proactively to protect creatures)
        let has_regenerate = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::Regenerate { .. }));
        if has_regenerate {
            // Only cast during combat when our creatures are in danger
            let current_step = view.current_step();
            let is_combat = matches!(
                current_step,
                crate::game::Step::DeclareAttackers
                    | crate::game::Step::DeclareBlockers
                    | crate::game::Step::CombatDamage
            );
            if is_combat {
                return true;
            }
        }

        // Planeswalkers are always worth casting (they provide ongoing value via loyalty abilities)
        if spell
            .types
            .iter()
            .any(|t| matches!(t, crate::core::CardType::Planeswalker))
        {
            return true;
        }

        // Utility artifacts with non-mana activated abilities (Icy Manipulator, etc.).
        // These have no spell effects themselves but provide board control via their
        // activated abilities once on the battlefield. Cast them when the opponent has
        // permanents that those abilities could affect (CR 302.6: artifacts are permanent
        // spells, they don't need ETB effects to be useful).
        if spell.is_artifact() && !spell.is_creature() {
            let has_useful_activated = spell.activated_abilities.iter().any(|ab| !ab.is_mana_ability);
            if has_useful_activated {
                // Only cast if the opponent has relevant permanents the ability can affect
                let opponent_has_permanents = view
                    .battlefield()
                    .iter()
                    .any(|&card_id| view.get_card(card_id).is_some_and(|c| c.controller != self.player_id));
                if opponent_has_permanents {
                    return true;
                }
            }
        }

        // Always-beneficial effects: search library, create tokens, scry, surveil, etc.
        // These effects always benefit the caster and should be cast when possible.
        // Examples: Demonic Tutor (SearchLibrary), Dragon Fodder (CreateToken),
        //           Opt (Scry), Thought Erasure (Surveil), Time Walk (AddTurn)
        //           Mind Sculpt (Mill), Healing Salve (GainLife), Overrun (PumpAllCreatures)
        let has_always_beneficial = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::SearchLibrary { .. }
                    | crate::core::Effect::CreateToken { .. }
                    | crate::core::Effect::Scry { .. }
                    | crate::core::Effect::Surveil { .. }
                    | crate::core::Effect::Loot { .. }
                    | crate::core::Effect::Dig { .. }
                    | crate::core::Effect::CopyPermanent { .. }
                    | crate::core::Effect::ExilePermanent { .. }
                    | crate::core::Effect::Balance { .. }
                    | crate::core::Effect::AddTurn { .. }
                    | crate::core::Effect::Mill { .. }
                    | crate::core::Effect::GainLife { .. }
                    | crate::core::Effect::PumpAllCreatures { .. }
                    | crate::core::Effect::AnimateAll { .. }
                    | crate::core::Effect::MultiplyCounter { .. }
                    | crate::core::Effect::PutCounter { .. }
                    | crate::core::Effect::Proliferate
                    | crate::core::Effect::PreventDamage { .. }
            )
        });
        if has_always_beneficial {
            return true;
        }

        false
    }

    /// Determine if we should cast an instant-speed spell now (bluffing logic)
    ///
    /// Reference: Java Forge phase restriction patterns (e.g., "AtOpponentsCombatOrAfter", "AtEOT")
    /// from various AI files (DestroyAi.java, DrawAi.java, etc.)
    ///
    /// This implements bluffing/deception by holding instant-speed spells until opponent's turn
    /// when possible, to:
    /// 1. Bluff having combat tricks/removal
    /// 2. See what opponent does before committing mana
    /// 3. Maintain maximum flexibility
    ///
    /// Key timing windows for instant-speed spells:
    /// - Opponent's end step: Preferred window (bluffs combat tricks all turn)
    /// - Our Main 2: Acceptable if we need to tap out for combat/attacks
    /// - Emergency: Immediate cast if hand is too full or spell is critical
    ///
    /// Returns true if we should cast the instant now, false if we should hold it.
    pub(crate) fn should_cast_instant_now(&self, view: &GameStateView, spell: &Card) -> bool {
        let current_step = view.current_step();
        let is_our_turn = view.active_player() == self.player_id;

        // Always cast sorcery-speed spells immediately (no bluffing possible)
        if !spell.is_instant() {
            return true;
        }

        // Interrupt 1: Hand is too full (7+ cards) - need to cast to avoid discarding
        // Reference: Similar to Java's hand size management in various AIs
        let hand_size = view.hand().len();
        if hand_size >= 7 {
            return true;
        }

        // Interrupt 2: Opponent's end step - BEST time to cast instant-speed non-combat spells
        // Reference: Java phase restrictions "AtEOT" pattern
        // This maximizes bluffing (held mana all turn = could be removal/combat tricks)
        if !is_our_turn && current_step == crate::game::Step::End {
            return true;
        }

        // Interrupt 3: Our Main 2 - acceptable timing if we're about to pass turn anyway
        // Reference: Java phase restrictions allowing Main 2 casting
        if is_our_turn && current_step == crate::game::Step::Main2 {
            return true;
        }

        // Interrupt 4: Combat phases - if opponent is attacking, might need to respond
        // Though draw spells don't directly interact, casting now prevents telegraphing
        let is_combat = matches!(
            current_step,
            crate::game::Step::DeclareAttackers | crate::game::Step::DeclareBlockers | crate::game::Step::CombatDamage
        );
        if is_combat {
            // During combat, only cast if hand is getting full (5+ cards)
            if hand_size >= 5 {
                return true;
            }
        }

        // Default: Hold the instant-speed spell for a better moment (bluffing)
        // This is the key bluffing logic - by default, don't cast instant-speed
        // draw/utility spells on our turn, wait for opponent's end step
        false
    }

    /// Evaluate whether to cast a counterspell now
    ///
    /// Reference: CounterAi.java:32-226 (checkApiLogic)
    ///
    /// Key logic from Java:
    /// 1. Stack must not be empty (line 40-42)
    /// 2. Target the topmost spell on stack (line 51)
    /// 3. Don't counter friendly spells (line 52)
    /// 4. Don't counter low CMC spells (lines 163-169, configurable)
    /// 5. Prefer countering dangerous spells: creatures, damage, removal (lines 171-182)
    ///
    /// Simplified for now:
    /// - Counter any opponent spell on the stack
    /// - Prioritize creatures, damage spells, and removal
    pub(crate) fn should_counter_spell(&self, view: &GameStateView) -> bool {
        // Stack must have something to counter
        if view.is_stack_empty() {
            return false;
        }

        // Get the topmost spell on the stack (last entry)
        let stack = view.stack();
        let Some(&top_spell_id) = stack.last() else {
            return false;
        };

        let Some(top_spell) = view.get_card(top_spell_id) else {
            return false;
        };

        // Don't counter our own spells!
        if top_spell.owner == self.player_id {
            return false;
        }

        // Evaluate what type of spell it is
        let is_creature = top_spell.is_creature();
        let is_damage_spell = top_spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DealDamage { .. } | crate::core::Effect::DealDamageXPaid { .. }
            )
        });
        let is_removal_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));
        let is_counter_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::CounterSpell { .. }));
        let is_pump_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::PumpCreature { .. }));
        let is_board_wipe = top_spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DestroyAll { .. }
                    | crate::core::Effect::SacrificeAll { .. }
                    | crate::core::Effect::DamageAll { .. }
                    | crate::core::Effect::ChangeZoneAll { .. }
            )
        });
        let is_extra_turn = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::AddTurn { .. }));
        let is_gain_control = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::GainControl { .. }));

        // Always counter dangerous spell types
        // Reference: CounterAi.java:151-182 (configurable countering preferences)
        if is_creature
            || is_damage_spell
            || is_removal_spell
            || is_counter_spell
            || is_pump_spell
            || is_board_wipe
            || is_extra_turn
            || is_gain_control
        {
            return true;
        }

        // For other spells, check mana value (CMC)
        // Don't waste counterspells on very cheap spells unless they're dangerous
        let cmc = top_spell.mana_cost.cmc();
        if cmc >= 2 {
            // Counter anything CMC 2 or higher
            return true;
        }

        // CMC 0-1 spells: counter with 50% chance (simplified from Java's configurable chance)
        // In a real implementation, we'd check the RNG, but for now just counter CMC 1 spells
        cmc >= 1
    }

    /// Evaluate whether to cast a global enchantment (non-Aura)
    ///
    /// Reference: PumpAllAi.java:29-240 (checkApiLogic)
    ///
    /// Global enchantments are permanent effects that buff/debuff creatures.
    /// Examples: Crusade (+1/+1 to white creatures), Bad Moon (+1/+1 to black creatures)
    ///
    /// Key decision logic:
    /// 1. Check if we have creatures that benefit from the buff (static abilities)
    /// 2. Compare our creature count vs opponent's for symmetric effects
    /// 3. Cast if the net benefit is positive for us
    pub(crate) fn should_cast_global_enchantment(&self, spell: &Card, view: &GameStateView) -> bool {
        // Look for ModifyPT static abilities on the enchantment
        let modify_pt_abilities: Vec<_> = spell
            .static_abilities
            .iter()
            .filter_map(|ability| {
                if let crate::core::StaticAbility::ModifyPT {
                    affected,
                    power,
                    toughness,
                    ..
                } = ability
                {
                    Some((affected, *power, *toughness))
                } else {
                    None
                }
            })
            .collect();

        if modify_pt_abilities.is_empty() {
            // No PT modification - check for keyword-granting or other beneficial statics
            // Cast keyword-granting enchantments if we have 2+ creatures that benefit
            let has_keyword_grant = spell.static_abilities.iter().any(|ability| {
                matches!(
                    ability,
                    crate::core::StaticAbility::GrantKeyword { .. } | crate::core::StaticAbility::GrantAbility { .. }
                )
            });

            if has_keyword_grant {
                // Count our creatures on battlefield - cast if we have 2+ to benefit
                let our_creature_count = view
                    .battlefield()
                    .iter()
                    .filter(|&&card_id| {
                        view.get_card(card_id)
                            .is_some_and(|c| c.is_creature() && c.controller == self.player_id)
                    })
                    .count();
                return our_creature_count >= 2;
            }

            // Check for enchantments with triggered abilities (beneficial ETB/upkeep triggers)
            if !spell.triggers.is_empty() {
                // Enchantments with triggers are usually beneficial - cast if we have creatures
                let has_creatures = view.battlefield().iter().any(|&card_id| {
                    view.get_card(card_id)
                        .is_some_and(|c| c.is_creature() && c.controller == self.player_id)
                });
                return has_creatures;
            }

            // Check for RaiseCost / ReduceCost statics (Gloom, Karma, etc.).
            // These are "hate" enchantments that hose a colour or type.
            // Cast if the opponent controls permanents that share the targeted
            // colour — even one permanent is enough to make the enchantment
            // valuable (it slows down every future spell of that colour).
            // CR 601.2f: cost-raising statics apply to all players, but the
            // primary value here is hosing the opponent.
            let raise_cost_abilities: Vec<_> = spell
                .static_abilities
                .iter()
                .filter_map(|ab| {
                    if let crate::core::StaticAbility::RaiseCost { valid_card, .. } = ab {
                        Some(valid_card)
                    } else {
                        None
                    }
                })
                .collect();

            if !raise_cost_abilities.is_empty() {
                // Cast if the opponent has any permanent whose colour/type
                // matches the RaiseCost target — meaning the effect will hose them.
                let opponent_has_target = view.battlefield().iter().any(|&card_id| {
                    let Some(card) = view.get_card(card_id) else {
                        return false;
                    };
                    if card.controller == self.player_id {
                        return false;
                    }
                    raise_cost_abilities
                        .iter()
                        .any(|&valid_card| crate::game::actions::spell_matches_cost_filter(card, valid_card))
                });
                if opponent_has_target {
                    return true;
                }
            }

            // Unknown enchantment type - don't cast
            return false;
        }

        // For each ModifyPT ability, count affected creatures
        for (affected_selector, power_bonus, toughness_bonus) in modify_pt_abilities {
            // Count creatures we control that would benefit
            let our_creatures = view.battlefield().iter().filter_map(|&card_id| {
                let card = view.get_card(card_id)?;
                if card.owner == self.player_id
                    && card.is_creature()
                    && self.creature_matches_selector(card, affected_selector)
                {
                    Some(card_id)
                } else {
                    None
                }
            });

            let our_count = our_creatures.clone().count();
            let our_total_benefit = (power_bonus + toughness_bonus) * our_count as i32;

            // Count opponent creatures that would benefit (for symmetric effects)
            let opponent_creatures = view.battlefield().iter().filter_map(|&card_id| {
                let card = view.get_card(card_id)?;
                if card.owner != self.player_id
                    && card.is_creature()
                    && self.creature_matches_selector(card, affected_selector)
                {
                    Some(card_id)
                } else {
                    None
                }
            });

            let opponent_count = opponent_creatures.count();
            let opponent_total_benefit = (power_bonus + toughness_bonus) * opponent_count as i32;

            // Cast if we benefit more than opponents
            // Reference: PumpAllAi.java uses various calculations, simplified here to net benefit
            if our_total_benefit > opponent_total_benefit && our_count > 0 {
                return true;
            }
        }

        false
    }

    /// Evaluate whether to cast an Aura enchantment
    ///
    /// Reference: AttachAi.java:47-91 (checkApiLogic)
    ///
    /// Auras enchant a creature and provide benefits (or penalties).
    /// Examples: Spirit Link (gain life when creature deals damage), Holy Strength (+1/+2)
    ///
    /// Key decision logic:
    /// 1. Check if there's a valid target creature
    /// 2. Prefer enchanting our own creatures with beneficial effects
    /// 3. Consider targeting opponent's creatures with negative effects
    pub(crate) fn should_cast_aura(&self, spell: &Card, view: &GameStateView) -> bool {
        // Look for beneficial static abilities (ModifyPT with positive values)
        let has_beneficial_pt = spell.static_abilities.iter().any(|ability| {
            if let crate::core::StaticAbility::ModifyPT { power, toughness, .. } = ability {
                // Beneficial if it grants positive power or toughness
                *power > 0 || *toughness > 0
            } else {
                false
            }
        });

        // Look for beneficial triggers (e.g., Spirit Link's life gain trigger)
        let has_beneficial_trigger = !spell.triggers.is_empty();

        if has_beneficial_pt || has_beneficial_trigger {
            // Try to find our best creature to enchant
            let our_creatures: Vec<_> = view
                .battlefield()
                .iter()
                .filter_map(|&card_id| {
                    let card = view.get_card(card_id)?;
                    if card.owner == self.player_id && card.is_creature() {
                        Some(card_id)
                    } else {
                        None
                    }
                })
                .collect();

            if !our_creatures.is_empty() {
                // Cast if we have at least one creature to enchant
                // Target selection will be handled by choose_aura_target
                return true;
            }
        }

        // TODO: Handle curse auras (negative effects on opponent creatures)
        // For now, don't cast those

        false
    }

    /// Evaluate whether to cast a board wipe (DestroyAll/DamageAll)
    ///
    /// Reference: DestroyAllAi.java:52-175 (doMassRemovalLogic)
    ///
    /// Key logic from Java:
    /// 1. Don't cast if opponent has no affected permanents
    /// 2. Cast if opponent creatures are more valuable than ours (creature_eval_threshold=200)
    /// 3. Cast immediately if life is in serious danger during combat
    /// 4. Prefer main phase 2 (after combat) unless emergency
    pub(crate) fn should_cast_board_wipe(&self, spell: &Card, view: &GameStateView) -> bool {
        // Evaluate each player's creatures that would be affected
        let mut our_creature_value: i32 = 0;
        let mut opp_creature_value: i32 = 0;
        let mut our_creature_count: i32 = 0;
        let mut opp_creature_count: i32 = 0;

        // Get the restriction from the effect (for type matching)
        #[allow(clippy::wildcard_enum_match_arm)]
        let restriction = spell.effects.iter().find_map(|e| match e {
            crate::core::Effect::DestroyAll { restriction, .. } => Some(restriction),
            crate::core::Effect::DamageAll { valid_cards, .. } => Some(valid_cards),
            _ => None,
        });

        for &card_id in view.battlefield() {
            let Some(card) = view.get_card(card_id) else {
                continue;
            };

            // Check if this permanent would be affected by the board wipe
            let affected = if let Some(r) = restriction {
                r.matches(card)
            } else {
                card.is_creature()
            };

            if !affected {
                continue;
            }

            // Skip indestructible creatures for DestroyAll
            if card.has_indestructible()
                && spell
                    .effects
                    .iter()
                    .any(|e| matches!(e, crate::core::Effect::DestroyAll { .. }))
            {
                continue;
            }

            let value = self.evaluate_creature(view, card_id);
            if card.controller == self.player_id {
                our_creature_value += value;
                our_creature_count += 1;
            } else {
                opp_creature_value += value;
                opp_creature_count += 1;
            }
        }

        // Don't cast if opponent has no affected creatures
        if opp_creature_count == 0 {
            return false;
        }

        // Java: CREATURE_EVAL_THRESHOLD = 200
        // Cast if opponent creatures are worth significantly more than ours
        let threshold = 200;
        if our_creature_value + threshold < opp_creature_value {
            return true;
        }

        // Cast if we're behind on board and losing life
        // (Simplified version of Java's lifeInSeriousDanger check)
        let our_life = view.life();
        if our_life <= 5 && opp_creature_count > our_creature_count {
            return true;
        }

        // Cast if opponent has significantly more creatures and we're losing
        if opp_creature_count >= our_creature_count + 2 && our_creature_value < opp_creature_value {
            return true;
        }

        false
    }

    /// Evaluate whether to cast a ForceSacrifice spell (e.g., Diabolic Edict)
    ///
    /// Simple heuristic: cast if opponent has creatures on the battlefield.
    /// More valuable if opponent has few creatures (they lose their best one).
    pub(crate) fn should_cast_force_sacrifice(&self, view: &GameStateView) -> bool {
        // Check if any opponent has creatures
        for opp_id in view.opponents() {
            let opp_creature_count = view
                .battlefield()
                .iter()
                .filter(|&&card_id| {
                    view.get_card(card_id)
                        .is_some_and(|c| c.is_creature() && c.controller == opp_id)
                })
                .count();

            if opp_creature_count > 0 {
                return true;
            }
        }
        false
    }

    /// Evaluate whether to cast TapAll
    ///
    /// Reference: TapAllAi.java
    /// Cast if opponent has untapped creatures (e.g., before our attack)
    pub(crate) fn should_cast_tap_all(&self, view: &GameStateView) -> bool {
        // Count opponent untapped creatures
        let opp_untapped_creatures = view
            .battlefield()
            .iter()
            .filter(|&&card_id| {
                view.get_card(card_id)
                    .is_some_and(|c| c.is_creature() && c.controller != self.player_id && !c.tapped)
            })
            .count();

        // Worth tapping if opponent has 2+ untapped creatures
        opp_untapped_creatures >= 2
    }

    /// Evaluate whether to cast UntapAll
    ///
    /// Reference: UntapAllAi.java
    /// Cast if we have tapped creatures that could attack or block
    pub(crate) fn should_cast_untap_all(&self, view: &GameStateView) -> bool {
        // Count our tapped creatures
        let our_tapped_creatures = view
            .battlefield()
            .iter()
            .filter(|&&card_id| {
                view.get_card(card_id)
                    .is_some_and(|c| c.is_creature() && c.controller == self.player_id && c.tapped)
            })
            .count();

        // Worth untapping if we have 2+ tapped creatures
        our_tapped_creatures >= 2
    }

    /// Evaluate whether to cast SetLife
    ///
    /// Cast if it would increase our life total
    pub(crate) fn should_cast_set_life(&self, spell: &Card, view: &GameStateView) -> bool {
        // Find the SetLife effect and its amount
        for effect in &spell.effects {
            if let crate::core::Effect::SetLife { amount, .. } = effect {
                // Cast if it would increase our life
                return *amount > view.life();
            }
        }
        false
    }

    /// Evaluate whether to cast a Discard spell (Hymn to Tourach, Mind Rot, etc.)
    ///
    /// Reference: DiscardAi.java:30-80 (checkApiLogic)
    /// Cast when opponent has cards in hand. More valuable early game.
    pub(crate) fn should_cast_discard(&self, view: &GameStateView) -> bool {
        // Check if any opponent has cards to discard
        let opp_hand_size: usize = view.opponents().map(|opp_id| view.player_hand_size(opp_id)).sum();

        // Don't cast if opponent has no cards to discard
        if opp_hand_size == 0 {
            return false;
        }

        // Always cast if opponent has cards (removing cards is always valuable)
        true
    }

    /// Evaluate whether to cast a single-target Tap spell
    ///
    /// Reference: TapAi.java:30-80 (checkApiLogic)
    /// Best used before combat to tap opponent's best blocker,
    /// or during opponent's turn to tap their best attacker.
    pub(crate) fn should_cast_tap_permanent(&self, view: &GameStateView) -> bool {
        // Check if opponent has untapped creatures we'd want to tap
        let has_untapped_opp_creature = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .any(|c| c.is_creature() && c.controller != self.player_id && !c.tapped);

        if has_untapped_opp_creature {
            return true;
        }

        // Also worthwhile if opponent has untapped mana sources we want to deny
        // (especially before they can cast something)
        false
    }

    /// Evaluate whether to cast a Debuff spell (remove keywords from opponent creature)
    ///
    /// Cast when opponent has creatures with relevant keywords (flying, etc.)
    pub(crate) fn should_cast_debuff(&self, view: &GameStateView) -> bool {
        // Check if opponent has creatures with evasion or other important keywords
        view.battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .any(|c| {
                c.is_creature()
                    && c.controller != self.player_id
                    && (c.keywords.contains(crate::core::Keyword::Flying)
                        || c.keywords.contains(crate::core::Keyword::FirstStrike)
                        || c.keywords.contains(crate::core::Keyword::DoubleStrike)
                        || c.keywords.contains(crate::core::Keyword::Trample)
                        || c.keywords.contains(crate::core::Keyword::Hexproof)
                        || c.keywords.contains(crate::core::Keyword::Indestructible))
            })
    }

    /// Evaluate whether to cast a Fight spell
    ///
    /// Reference: FightAi.java:27-108 (checkApiLogic)
    ///
    /// Key logic from Java:
    /// 1. Need at least one targetable opponent creature
    /// 2. Find a favorable matchup where our creature can kill theirs without dying
    /// 3. Favorable = our power >= their toughness AND our toughness > their power
    ///
    /// For Fight spells, we target one of our creatures and one opponent creature.
    /// The AI should only cast if we can find a favorable fight.
    pub(crate) fn should_cast_fight(&self, view: &GameStateView) -> bool {
        // Get our creatures on the battlefield
        let our_creatures: Vec<_> = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .filter(|c| c.is_creature() && c.controller == self.player_id && !c.tapped)
            .collect();

        // Get opponent creatures on the battlefield
        let opp_creatures: Vec<_> = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .filter(|c| c.is_creature() && c.controller != self.player_id)
            .collect();

        if our_creatures.is_empty() || opp_creatures.is_empty() {
            return false;
        }

        // Look for a favorable matchup
        // Favorable = we can kill them AND we survive
        for our in &our_creatures {
            let our_power = i32::from(our.current_power());
            let our_toughness = i32::from(our.current_toughness());
            let our_has_deathtouch = our.has_deathtouch();

            for opp in &opp_creatures {
                let opp_power = i32::from(opp.current_power());
                let opp_toughness = i32::from(opp.current_toughness());
                let opp_has_deathtouch = opp.has_deathtouch();

                // Skip if opponent has indestructible (can't kill them)
                if opp.has_indestructible() {
                    continue;
                }

                // Check if we can kill them:
                // - With deathtouch: any damage (power > 0) is lethal
                // - Without deathtouch: need power >= toughness
                let we_can_kill = if our_has_deathtouch {
                    our_power > 0
                } else {
                    our_power >= opp_toughness
                };

                // Check if we survive (they can't kill us):
                // - We have indestructible: always survive
                // - They have deathtouch: any damage kills us
                // - Otherwise: their power < our toughness
                let we_survive = our.has_indestructible()
                    || (if opp_has_deathtouch {
                        opp_power == 0 // They can't deal damage
                    } else {
                        opp_power < our_toughness
                    });

                // Favorable fight: we kill them and we survive
                if we_can_kill && we_survive {
                    return true;
                }

                // Check if they can kill us
                let they_can_kill_us = if opp_has_deathtouch {
                    opp_power > 0
                } else {
                    opp_power >= our_toughness
                };

                // Also accept if we can trade for a more valuable creature
                // Trade = both die, but their creature is more valuable
                let we_die = they_can_kill_us && !our.has_indestructible();
                let they_die = we_can_kill && !opp.has_indestructible();
                if we_die && they_die {
                    let our_value = self.evaluate_creature(view, our.id);
                    let their_value = self.evaluate_creature(view, opp.id);
                    if their_value > our_value + 50 {
                        // Trade up: their creature is worth 50+ more points
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Evaluate whether to cast a PutCounterAll spell
    ///
    /// Reference: CountersPutAllAi.java:25-115 (checkApiLogic)
    ///
    /// For beneficial counters (+1/+1): Only cast if we have more creatures benefiting than opponent.
    /// For curse counters (-1/-1): Only cast if 3+ opponent creatures would be killed.
    pub(crate) fn should_cast_put_counter_all(&self, spell: &Card, view: &GameStateView) -> bool {
        use crate::core::{CounterType, Effect};

        // Find the PutCounterAll effect to inspect its parameters
        let (restriction, counter_type, amount) = match spell.effects.iter().find_map(|e| {
            if let Effect::PutCounterAll {
                restriction,
                counter_type,
                amount,
            } = e
            {
                Some((restriction, counter_type, amount))
            } else {
                None
            }
        }) {
            Some(found) => found,
            None => return false,
        };

        // Count how many of our creatures match the restriction vs opponent's
        let mut our_count = 0u32;
        let mut opp_count = 0u32;

        for &card_id in view.battlefield() {
            if let Some(card) = view.get_card(card_id) {
                if restriction.matches(card) {
                    if card.controller == self.player_id {
                        our_count += 1;
                    } else {
                        opp_count += 1;
                    }
                }
            }
        }

        let is_curse = *counter_type == CounterType::M1M1;

        if is_curse {
            // For -1/-1 counters: only cast if we can kill 3+ opponent creatures
            // Reference: CountersPutAllAi.java:72-76
            let mut killable = 0u32;
            for &card_id in view.battlefield() {
                if let Some(card) = view.get_card(card_id) {
                    if restriction.matches(card)
                        && card.controller != self.player_id
                        && card.current_toughness() <= i8::try_from(*amount).unwrap_or(i8::MAX)
                    {
                        killable += 1;
                    }
                }
            }
            killable >= 3
        } else {
            // For beneficial counters: only cast if we benefit more creatures
            // Reference: CountersPutAllAi.java:86-88
            // Also need at least 1 creature of our own to benefit
            our_count > 0 && our_count > opp_count
        }
    }

    /// Evaluate whether to cast a ChangeZoneAll spell (mass zone change)
    ///
    /// Reference: ChangeZoneAllAi.java:20-200 (canPlay)
    ///
    /// For battlefield → hand/exile: Only cast if opponent loses more value than we do.
    /// For graveyard → exile: Cast if opponent has 3+ cards in graveyard.
    /// For graveyard → battlefield: Cast if we have creatures in graveyard (reanimation).
    pub(crate) fn should_cast_change_zone_all(&self, spell: &Card, view: &GameStateView) -> bool {
        use crate::core::Effect;

        // Find the ChangeZoneAll effect to inspect its parameters
        let (restriction, origins, _destination) = match spell.effects.iter().find_map(|e| {
            if let Effect::ChangeZoneAll {
                restriction,
                origins,
                destination,
                shuffle: _,
            } = e
            {
                Some((restriction, origins, destination))
            } else {
                None
            }
        }) {
            Some(found) => found,
            None => return false,
        };

        use crate::zones::Zone;

        // Mass moves that touch the battlefield are evaluated by board value; any
        // other origin (hand/graveyard shuffle like Timetwister) defaults to
        // beneficial. Pick the battlefield arm if it's among the origins.
        let primary_origin = if origins.contains(&Zone::Battlefield) {
            Zone::Battlefield
        } else {
            origins.first().copied().unwrap_or(Zone::Battlefield)
        };

        match primary_origin {
            Zone::Battlefield => {
                // Mass bounce/exile from battlefield: only do if opponent loses more
                // Count matching permanents for each player
                let mut our_value = 0i32;
                let mut opp_value = 0i32;

                for &card_id in view.battlefield() {
                    if let Some(card) = view.get_card(card_id) {
                        if restriction.matches(card) {
                            let value = if card.is_creature() {
                                // Use power + toughness as rough value
                                i32::from(card.current_power()) + i32::from(card.current_toughness())
                            } else {
                                // Non-creature permanents have some value
                                3
                            };

                            if card.controller == self.player_id {
                                our_value += value;
                            } else {
                                opp_value += value;
                            }
                        }
                    }
                }

                // Only cast if opponent loses significantly more value
                // Reference: ChangeZoneAllAi.java:163-166 (creatureEvalThreshold)
                opp_value > our_value + 4
            }
            Zone::Graveyard => {
                // Graveyard effects (exile, reanimation) are almost always beneficial
                // Reference: ChangeZoneAllAi.java:174-194 (graveyard handling)
                true
            }
            Zone::Hand | Zone::Exile | Zone::Library | Zone::Stack | Zone::Command => {
                // Other origin zones: default to casting
                true
            }
        }
    }

    pub(crate) fn should_cast_gain_control(&self, view: &GameStateView) -> bool {
        // Get opponent creatures on the battlefield
        let opp_creatures: Vec<_> = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .filter(|c| c.is_creature() && c.controller != self.player_id)
            .collect();

        if opp_creatures.is_empty() {
            return false;
        }

        // Stealing any creature is almost always good - it's a 2-for-1
        // (remove their creature AND gain one ourselves)
        // Only skip if opponent has literally no creatures
        true
    }

    /// Helper to check if a creature matches an AffectedSelector
    ///
    /// Simplified implementation - matches "Creature.YouCtrl", "Creature.White", etc.
    ///
    /// Note: Wildcard is intentional - AffectedSelector has 80+ variants;
    /// we handle the subset relevant to AI creature targeting decisions.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub(crate) fn creature_matches_selector(&self, creature: &Card, selector: &crate::core::AffectedSelector) -> bool {
        use crate::core::AffectedSelector;

        match selector {
            AffectedSelector::CreaturesYouControl => creature.owner == self.player_id,
            AffectedSelector::AllCreatures => true,
            AffectedSelector::AllCreaturesOfColor { color } => {
                // Color is a String like "White", "Black", etc.
                // We need to check if the creature's colors contain the specified color
                creature.colors.iter().any(|c| {
                    let color_name = format!("{:?}", c); // "Red", "Blue", etc.
                    color.eq_ignore_ascii_case(&color_name)
                })
            }
            AffectedSelector::AllCreaturesOfType { subtype } => creature.subtypes.contains(subtype),
            AffectedSelector::CreatureTypeYouControl { subtype } => {
                creature.owner == self.player_id && creature.subtypes.contains(subtype)
            }
            AffectedSelector::CreatureEquippedBy => {
                // For equipment static abilities, not relevant for casting decision
                false
            }
            AffectedSelector::CreatureEnchantedBy => {
                // For aura static abilities, not relevant for casting decision
                false
            }
            AffectedSelector::CreatureAttachedBy => {
                // For equipment/aura static abilities, not relevant for casting decision
                false
            }
            AffectedSelector::CreaturesOpponentControls => creature.owner != self.player_id,
            // For other selectors, return false (not matched)
            _ => false,
        }
    }

    /// Choose the best creature to target with removal
    ///
    /// Reference: DestroyAi.java:152-247 (target selection logic)
    ///
    /// Key filtering steps from Java:
    /// 1. Get targetable opponent creatures (line 153)
    /// 2. Filter out indestructible (line 157)
    /// 3. Prioritize creatures worth removing (lines 158-160)
    /// 4. Filter out creatures with shield counters (lines 162-163)
    /// 5. Filter out creatures that can regenerate (lines 189-194)
    /// 6. Filter out creatures that will die this turn (line 197)
    /// 7. Select best creature (line 224: getBestCreatureAI)
    ///
    /// Simplified version for now:
    /// - Target opponent's best creature
    /// - Filter out indestructible
    /// - Filter out creatures already dying (toughness <= 0)
    #[allow(clippy::wildcard_enum_match_arm)]
    pub(crate) fn choose_best_removal_target(&self, spell: &Card, view: &GameStateView) -> Option<CardId> {
        // For damage-based removal, find the damage amount
        // For XPaid spells, use the x_paid value stored on the card
        let damage_amount = spell.effects.iter().find_map(|e| match e {
            crate::core::Effect::DealDamage { amount, .. } => Some(*amount),
            crate::core::Effect::DealDamageXPaid { .. } => Some(i32::from(spell.x_paid)),
            _ => None,
        });

        // Get all valid opponent creatures using chained filters (zero intermediate allocations)
        // Filters:
        // 1. Opponent's creatures
        // 2. Not indestructible
        // 3. Not already dying (toughness > 0)
        // 4. For damage spells: toughness <= damage amount
        let opponent_creature_ids: SmallVec<[CardId; 8]> = view
            .battlefield()
            .iter()
            .copied()
            .filter(|&id| {
                if let Some(c) = view.get_card(id) {
                    c.owner != self.player_id
                        && c.is_creature()
                        && !c.has_indestructible()
                        && c.current_toughness() > 0
                        && damage_amount
                            .map(|dmg| i32::from(c.current_toughness()) <= dmg)
                            .unwrap_or(true)
                } else {
                    false
                }
            })
            .collect();

        if opponent_creature_ids.is_empty() {
            return None;
        }

        // TODO(mtg-77): Implement more filtering from DestroyAi.java:
        // - Filter out creatures with shield counters (line 162)
        // - Filter out creatures that can be sacrificed in response (lines 165-186)
        // - Filter out creatures with regeneration shields (lines 191-194)
        // Note: useRemovalNow() timing check is now implemented separately

        // Select the best creature (highest evaluation score)
        // Reference: ComputerUtilCard.getBestCreatureAI() (line 224)
        self.get_best_creature(view, &opponent_creature_ids)
    }

    /// Determine if removal should be used NOW or held for a better moment
    ///
    /// Reference: ComputerUtilCard.useRemovalNow() (lines 1062-1278)
    ///
    /// Key logic from Java (simplified for our engine):
    /// 1. Sorcery-speed removal: always use now (limited casting windows)
    /// 2. Non-spell removal (activated abilities): always use now
    /// 3. Interrupt: target is enchanted → two-for-one card advantage
    /// 4. Interrupt: during combat → remove blocker/attacker for tempo
    /// 5. Value threshold: removal cost vs target evaluation score
    /// 6. Phase awareness: prefer opponent's end step for instant removal
    pub(crate) fn use_removal_now(&self, spell: &Card, target_id: CardId, view: &GameStateView) -> bool {
        let current_step = view.current_step();

        // Sorcery-speed removal must be used now (limited windows)
        // Reference: Java useRemovalNow line 1185 (sorcery speed multiplier 2x)
        if spell.is_sorcery() {
            return true;
        }

        // --- Interrupt conditions (always use now) ---

        // Interrupt 1: Target is enchanted → removing it also removes attached auras (two-for-one)
        // Reference: Java useRemovalNow lines 1107-1115
        if self.target_has_auras(target_id, view) {
            return true;
        }

        // Interrupt 2: During our Main1 → remove blocker to enable attack
        // Reference: Java useRemovalNow lines 1070-1086
        if current_step == crate::game::Step::Main1 && view.active_player() == self.player_id {
            // We're about to attack; removing an opponent's creature now enables better attacks
            return true;
        }

        // Interrupt 3: During combat → tactical removal
        // Reference: Java useRemovalNow lines 1089-1104
        let is_combat = matches!(
            current_step,
            crate::game::Step::DeclareAttackers | crate::game::Step::DeclareBlockers | crate::game::Step::CombatDamage
        );
        if is_combat {
            return true;
        }

        // --- Value-based timing for instants outside combat ---

        // At opponent's end step: good time to use instant removal
        // Reference: Java useRemovalNow line 1192 (end-of-turn multiplier 2x)
        if current_step == crate::game::Step::End && view.active_player() != self.player_id {
            return true;
        }

        // Main2: acceptable timing for removal (post-combat cleanup)
        if current_step == crate::game::Step::Main2 {
            return true;
        }

        // Value threshold: if target is high-value, use removal even at suboptimal timing
        // Reference: Java useRemovalNow lines 1226-1260 (threat evaluation)
        if let Some(target) = view.get_card(target_id) {
            if target.is_creature() {
                let target_eval = self.evaluate_creature(view, target_id);
                // High-value creatures (evaluation >= 200) are worth removing immediately
                // This threshold matches Java's 0.8 * cost normalization for typical removal
                if target_eval >= 200 {
                    return true;
                }
            }
        }

        // Default: hold instant removal for a better moment
        false
    }

    /// Check if a permanent has auras attached to it
    ///
    /// Used by use_removal_now() to detect two-for-one opportunities.
    /// Removing an enchanted creature also destroys the attached auras.
    pub(crate) fn target_has_auras(&self, target_id: CardId, view: &GameStateView) -> bool {
        view.battlefield().iter().any(|&bf_id| {
            view.get_card(bf_id)
                .is_some_and(|c| c.definition.cache.is_aura && c.attached_to == Some(target_id))
        })
    }
}
