//! Combat actions: declare_attacker, declare_blocker, assign_combat_damage
//!
//! This module contains the implementation of combat-related actions in MTG:
//! - Declaring attackers (with summoning sickness, defender, vigilance checks)
//! - Declaring blockers (with flying/reach restrictions)
//! - Assigning and dealing combat damage (with first strike, trample, lifelink, deathtouch)
//!
//! ## SMART Damage Assignment
//!
//! When an attacker is blocked by multiple creatures, the engine uses SMART damage
//! assignment to reduce the choice space:
//!
//! 1. If attacker has enough power to kill ALL blockers → auto-assign, no choice needed
//! 2. Otherwise, iteratively ask "assign lethal damage to which blocker first?"
//! 3. After all killable blockers are handled, ask where to put remaining non-lethal damage

use crate::core::{CardId, Keyword, PlayerId, TriggerEvent};
use crate::game::state::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};
use smallvec::SmallVec;
use std::collections::HashMap;

impl GameState {
    /// Declare a creature as an attacker
    ///
    /// This validates that the creature can attack and then:
    /// 1. Adds it to combat state as an attacker
    /// 2. Taps it (unless it has vigilance)
    ///
    /// # MTG Rules
    /// - Creatures must not be tapped to attack
    /// - Creatures with defender cannot attack (CR 702.3)
    /// - Creatures have summoning sickness unless they have haste (CR 302.6)
    /// - Attacking creatures are tapped unless they have vigilance (CR 702.20)
    ///
    /// # Arguments
    /// * `player_id` - The attacking player
    /// * `card_id` - The creature to declare as attacker
    ///
    /// # Errors
    ///
    /// Returns an error if the creature cannot attack (not a creature, wrong controller,
    /// tapped, defender, summoning sickness).
    pub fn declare_attacker(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Validate creature can attack
        let card = self.cards.get(card_id)?;

        // Must be a creature
        if !card.is_creature() {
            return Err(MtgError::InvalidAction("Only creatures can attack".to_string()));
        }

        // Must be controlled by the attacking player
        if card.controller != player_id {
            return Err(MtgError::InvalidAction(
                "Can't attack with opponent's creatures".to_string(),
            ));
        }

        // Must be on battlefield
        if !self.battlefield.contains(card_id) {
            return Err(MtgError::InvalidAction(
                "Creature must be on battlefield to attack".to_string(),
            ));
        }

        // Must not be tapped
        if card.tapped {
            return Err(MtgError::InvalidAction(
                "Creature is tapped and can't attack".to_string(),
            ));
        }

        // Check for defender keyword
        // Creatures with defender can't attack
        if card.has_defender() {
            return Err(MtgError::InvalidAction(
                "Creature with defender can't attack".to_string(),
            ));
        }

        // Check for summoning sickness
        // Creatures can't attack the turn they entered the battlefield unless they have haste
        // Uses has_keyword_with_effects to account for granted haste (e.g., from Spider-Punk)
        if let Some(entered_turn) = card.turn_entered_battlefield {
            if entered_turn == self.turn.turn_number && !self.has_keyword_with_effects(card_id, Keyword::Haste) {
                return Err(MtgError::InvalidAction(
                    "Creature has summoning sickness and can't attack this turn".to_string(),
                ));
            }
        }

        // Get defending player (for 2-player, it's the other player)
        let defending_player = self
            .players
            .iter()
            .find(|p| p.id != player_id)
            .map(|p| p.id)
            .ok_or_else(|| MtgError::InvalidAction("No opponent found".to_string()))?;

        // Declare attacker in combat state
        self.combat.declare_attacker(card_id, defending_player);

        // Tap the creature (unless it has vigilance)
        // Uses has_keyword_with_effects to account for granted vigilance
        let has_vigilance = self.has_keyword_with_effects(card_id, Keyword::Vigilance);
        if !has_vigilance {
            // Use helper that handles tap + undo log + mana version
            self.tap_permanent(card_id)?;

            // Check for Taps triggers (MTG Rules 603.2a)
            // "Whenever this creature becomes tapped" triggers fire when tapped
            self.check_triggers(TriggerEvent::Taps, card_id)?;
        }

        // Check for attack triggers (MTG Rules 508.1m)
        // "Whenever this creature attacks" triggers fire after attackers are declared
        self.check_triggers(TriggerEvent::Attacks, card_id)?;

        Ok(())
    }

