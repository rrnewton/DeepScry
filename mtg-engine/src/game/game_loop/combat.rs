//! Combat phase handler functions
//!
//! This module contains all combat-related step handlers including attacker/blocker declaration
//! and combat damage resolution.

use crate::game::controller::{format_attackers_prompt, format_blockers_prompt, GameStateView, PlayerController};
use crate::{handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

impl<'a> GameLoop<'a> {
    /// Combat phases (simplified for now)
    pub(super) fn begin_combat_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    pub(super) fn declare_attackers_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Active player declares attackers
        let active_player = self.game.turn.active_player;
        let controller: &mut dyn PlayerController = if active_player == controller1.player_id() {
            controller1
        } else {
            controller2
        };

        // Get available creatures that can attack
        let available_creatures = self.get_available_attacker_creatures(active_player);

        if !available_creatures.is_empty() {
            // Clear replay mode if all choices have been replayed
            // This happens BEFORE checking stop conditions, so a snapshot taken here will NOT
            // include the upcoming choice (which hasn't been presented yet)
            //
            // We stay in replay mode until BOTH conditions are met:
            // 1. All intra-turn choices have been replayed (replay_choices_remaining == 0)
            // 2. We've passed the baseline choice count from the snapshot
            //
            // This ensures that automatic actions (like draws) that happen before the first
            // NEW choice point are properly suppressed, avoiding duplicate logging.
            if self.replaying
                && self.replay_choices_remaining == 0
                && (self.choice_counter as usize) >= self.baseline_choice_count
            {
                eprintln!(
                    "🔍 [REPLAY_CLEAR_ATTACKERS] choice_counter={}, baseline={}, CLEARING replay mode",
                    self.choice_counter, self.baseline_choice_count
                );
                self.replaying = false;
                if self.verbosity >= VerbosityLevel::Verbose {
                    eprintln!("✅ REPLAY MODE COMPLETE - will present attacker choice to controller");
                }
            } else if self.replaying {
                eprintln!(
                    "🔍 [REPLAY_STILL_ACTIVE_ATTACKERS] choice_counter={}, baseline={}, remaining={}",
                    self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                );
            }

            // Create view and print prompt BEFORE checking stop conditions
            // so users see what choice was about to be made when using --stop-when-fixed-exhausted
            {
                let view = GameStateView::new(self.game, active_player);
                // Print attacker selection prompt (controlled by show_choice_menu flag)
                if view.logger().should_show_choice_menu() && !available_creatures.is_empty() {
                    print!("{}", format_attackers_prompt(&view, &available_creatures));
                }
            } // Drop view before mutable borrow

            // PREAMBLE: Check stop conditions before asking for choice
            if let Some(result) = self.check_stop_conditions(controller, active_player)? {
                return Ok(Some(result));
            }

            // Ask controller to choose all attackers at once (v2 interface)
            // Capture log size BEFORE asking controller (before controller logs its choice)
            let prior_log_size = self.game.logger.log_count();
            let view = GameStateView::new(self.game, active_player);
            let choice = controller.choose_attackers(&view, &available_creatures);
            let attackers = handle_choice_result_break!(choice, self.game, active_player);

            // Log this choice point for snapshot/replay
            let replay_choice = crate::game::ReplayChoice::Attackers(attackers.clone());
            self.log_choice_point(active_player, Some(replay_choice), prior_log_size);

            // Declare each chosen attacker
            for attacker_id in attackers.iter() {
                // Use GameState::declare_attacker() which taps the creature (MTG Rules 508.1f)
                // NOT Combat::declare_attacker() which only adds to the attackers list
                if let Err(e) = self.game.declare_attacker(active_player, *attacker_id) {
                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                        eprintln!("  Error declaring attacker: {e}");
                    }
                    continue;
                }

                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                    let card_name = self
                        .game
                        .cards
                        .get(*attacker_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");

                    // Get power/toughness for more detail
                    // Use get_effective_power/toughness to include all continuous effects
                    if let Ok(card) = self.game.cards.get(*attacker_id) {
                        let power = self
                            .game
                            .get_effective_power(*attacker_id)
                            .unwrap_or_else(|_| i32::from(card.current_power()));
                        let toughness = self
                            .game
                            .get_effective_toughness(*attacker_id)
                            .unwrap_or_else(|_| i32::from(card.current_toughness()));
                        let message = format!(
                            "{} declares {} ({}) ({}/{}) as attacker",
                            self.get_player_name(active_player),
                            card_name,
                            attacker_id,
                            power,
                            toughness
                        );
                        // Use gamelog for official game action
                        self.game.logger.gamelog(&message);
                    }
                }
                // NOTE: Attack triggers are already checked in declare_attacker() via check_triggers()
                // which now handles optional triggers with sacrifice costs. No need to call
                // check_attack_triggers() here as it would duplicate trigger execution.
            }
        }

        // MTG Rules 508.4: After attackers are declared, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    pub(super) fn declare_blockers_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Defending player declares blockers
        let active_player = self.game.turn.active_player;
        let defending_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have defending player");

        let controller: &mut dyn PlayerController = if defending_player == controller1.player_id() {
            controller1
        } else {
            controller2
        };

        // Get available blockers and attackers
        let available_blockers = self.get_available_blocker_creatures(defending_player);
        let attackers = self.get_current_attackers();

        if !available_blockers.is_empty() && !attackers.is_empty() {
            // Clear replay mode if all choices have been replayed
            // This happens BEFORE checking stop conditions, so a snapshot taken here will NOT
            // include the upcoming choice (which hasn't been presented yet)
            //
            // We stay in replay mode until BOTH conditions are met:
            // 1. All intra-turn choices have been replayed (replay_choices_remaining == 0)
            // 2. We've passed the baseline choice count from the snapshot
            //
            // This ensures that automatic actions (like draws) that happen before the first
            // NEW choice point are properly suppressed, avoiding duplicate logging.
            if self.replaying
                && self.replay_choices_remaining == 0
                && (self.choice_counter as usize) >= self.baseline_choice_count
            {
                eprintln!(
                    "🔍 [REPLAY_CLEAR_BLOCKERS] choice_counter={}, baseline={}, CLEARING replay mode",
                    self.choice_counter, self.baseline_choice_count
                );
                self.replaying = false;
                if self.verbosity >= VerbosityLevel::Verbose {
                    println!("✅ REPLAY MODE COMPLETE - will present blocker choice to controller");
                }
            } else if self.replaying {
                eprintln!(
                    "🔍 [REPLAY_STILL_ACTIVE_BLOCKERS] choice_counter={}, baseline={}, remaining={}",
                    self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                );
            }

            // Create view and print prompt BEFORE checking stop conditions
            // so users see what choice was about to be made when using --stop-when-fixed-exhausted
            {
                let view = GameStateView::new(self.game, defending_player);
                // Print blocker selection prompt (controlled by show_choice_menu flag)
                if view.logger().should_show_choice_menu() {
                    print!("{}", format_blockers_prompt(&view, &available_blockers, &attackers));
                }
            } // Drop view before mutable borrow

            // PREAMBLE: Check stop conditions before asking for choice
            if let Some(result) = self.check_stop_conditions(controller, defending_player)? {
                return Ok(Some(result));
            }

            // Ask controller to choose all blocker assignments at once (v2 interface)
            // Capture log size BEFORE asking controller (before controller logs its choice)
            let prior_log_size = self.game.logger.log_count();
            let view = GameStateView::new(self.game, defending_player);
            let choice = controller.choose_blockers(&view, &available_blockers, &attackers);
            let blocks = handle_choice_result_break!(choice, self.game, defending_player);

            // Log this choice point for snapshot/replay
            let replay_choice = crate::game::ReplayChoice::Blockers(blocks.clone());
            self.log_choice_point(defending_player, Some(replay_choice), prior_log_size);

            // Validate Menace: count blockers per attacker and remove illegal single-blocker assignments
            // MTG Rule 702.111b: A creature with menace can't be blocked except by two or more creatures
            let validated_blocks = self.validate_menace_blockers(&blocks, &attackers)?;

            // Declare each valid blocking assignment
            for (blocker_id, attacker_id) in validated_blocks.iter() {
                let mut attackers_vec = SmallVec::new();
                attackers_vec.push(*attacker_id);
                self.game.combat.declare_blocker(*blocker_id, attackers_vec);

                if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                    let blocker_name = self
                        .game
                        .cards
                        .get(*blocker_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let attacker_name = self
                        .game
                        .cards
                        .get(*attacker_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let message = format!(
                        "{} blocks {} with {}",
                        self.get_player_name(defending_player),
                        attacker_name,
                        blocker_name
                    );
                    // Use gamelog for official game action
                    self.game.logger.gamelog(&message);
                }
            }
        }

        // MTG Rules 509.4: After blockers are declared, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    pub(super) fn combat_damage_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check if any attacking or blocking creature has first strike or double strike
        // MTG Rules 510.4: If so, we have two combat damage steps
        let has_first_strike = self.has_first_strike_combat();

        if has_first_strike {
            // First strike damage step
            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                self.game.logger.normal("--- First Strike Combat Damage ---");
            }
            self.log_combat_damage(true)?;
            self.game.assign_combat_damage(controller1, controller2, true)?;
            if let Some(result) = self.priority_round(controller1, controller2)? {
                return Ok(Some(result));
            }
        }

        // Normal combat damage step (or only step if no first strike)
        if self.verbosity >= VerbosityLevel::Normal && has_first_strike && !self.replaying {
            self.game.logger.normal("--- Normal Combat Damage ---");
        }
        self.log_combat_damage(false)?;
        self.game.assign_combat_damage(controller1, controller2, false)?;

        // After damage is dealt, players get priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Check if any attacking or blocking creature has first strike or double strike
    pub(super) fn has_first_strike_combat(&self) -> bool {
        // Check all attackers (using iterator to avoid Vec allocation)
        for attacker_id in self.game.combat.attackers_iter() {
            if let Ok(attacker) = self.game.cards.get(attacker_id) {
                if attacker.has_first_strike() || attacker.has_double_strike() {
                    return true;
                }
            }

            // Check all blockers of this attacker
            if self.game.combat.is_blocked(attacker_id) {
                let blockers = self.game.combat.get_blockers(attacker_id);
                for blocker_id in &blockers {
                    if let Ok(blocker) = self.game.cards.get(*blocker_id) {
                        if blocker.has_first_strike() || blocker.has_double_strike() {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Log combat damage for debugging
    pub(super) fn log_combat_damage(&self, first_strike_step: bool) -> Result<()> {
        if self.verbosity < VerbosityLevel::Normal || self.replaying {
            return Ok(());
        }

        let mut attackers = self.game.combat.get_attackers();
        // Sort for deterministic logging output
        attackers.sort_by_key(|id| id.as_u32());

        for attacker_id in &attackers {
            // Skip creatures that are no longer on the battlefield
            // MTG Rule 510.1c: Only creatures still on the battlefield deal combat damage
            if !self.game.battlefield.contains(*attacker_id) {
                continue;
            }

            if let Ok(attacker) = self.game.cards.get(*attacker_id) {
                // Check if this attacker deals damage in this step
                let deals_damage = if first_strike_step {
                    attacker.has_first_strike() || attacker.has_double_strike()
                } else {
                    attacker.has_normal_strike()
                };

                if !deals_damage {
                    continue;
                }

                // Use effective power for accurate display (includes anthem/equipment effects)
                let power = self
                    .game
                    .get_effective_power(*attacker_id)
                    .unwrap_or_else(|_| i32::from(attacker.current_power()));
                let attacker_name = &attacker.name;

                if self.game.combat.is_blocked(*attacker_id) {
                    let mut blockers = self.game.combat.get_blockers(*attacker_id);
                    // Sort for deterministic logging output
                    blockers.sort_by_key(|id| id.as_u32());
                    for blocker_id in &blockers {
                        if let Ok(blocker) = self.game.cards.get(*blocker_id) {
                            // Check if blocker deals damage in this step
                            let blocker_deals_damage = if first_strike_step {
                                blocker.has_first_strike() || blocker.has_double_strike()
                            } else {
                                blocker.has_normal_strike()
                            };

                            if !blocker_deals_damage {
                                continue;
                            }

                            let blocker_power = self
                                .game
                                .get_effective_power(*blocker_id)
                                .unwrap_or_else(|_| i32::from(blocker.current_power()));
                            let blocker_name = &blocker.name;
                            let message = format!(
                                "Combat: {attacker_name} ({attacker_id}) ({power} damage) ↔ {blocker_name} ({blocker_id}) ({blocker_power} damage)"
                            );
                            // Use gamelog for official game action
                            self.game.logger.gamelog(&message);
                        }
                    }
                } else {
                    // Unblocked attacker
                    if let Some(defending_player) = self.game.combat.get_defending_player(*attacker_id) {
                        let defender_name = self.get_player_name(defending_player);
                        if power > 0 {
                            let message =
                                format!("{attacker_name} ({attacker_id}) deals {power} damage to {defender_name}");
                            // Use gamelog for official game action
                            self.game.logger.gamelog(&message);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Validate blocker assignments for Menace (MTG Rule 702.111b)
    ///
    /// A creature with menace can't be blocked except by two or more creatures.
    /// This method removes illegal single-blocker assignments from the blocks list.
    ///
    /// Returns a filtered list of valid blocker assignments.
    fn validate_menace_blockers(
        &self,
        blocks: &SmallVec<[(crate::core::CardId, crate::core::CardId); 8]>,
        _attackers: &SmallVec<[crate::core::CardId; 8]>,
    ) -> Result<SmallVec<[(crate::core::CardId, crate::core::CardId); 8]>> {
        use std::collections::HashMap;

        // Count how many blockers each attacker has
        let mut blocker_counts: HashMap<crate::core::CardId, usize> = HashMap::new();
        for (_blocker_id, attacker_id) in blocks.iter() {
            *blocker_counts.entry(*attacker_id).or_insert(0) += 1;
        }

        // Filter out blocks where a Menace creature has exactly 1 blocker
        let mut validated_blocks = SmallVec::new();
        for (blocker_id, attacker_id) in blocks.iter() {
            let count = blocker_counts.get(attacker_id).copied().unwrap_or(0);

            // Check if attacker has Menace
            let has_menace = self
                .game
                .has_keyword_with_effects(*attacker_id, crate::core::Keyword::Menace);

            if has_menace && count == 1 {
                // Menace creature with exactly 1 blocker - invalid, skip this block
                if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                    if let Ok(attacker) = self.game.cards.get(*attacker_id) {
                        if let Ok(blocker) = self.game.cards.get(*blocker_id) {
                            self.game.logger.verbose(&format!(
                                "Menace prevents {} from blocking {} alone (requires 2+ blockers)",
                                blocker.name, attacker.name
                            ));
                        }
                    }
                }
                continue;
            }

            validated_blocks.push((*blocker_id, *attacker_id));
        }

        Ok(validated_blocks)
    }

    pub(super) fn end_combat_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Clear combat state at end of combat
        self.game.combat.clear();

        // Clear combat mana pools (mana from Firebending, etc. lasts until end of combat)
        // Fast path: has_combat_mana() is a single well-predicted branch (usually false)
        for player in &mut self.game.players {
            if player.has_combat_mana() {
                log::debug!(target: "mana", "Clearing combat mana for {}: {:?}",
                    player.name, player.combat_mana_pool);
                player.empty_combat_mana_pool();
            }
        }

        // Players get priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }
}
