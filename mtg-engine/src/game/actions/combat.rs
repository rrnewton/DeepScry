//! Combat actions: declare_attacker, declare_blocker, assign_combat_damage
//!
//! This module contains the implementation of combat-related actions in MTG:
//! - Declaring attackers (with summoning sickness, defender, vigilance checks)
//! - Declaring blockers (with flying/reach restrictions)
//! - Assigning and dealing combat damage (with first strike, trample, lifelink, deathtouch)

use crate::core::{CardId, Keyword, PlayerId};
use crate::game::state::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};
use smallvec::SmallVec;

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
        if let Some(entered_turn) = card.turn_entered_battlefield {
            if entered_turn == self.turn.turn_number && !card.has_keyword(Keyword::Haste) {
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
        let has_vigilance = self.cards.get(card_id)?.has_keyword(Keyword::Vigilance);
        if !has_vigilance {
            // Capture log size before tap
            let prior_log_size = self.logger.log_count();

            let card = self.cards.get_mut(card_id)?;
            card.tap();

            // Log the action
            self.undo_log.log(
                crate::undo::GameAction::TapCard { card_id, tapped: true },
                prior_log_size,
            );
        }

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
        let blocker_has_flying = blocker.has_keyword(Keyword::Flying);
        let blocker_has_reach = blocker.has_keyword(Keyword::Reach);

        for &attacker_id in &attackers {
            let attacker = self.cards.get(attacker_id)?;
            let attacker_has_flying = attacker.has_keyword(Keyword::Flying);

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

    /// Assign and deal combat damage
    ///
    /// This method handles the combat damage step. For each attacker:
    /// - If unblocked, damage goes to defending player
    /// - If blocked by multiple creatures, attacker's controller chooses damage assignment order
    /// - Damage is assigned in order, with lethal damage assigned to each blocker before the next
    ///
    /// MTG Rules 510.1: Combat damage is assigned and dealt simultaneously.
    /// MTG Rules 510.4: If any creature has first strike or double strike, there are two
    /// combat damage steps. Creatures with first strike or double strike deal damage in the
    /// first step, and creatures without first strike (plus double strike creatures) deal
    /// damage in the second step.
    ///
    /// # Arguments
    /// * `attacker_controller` - Controller for the attacking player
    /// * `blocker_controller` - Controller for the defending player
    /// * `first_strike_step` - True for first strike damage step, false for normal damage step
    pub fn assign_combat_damage(
        &mut self,
        attacker_controller: &mut dyn crate::game::controller::PlayerController,
        blocker_controller: &mut dyn crate::game::controller::PlayerController,
        first_strike_step: bool,
    ) -> Result<()> {
        use crate::game::controller::GameStateView;
        use std::collections::HashMap;

        // First pass: collect all damage assignment orders for attackers with multiple blockers
        let mut damage_orders: HashMap<CardId, SmallVec<[CardId; 4]>> = HashMap::new();

        // Collect attackers to avoid borrow conflict with undo in handle_choice_result!
        let attackers: SmallVec<[CardId; 8]> = self.combat.attackers_iter().collect();
        for attacker_id in attackers {
            if self.combat.is_blocked(attacker_id) {
                let blockers = self.combat.get_blockers(attacker_id);

                // If multiple blockers, ask attacker's controller for damage assignment order
                if blockers.len() > 1 {
                    let attacker = self.cards.get(attacker_id)?;
                    let attacker_owner = attacker.owner;

                    // Ask controller for damage assignment order
                    let view = GameStateView::new(self, attacker_owner);
                    let choice = if attacker_owner == attacker_controller.player_id() {
                        attacker_controller.choose_damage_assignment_order(&view, attacker_id, &blockers)
                    } else {
                        blocker_controller.choose_damage_assignment_order(&view, attacker_id, &blockers)
                    };

                    use crate::game::controller::ChoiceResult;
                    let ordered_blockers = match choice {
                        ChoiceResult::Ok(value) => value,
                        ChoiceResult::UndoRequest(n) => {
                            // Perform undo and exit early - game loop will re-execute from rewound state
                            if n == usize::MAX {
                                if let Ok(Some((_actions_undone, choice_log_size))) =
                                    self.undo_to_previous_choice_point(attacker_owner)
                                {
                                    self.logger.truncate_to(choice_log_size);
                                }
                            } else {
                                for _ in 0..n {
                                    if let Ok(Some(prior_log_size)) = self.undo() {
                                        self.logger.truncate_to(prior_log_size);
                                    }
                                }
                            }
                            return Ok(());
                        }
                        ChoiceResult::ExitGame => {
                            return Err(MtgError::InvalidAction("Game exit requested".to_string()));
                        }
                        ChoiceResult::Error(msg) => {
                            return Err(MtgError::InvalidAction(format!("Controller error: {}", msg)));
                        }
                    };

                    damage_orders.insert(attacker_id, ordered_blockers);
                }
            }
        }

        // Second pass: assign all damage
        let mut damage_to_creatures: HashMap<CardId, i32> = HashMap::new();
        let mut damage_to_players: HashMap<PlayerId, i32> = HashMap::new();
        // Track damage dealt by each creature for lifelink (creature_id -> total damage dealt)
        let mut damage_dealt_by_creature: HashMap<CardId, i32> = HashMap::new();
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
            let deals_damage_this_step = if first_strike_step {
                attacker.has_first_strike() || attacker.has_double_strike()
            } else {
                attacker.has_normal_strike()
            };

            if !deals_damage_this_step {
                continue; // This creature doesn't deal damage in this step
            }

            // Use effective power (includes Equipment buffs)
            let mut remaining_power = self
                .get_effective_power(attacker_id)
                .unwrap_or(attacker.current_power() as i32);

            if remaining_power <= 0 {
                continue; // 0 or negative power deals no damage
            }

            // Check if attacker is blocked
            if self.combat.is_blocked(attacker_id) {
                // Attacker deals damage to blockers
                let blockers = self.combat.get_blockers(attacker_id);

                // Use the pre-determined order if we have one, otherwise use default order
                let ordered_blockers = damage_orders.get(&attacker_id).cloned().unwrap_or(blockers);

                // Assign damage in order
                // MTG Rules 510.1c:
                // - If exactly one creature is blocking:
                //   * WITHOUT trample: assign ALL damage to that blocker
                //   * WITH trample: assign at least lethal, rest can trample over
                // - If multiple creatures are blocking: assign at least lethal to each
                //   before assigning to the next (can assign more)
                // Note: Current implementation doesn't track damage, so lethal = toughness
                let has_trample = attacker.has_trample();
                for blocker_id in &ordered_blockers {
                    if remaining_power <= 0 {
                        break;
                    }

                    let blocker = self.cards.get(*blocker_id)?;
                    let blocker_toughness = blocker.current_toughness();

                    // Lethal damage is the creature's toughness
                    // MTG Rules 702.2c: If attacker has deathtouch, any nonzero damage is lethal
                    // (In full MTG, this would be toughness minus damage already marked)
                    let has_deathtouch = attacker.has_deathtouch();
                    let lethal_damage = if has_deathtouch && blocker_toughness > 0 {
                        1 // Any nonzero damage from deathtouch is lethal
                    } else {
                        blocker_toughness
                    };

                    let damage_to_assign = if ordered_blockers.len() == 1 && !has_trample {
                        // MTG Rules 510.1c: With exactly one blocker and NO trample,
                        // assign ALL damage to it (even if more than lethal)
                        remaining_power
                    } else {
                        // MTG Rules 510.1c: With trample OR multiple blockers,
                        // assign at least lethal to each before moving to next.
                        // For simplicity, we assign exactly lethal.
                        remaining_power.min(lethal_damage as i32)
                    };

                    if damage_to_assign > 0 {
                        *damage_to_creatures.entry(*blocker_id).or_insert(0) += damage_to_assign;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += damage_to_assign;
                        // Track deathtouch damage (MTG Rules 702.2b)
                        if has_deathtouch {
                            deathtouch_damaged_creatures.insert(*blocker_id);
                        }
                        remaining_power -= damage_to_assign;
                    }
                }

                // Trample: If attacker has trample and there's remaining damage after
                // assigning lethal to all blockers, assign remaining to defending player
                // MTG Rules 702.19
                if attacker.has_trample() && remaining_power > 0 {
                    if let Some(defending_player) = self.combat.get_defending_player(attacker_id) {
                        *damage_to_players.entry(defending_player).or_insert(0) += remaining_power;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += remaining_power;
                    }
                }

                // All blockers deal their damage back to attacker (simultaneously)
                // But only if they deal damage in this step (same rules as attackers)
                for blocker_id in &ordered_blockers {
                    // Skip blockers that are no longer on the battlefield
                    if !self.battlefield.contains(*blocker_id) {
                        continue;
                    }

                    let blocker = self.cards.get(*blocker_id)?;

                    // Check if blocker deals damage in this step
                    let blocker_deals_damage = if first_strike_step {
                        blocker.has_first_strike() || blocker.has_double_strike()
                    } else {
                        blocker.has_normal_strike()
                    };

                    if !blocker_deals_damage {
                        continue;
                    }

                    // Use effective power (includes Equipment buffs)
                    let blocker_power = self
                        .get_effective_power(*blocker_id)
                        .unwrap_or(blocker.current_power() as i32);
                    if blocker_power > 0 {
                        *damage_to_creatures.entry(attacker_id).or_insert(0) += blocker_power;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(*blocker_id).or_insert(0) += blocker_power;
                        // Track deathtouch damage from blocker (MTG Rules 702.2b)
                        if blocker.has_deathtouch() {
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
                }
            }
        }

        // Apply lifelink BEFORE dealing damage (since creatures might die)
        // MTG Rules 702.15: Damage dealt by a source with lifelink also causes
        // its controller to gain that much life
        for (creature_id, total_damage) in &damage_dealt_by_creature {
            if let Ok(creature) = self.cards.get(*creature_id) {
                if creature.has_lifelink() {
                    let controller = creature.controller;
                    if let Ok(player) = self.get_player_mut(controller) {
                        player.gain_life(*total_damage);
                    }
                }
            }
        }

        // Deal all damage to players first (they don't die from damage in combat)
        for (player_id, damage) in damage_to_players {
            self.deal_damage(player_id, damage)?;
        }

        // Track which creatures should die (MTG Rules 704.5f: State-based actions)
        // Creatures die if:
        // 1. They have lethal damage (damage >= toughness), OR
        // 2. They were dealt any damage by a source with deathtouch
        // MTG Rules 702.12b: Permanents with indestructible can't be destroyed
        let mut creatures_to_destroy = std::collections::HashSet::new();

        // Check creatures for lethal damage
        for (creature_id, damage) in damage_to_creatures {
            if self.battlefield.contains(creature_id) {
                if let Ok(creature) = self.cards.get(creature_id) {
                    if creature.is_creature() && !creature.has_indestructible() {
                        // Lethal damage: damage >= toughness
                        if damage >= creature.current_toughness() as i32 {
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
                    if creature.is_creature() && creature.current_toughness() > 0 && !creature.has_indestructible() {
                        creatures_to_destroy.insert(creature_id);
                    }
                }
            }
        }

        // Move all dying creatures to graveyard (MTG Rules 704.5f)
        for creature_id in creatures_to_destroy {
            if let Ok(creature) = self.cards.get(creature_id) {
                let owner = creature.owner;
                self.move_card(creature_id, Zone::Battlefield, Zone::Graveyard, owner)?;
            }
        }

        Ok(())
    }
}