    /// Declare a creature as a blocker
    ///
    /// This validates that the blocker can block the specified attackers and then
    /// adds it to combat state.
    ///
    /// # MTG Rules
    /// - Creatures must not be tapped to block
    /// - Flying creatures can only be blocked by creatures with flying or reach (CR 702.9)
    /// - Menace creatures must be blocked by 2+ creatures if blocked at all (CR 702.111)
    /// - Normally a creature can only block one attacker (CR 509.1b)
    ///
    /// # Arguments
    /// * `player_id` - The defending player
    /// * `blocker_id` - The creature to declare as blocker
    /// * `attackers` - The attackers to block (usually 1, unless blocker has special ability)
    ///
    /// # Errors
    ///
    /// Returns an error if the creature cannot block (not a creature, wrong controller,
    /// tapped, cannot block due to flying/menace restrictions).
    pub fn declare_blocker(&mut self, player_id: PlayerId, blocker_id: CardId, attackers: Vec<CardId>) -> Result<()> {
        // Validate blocker can block
        let blocker = self.cards.get(blocker_id)?;

        // Must be a creature
        if !blocker.is_creature() {
            return Err(MtgError::InvalidAction("Only creatures can block".to_string()));
        }

        // Must be controlled by the defending player
        if blocker.controller != player_id {
            return Err(MtgError::InvalidAction(
                "Can't block with opponent's creatures".to_string(),
            ));
        }

        // Must be on battlefield
        if !self.battlefield.contains(blocker_id) {
            return Err(MtgError::InvalidAction(
                "Creature must be on battlefield to block".to_string(),
            ));
        }

        // Must not be tapped
        if blocker.tapped {
            return Err(MtgError::InvalidAction(
                "Creature is tapped and can't block".to_string(),
            ));
        }

        // Validate all attackers are actually attacking
        for &attacker in &attackers {
            if !self.combat.is_attacking(attacker) {
                return Err(MtgError::InvalidAction(format!("Card {attacker:?} is not attacking")));
            }
        }

        // Check Flying/Reach restrictions (MTG rule 702.9)
        // A creature with Flying can only be blocked by creatures with Flying or Reach
        // Uses has_keyword_with_effects to account for granted flying/reach
        let blocker_has_flying = self.has_keyword_with_effects(blocker_id, Keyword::Flying);
        let blocker_has_reach = self.has_keyword_with_effects(blocker_id, Keyword::Reach);

        for &attacker_id in &attackers {
            // Check for "can't be blocked" effects (from Deserter's Disciple, etc.)
            // These are tracked in PersistentEffectStore
            if self.persistent_effects.is_creature_unblockable(attacker_id) {
                return Err(MtgError::InvalidAction(
                    "Creature can't be blocked this turn".to_string(),
                ));
            }

            let attacker_has_flying = self.has_keyword_with_effects(attacker_id, Keyword::Flying);

            if attacker_has_flying && !blocker_has_flying && !blocker_has_reach {
                return Err(MtgError::InvalidAction(
                    "Creature cannot block attacker with flying (needs flying or reach)".to_string(),
                ));
            }

            // Note: Menace validation (MTG rule 702.111b) would require checking that creatures
            // with Menace have 0 or 2+ blockers, but this can only be validated after all
            // blockers are declared. Controllers should be smart enough not to block a Menace
            // creature with exactly 1 blocker. Incremental validation during blocker declaration
            // would reject the first blocker, which is incorrect.
        }

        // MTG rule: normally a creature can only block one attacker
        // (unless it has an ability that allows it to block multiple)
        if attackers.len() > 1 {
            // TODO: Check for abilities that allow blocking multiple
            return Err(MtgError::InvalidAction(
                "Creature can only block one attacker".to_string(),
            ));
        }

        // Declare blocker
        let mut attackers_vec = smallvec::SmallVec::new();
        for &attacker in &attackers {
            attackers_vec.push(attacker);
        }
        self.combat.declare_blocker(blocker_id, attackers_vec);

        Ok(())
    }

    /// Calculate lethal damage needed for a blocker
    ///
    /// Returns the amount of damage needed to kill the blocker, accounting for:
    /// - Deathtouch (1 damage is lethal if toughness > 0)
    /// - Indestructible (returns None - cannot be killed)
    /// - Effective toughness (including buffs)
    fn calculate_lethal_damage(&self, blocker_id: CardId, attacker_has_deathtouch: bool) -> Option<i32> {
        let blocker = self.cards.get(blocker_id).ok()?;

        // Indestructible creatures can't be killed by damage
        if self.has_keyword_with_effects(blocker_id, Keyword::Indestructible) {
            return None;
        }

        // Get effective toughness (includes buffs)
        let toughness = self
            .get_effective_toughness(blocker_id)
            .unwrap_or_else(|_| i32::from(blocker.current_toughness()));

        if toughness <= 0 {
            return None; // Already dead or can't be killed
        }

        // Deathtouch: 1 damage is lethal
        if attacker_has_deathtouch {
            Some(1)
        } else {
            Some(toughness)
        }
    }

    /// SMART damage assignment for multiple blockers
    ///
    /// Returns an ordered list of (blocker_id, damage_to_assign) pairs.
    /// Uses the SMART algorithm to reduce choice space:
    /// 1. If can kill all blockers → auto-assign in order
    /// 2. Otherwise, iteratively ask which blocker to kill first
    /// 3. Finally, assign remaining damage to any blocker
    fn smart_damage_assignment(
        &mut self,
        attacker_id: CardId,
        blockers: &[CardId],
        controller: &mut dyn crate::game::controller::PlayerController,
    ) -> Result<SmallVec<[(CardId, i32); 4]>> {
        use crate::game::controller::{ChoiceResult, GameStateView};

        let attacker = self.cards.get(attacker_id)?;
        let attacker_owner = attacker.owner;
        let attacker_has_deathtouch = self.has_keyword_with_effects(attacker_id, Keyword::Deathtouch);
        let has_trample = self.has_keyword_with_effects(attacker_id, Keyword::Trample);

        // Get attacker's effective power
        let total_power = self
            .get_effective_power(attacker_id)
            .unwrap_or_else(|_| i32::from(attacker.current_power()));

        // Calculate lethal damage for each blocker
        let mut blocker_info: Vec<(CardId, Option<i32>)> = blockers
            .iter()
            .map(|&id| (id, self.calculate_lethal_damage(id, attacker_has_deathtouch)))
            .collect();

        // Calculate total lethal needed for all killable blockers
        let total_lethal_needed: i32 = blocker_info.iter().filter_map(|(_, lethal)| *lethal).sum();

        let mut result: SmallVec<[(CardId, i32); 4]> = SmallVec::new();
        let mut remaining_power = total_power;

        // Case 1: Can kill ALL blockers - no choice needed, auto-assign
        if total_power >= total_lethal_needed {
            // Sort by lethal damage (smallest first for efficiency) - all will be killed anyway
            // Use CardId as secondary key for deterministic ordering
            blocker_info.sort_by_key(|(id, lethal)| (lethal.unwrap_or(i32::MAX), *id));

            for (blocker_id, lethal) in &blocker_info {
                if let Some(lethal_dmg) = lethal {
                    let damage = remaining_power.min(*lethal_dmg);
                    result.push((*blocker_id, damage));
                    remaining_power -= damage;
                }
            }

            // If trample, remaining damage goes to player (handled later)
            // Otherwise, dump remaining on last blocker
            if !has_trample && remaining_power > 0 {
                if let Some((_, ref mut damage)) = result.last_mut() {
                    *damage += remaining_power;
                }
            }

            return Ok(result);
        }

        // Case 2: Cannot kill all blockers - use iterative choice
        // Separate into killable (with enough power) and unkillable
        let mut remaining_blockers: Vec<(CardId, Option<i32>)> = blocker_info;

        while remaining_power > 0 {
            // Find blockers we CAN kill with remaining power
            let killable: Vec<(CardId, i32)> = remaining_blockers
                .iter()
                .filter_map(|(id, lethal)| {
                    lethal.and_then(|l| if l <= remaining_power { Some((*id, l)) } else { None })
                })
                .collect();

            if killable.is_empty() {
                // No more blockers can be killed - assign remaining damage
                let alive_blockers: Vec<CardId> = remaining_blockers.iter().map(|(id, _)| *id).collect();

                if !alive_blockers.is_empty() && remaining_power > 0 && !has_trample {
                    // Ask where to put remaining non-lethal damage
                    let view = GameStateView::new(self, attacker_owner);
                    let choice = controller.choose_blocker_for_remaining_damage(
                        &view,
                        attacker_id,
                        &alive_blockers,
                        remaining_power,
                    );

                    match choice {
                        ChoiceResult::Ok(blocker_id) => {
                            result.push((blocker_id, remaining_power));
                            break; // All remaining damage assigned
                        }
                        ChoiceResult::UndoRequest(n) => {
                            self.handle_undo_request(attacker_owner, n)?;
                            return Ok(SmallVec::new()); // Will retry
                        }
                        ChoiceResult::ExitGame => {
                            return Err(MtgError::InvalidAction("Game exit requested".to_string()));
                        }
                        ChoiceResult::Error(msg) => {
                            return Err(MtgError::InvalidAction(format!("Controller error: {}", msg)));
                        }
                        ChoiceResult::NeedInput(_) => {
                            return Err(MtgError::InvalidAction(
                                "NeedInput returned in synchronous game loop".to_string(),
                            ));
                        }
                    }
                }
                break;
            }

            // Only 1 killable blocker - auto-select it
            if killable.len() == 1 {
                let (blocker_id, lethal) = killable[0];
                result.push((blocker_id, lethal));
                remaining_power -= lethal;
                remaining_blockers.retain(|(id, _)| *id != blocker_id);
                continue;
            }

            // Multiple killable blockers - ask the player which to kill first
            let view = GameStateView::new(self, attacker_owner);
            let choice = controller.choose_blocker_for_lethal_damage(&view, attacker_id, &killable, remaining_power);

            match choice {
                ChoiceResult::Ok(blocker_id) => {
                    // Find the lethal damage for chosen blocker
                    if let Some((_, lethal)) = killable.iter().find(|(id, _)| *id == blocker_id) {
                        result.push((blocker_id, *lethal));
                        remaining_power -= lethal;
                        remaining_blockers.retain(|(id, _)| *id != blocker_id);
                    }
                }
                ChoiceResult::UndoRequest(n) => {
                    self.handle_undo_request(attacker_owner, n)?;
                    return Ok(SmallVec::new()); // Will retry
                }
                ChoiceResult::ExitGame => {
                    return Err(MtgError::InvalidAction("Game exit requested".to_string()));
                }
                ChoiceResult::Error(msg) => {
                    return Err(MtgError::InvalidAction(format!("Controller error: {}", msg)));
                }
                ChoiceResult::NeedInput(_) => {
                    return Err(MtgError::InvalidAction(
                        "NeedInput returned in synchronous game loop".to_string(),
                    ));
                }
            }
        }

        Ok(result)
    }

    /// Helper to handle undo requests during damage assignment
    fn handle_undo_request(&mut self, player_id: PlayerId, n: usize) -> Result<()> {
        if n == usize::MAX {
            if let Ok(Some((_actions_undone, choice_log_size))) = self.undo_to_previous_choice_point(player_id) {
                self.logger.truncate_to(choice_log_size);
            }
        } else {
            for _ in 0..n {
                if let Ok(Some(prior_log_size)) = self.undo() {
                    self.logger.truncate_to(prior_log_size);
                }
            }
        }
        Ok(())
    }

    /// Assign and deal combat damage
    ///
    /// This method handles the combat damage step using SMART damage assignment.
    /// For each attacker:
    /// - If unblocked, damage goes to defending player
    /// - If blocked by multiple creatures, uses SMART assignment to minimize choices
    /// - Damage is assigned in order, with lethal damage assigned to each blocker before the next
    ///
    /// ## SMART Damage Assignment
    /// - If attacker can kill ALL blockers → auto-assign, no choice needed
    /// - Otherwise, iteratively ask "which blocker to assign lethal damage to first?"
    /// - Finally, ask where to put remaining non-lethal damage (unless trample)
    ///
    /// MTG Rules 510.1: Combat damage is assigned and dealt simultaneously.
    /// MTG Rules 510.4: If any creature has first strike or double strike, there are two
    /// combat damage steps.
    ///
    /// # Arguments
    /// * `attacker_controller` - Controller for the attacking player
    /// * `blocker_controller` - Controller for the defending player
    /// * `first_strike_step` - True for first strike damage step, false for normal damage step
    ///
    /// # Errors
    ///
    /// Returns an error if damage assignment or application fails.
    pub fn assign_combat_damage(
        &mut self,
        attacker_controller: &mut dyn crate::game::controller::PlayerController,
        blocker_controller: &mut dyn crate::game::controller::PlayerController,
        first_strike_step: bool,
    ) -> Result<()> {
        // First pass: collect SMART damage assignments for attackers with multiple blockers
        let mut damage_assignments: HashMap<CardId, SmallVec<[(CardId, i32); 4]>> = HashMap::new();

        // Collect attackers to avoid borrow conflict
        let attackers: SmallVec<[CardId; 8]> = self.combat.attackers_iter().collect();
        for attacker_id in attackers {
            if self.combat.is_blocked(attacker_id) {
                let blockers = self.combat.get_blockers(attacker_id);

                // For multiple blockers, use SMART damage assignment
                if blockers.len() > 1 {
                    let attacker = self.cards.get(attacker_id)?;
                    let attacker_owner = attacker.owner;

                    // Get the appropriate controller
                    let controller: &mut dyn crate::game::controller::PlayerController =
                        if attacker_owner == attacker_controller.player_id() {
                            attacker_controller
                        } else {
                            blocker_controller
                        };

                    // Use SMART assignment
                    let assignment = self.smart_damage_assignment(attacker_id, &blockers, controller)?;
                    if !assignment.is_empty() {
                        damage_assignments.insert(attacker_id, assignment);
                    }
                }
            }
        }

        // Second pass: assign all damage
        let mut damage_to_creatures: HashMap<CardId, i32> = HashMap::new();
        let mut damage_to_players: HashMap<PlayerId, i32> = HashMap::new();
        // Track damage dealt by each creature for lifelink (creature_id -> total damage dealt)
        let mut damage_dealt_by_creature: HashMap<CardId, i32> = HashMap::new();
        // Track creatures that dealt combat damage to players (for DealsCombatDamage triggers)
        // Maps creature_id -> (target_player_id, damage_amount)
        let mut creatures_that_dealt_player_damage: Vec<(CardId, PlayerId, i32)> = Vec::new();
        // Track creatures dealt deathtouch damage (for state-based destruction)
        let mut deathtouch_damaged_creatures: std::collections::HashSet<CardId> = std::collections::HashSet::new();

        // Use iterator again for second pass (zero allocation)
        for attacker_id in self.combat.attackers_iter() {
            // Skip creatures that are no longer on the battlefield
            // (e.g., died in first strike damage step)
            if !self.battlefield.contains(attacker_id) {
                continue;
            }

            let attacker = self.cards.get(attacker_id)?;

            // Check if this creature deals damage in this step
            // First strike step: only first strike or double strike creatures
            // Normal step: only creatures without first strike, plus double strike creatures
            // Uses has_keyword_with_effects to account for granted keywords
            let has_first_strike = self.has_keyword_with_effects(attacker_id, Keyword::FirstStrike);
            let has_double_strike = self.has_keyword_with_effects(attacker_id, Keyword::DoubleStrike);
            let deals_damage_this_step = if first_strike_step {
                has_first_strike || has_double_strike
            } else {
                has_double_strike || !has_first_strike // has_normal_strike logic
            };

            if !deals_damage_this_step {
                continue; // This creature doesn't deal damage in this step
            }

            // Use effective power (includes Equipment buffs)
            let mut remaining_power = self
                .get_effective_power(attacker_id)
                .unwrap_or_else(|_| i32::from(attacker.current_power()));

            if remaining_power <= 0 {
                continue; // 0 or negative power deals no damage
            }

            // Check if attacker is blocked
            if self.combat.is_blocked(attacker_id) {
                // Attacker deals damage to blockers
                let blockers = self.combat.get_blockers(attacker_id);
                let has_trample = self.has_keyword_with_effects(attacker_id, Keyword::Trample);
                let has_deathtouch = self.has_keyword_with_effects(attacker_id, Keyword::Deathtouch);

                // Check if we have SMART damage assignment for this attacker
                if let Some(assignments) = damage_assignments.get(&attacker_id) {
                    // Use explicit damage assignments from SMART algorithm
                    for (blocker_id, damage) in assignments {
                        if *damage > 0 {
                            *damage_to_creatures.entry(*blocker_id).or_insert(0) += damage;
                            *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += damage;
                            if has_deathtouch {
                                deathtouch_damaged_creatures.insert(*blocker_id);
                            }
                            remaining_power -= damage;
                        }
                    }

                    // Trample: remaining damage goes to defending player
                    if has_trample && remaining_power > 0 {
                        if let Some(defending_player) = self.combat.get_defending_player(attacker_id) {
                            *damage_to_players.entry(defending_player).or_insert(0) += remaining_power;
                            *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += remaining_power;
                            // Track for DealsCombatDamage triggers
                            creatures_that_dealt_player_damage.push((attacker_id, defending_player, remaining_power));
                        }
                    }
                } else {
                    // Single blocker - use original simple logic (no SMART assignment needed)
                    for blocker_id in &blockers {
                        if remaining_power <= 0 {
                            break;
                        }

                        let blocker = self.cards.get(*blocker_id)?;
                        let blocker_toughness = blocker.current_toughness();

                        // Lethal damage calculation
                        let lethal_damage = if has_deathtouch && blocker_toughness > 0 {
                            1
                        } else {
                            blocker_toughness
                        };

                        let damage_to_assign = if blockers.len() == 1 && !has_trample {
                            // Single blocker without trample: assign ALL damage
                            remaining_power
                        } else {
                            remaining_power.min(i32::from(lethal_damage))
                        };

                        if damage_to_assign > 0 {
                            *damage_to_creatures.entry(*blocker_id).or_insert(0) += damage_to_assign;
                            *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += damage_to_assign;
                            if has_deathtouch {
                                deathtouch_damaged_creatures.insert(*blocker_id);
                            }
                            remaining_power -= damage_to_assign;
                        }
                    }

                    // Trample: remaining damage to defending player
                    if has_trample && remaining_power > 0 {
                        if let Some(defending_player) = self.combat.get_defending_player(attacker_id) {
                            *damage_to_players.entry(defending_player).or_insert(0) += remaining_power;
                            *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += remaining_power;
                            // Track for DealsCombatDamage triggers
                            creatures_that_dealt_player_damage.push((attacker_id, defending_player, remaining_power));
                        }
                    }
                }

                // All blockers deal their damage back to attacker (simultaneously)
                // But only if they deal damage in this step (same rules as attackers)
                for blocker_id in &blockers {
                    // Skip blockers that are no longer on the battlefield
                    if !self.battlefield.contains(*blocker_id) {
                        continue;
                    }

                    let blocker = self.cards.get(*blocker_id)?;

                    // Check if blocker deals damage in this step
                    // Uses has_keyword_with_effects to account for granted keywords
                    let blocker_has_first_strike = self.has_keyword_with_effects(*blocker_id, Keyword::FirstStrike);
                    let blocker_has_double_strike = self.has_keyword_with_effects(*blocker_id, Keyword::DoubleStrike);
                    let blocker_deals_damage = if first_strike_step {
                        blocker_has_first_strike || blocker_has_double_strike
                    } else {
                        blocker_has_double_strike || !blocker_has_first_strike
                    };

                    if !blocker_deals_damage {
                        continue;
                    }

                    // Use effective power (includes Equipment buffs)
                    let blocker_power = self
                        .get_effective_power(*blocker_id)
                        .unwrap_or_else(|_| i32::from(blocker.current_power()));
                    if blocker_power > 0 {
                        *damage_to_creatures.entry(attacker_id).or_insert(0) += blocker_power;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(*blocker_id).or_insert(0) += blocker_power;
                        // Track deathtouch damage from blocker (MTG Rules 702.2b)
                        // Uses has_keyword_with_effects to account for granted deathtouch
                        if self.has_keyword_with_effects(*blocker_id, Keyword::Deathtouch) {
                            deathtouch_damaged_creatures.insert(attacker_id);
                        }
                    }
                }
            } else {
                // Unblocked attacker deals damage to defending player
                if let Some(defending_player) = self.combat.get_defending_player(attacker_id) {
                    *damage_to_players.entry(defending_player).or_insert(0) += remaining_power;
                    // Track damage for lifelink
                    *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += remaining_power;
                    // Track for DealsCombatDamage triggers
                    creatures_that_dealt_player_damage.push((attacker_id, defending_player, remaining_power));
                }
            }
        }

        // Apply lifelink BEFORE dealing damage (since creatures might die)
        // MTG Rules 702.15: Damage dealt by a source with lifelink also causes
        // its controller to gain that much life
        // Sort by CardId for deterministic ordering -- HashMap iteration is non-deterministic.
        let mut damage_dealt_sorted: Vec<_> = damage_dealt_by_creature.into_iter().collect();
        damage_dealt_sorted.sort_by_key(|(cid, _)| *cid);
        for (creature_id, total_damage) in &damage_dealt_sorted {
            // Uses has_keyword_with_effects to account for granted lifelink
            if self.has_keyword_with_effects(*creature_id, Keyword::Lifelink) {
                if let Ok(creature) = self.cards.get(*creature_id) {
                    let controller = creature.controller;
                    if let Ok(player) = self.get_player_mut(controller) {
                        player.gain_life(*total_damage);
                    }
                }
            }
        }

        // Deal all damage to players first (they don't die from damage in combat)
        // Sort by PlayerId for deterministic ordering -- HashMap iteration is non-deterministic.
        let mut damage_to_players_sorted: Vec<_> = damage_to_players.into_iter().collect();
        damage_to_players_sorted.sort_by_key(|(pid, _)| *pid);
        for (player_id, damage) in damage_to_players_sorted {
            self.deal_damage(player_id, damage)?;
        }

        // Fire DealsCombatDamage triggers for creatures that dealt combat damage to players
        // MTG Rule 702.18: "Whenever this creature deals combat damage to a player..."
        // Sort by card ID for deterministic trigger ordering
        creatures_that_dealt_player_damage.sort_by_key(|(card_id, _, _)| card_id.as_u32());
        for (creature_id, _target_player, _damage) in creatures_that_dealt_player_damage {
            // Only fire if creature is still on the battlefield
            if self.battlefield.contains(creature_id) {
                self.check_triggers(TriggerEvent::DealsCombatDamage, creature_id)?;
            }
        }

        // Track which creatures should die (MTG Rules 704.5f: State-based actions)
        // Creatures die if:
        // 1. They have lethal damage (damage >= toughness), OR
        // 2. They were dealt any damage by a source with deathtouch
        // MTG Rules 702.12b: Permanents with indestructible can't be destroyed
        let mut creatures_to_destroy = std::collections::HashSet::new();

        // Check creatures for lethal damage
        // Sort by CardId for deterministic ordering -- HashMap iteration is non-deterministic.
        let mut damage_to_creatures_sorted: Vec<_> = damage_to_creatures.into_iter().collect();
        damage_to_creatures_sorted.sort_by_key(|(cid, _)| *cid);
        for (creature_id, damage) in damage_to_creatures_sorted {
            if self.battlefield.contains(creature_id) {
                if let Ok(creature) = self.cards.get(creature_id) {
                    // Uses has_keyword_with_effects to account for granted indestructible
                    if creature.is_creature() && !self.has_keyword_with_effects(creature_id, Keyword::Indestructible) {
                        // Lethal damage: damage >= toughness
                        if damage >= i32::from(creature.current_toughness()) {
                            creatures_to_destroy.insert(creature_id);
                        }
                    }
                }
            }
        }

        // Check creatures for deathtouch damage (MTG Rules 702.2b)
        // Any creature dealt damage by a deathtouch source is destroyed
        for creature_id in deathtouch_damaged_creatures {
            if self.battlefield.contains(creature_id) {
                if let Ok(creature) = self.cards.get(creature_id) {
                    // Only destroy if it's a creature with toughness > 0 and doesn't have indestructible
                    // Uses has_keyword_with_effects to account for granted indestructible
                    if creature.is_creature()
                        && creature.current_toughness() > 0
                        && !self.has_keyword_with_effects(creature_id, Keyword::Indestructible)
                    {
                        creatures_to_destroy.insert(creature_id);
                    }
                }
            }
        }

        // Process dying creatures: check death triggers, then move to graveyard
        // (MTG Rules 704.5f: State-based actions move creatures with lethal damage to graveyard)
        // (MTG Rules 603.6c: Death triggers check the game state as it was just before the creature left)
        // Sort by CardId for deterministic ordering when multiple creatures die simultaneously
        let mut creatures_to_destroy_sorted: Vec<_> = creatures_to_destroy.into_iter().collect();
        creatures_to_destroy_sorted.sort_by_key(|id| id.as_u32());

        for creature_id in creatures_to_destroy_sorted {
            // CR 701.15a: Check regeneration shields before destruction
            let has_regen_shield = self
                .cards
                .get(creature_id)
                .map(|c| c.regeneration_shields > 0)
                .unwrap_or(false);

            if has_regen_shield {
                // Regeneration replaces destruction: tap, clear damage, remove from combat
                self.apply_regeneration_shield(creature_id)?;
                continue;
            }

            // Get creature name before moving to graveyard (for logging)
            let creature_name = self
                .cards
                .get(creature_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|_| "Unknown".into());

            // Check death triggers BEFORE moving the card (trigger still has access to card data)
            // This handles cards like Su-Chi which adds mana when it dies
            let _ = self.check_death_triggers(creature_id);

            // Now move the creature to graveyard
            if let Ok(creature) = self.cards.get(creature_id) {
                let owner = creature.owner;
                self.move_card(creature_id, Zone::Battlefield, Zone::Graveyard, owner)?;

                // Log the death from combat damage (matching format of check_lethal_damage in state.rs)
                self.logger
                    .gamelog(&format!("{} ({}) dies from combat damage", creature_name, creature_id));
            }
        }

        Ok(())
    }
}
