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
        // Check for beginning of combat triggers (e.g., Avatar Kyoshi's earthbend trigger)
        use crate::core::TriggerEvent;
        self.check_phase_triggers(TriggerEvent::BeginningOfCombat)?;

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

        // NETARCH N4 (mtg-53okw/mtg-610): the former attackers_declared_turn
        // re-entry guard is gone. The WASM harness now blocks via rewind+replay,
        // and rewind_to_turn_start clears CombatState (undo.rs), so the declare-
        // attackers choice is presented exactly once on the forward pass and re-
        // presented cleanly after a rewind. Proven by the full validate gate
        // (STRICT native-vs-WASM DIVERGED:0 + 3-deck network mirror incl. the
        // All Hallow's Eve mtg-609 scenario + robots42 4/4).

        // Get available creatures that can attack
        let available_creatures = self.get_available_attacker_creatures(active_player);

        // (combat debug logging removed after fixing combat state rewind bug)

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
            let choice = self.choose_attackers_with_hook(controller, active_player, &available_creatures);
            let attackers = handle_choice_result_break!(choice, self.game, active_player);

            // Log this choice point for snapshot/replay
            let replay_choice = crate::game::ReplayChoice::Attackers(attackers.clone());
            self.log_choice_point(active_player, Some(replay_choice), prior_log_size);

            // Declare each chosen attacker
            for attacker_id in attackers.iter() {
                // Pre-check: Skip creatures no longer on battlefield
                // This can happen when a previous attacker's trigger (e.g., Beetle-Headed Merchants)
                // sacrifices another chosen attacker (e.g., Fire Sages) as a cost.
                // MTG Rules allow triggers to modify game state during the declare attackers step.
                if !self.game.battlefield.contains(*attacker_id) {
                    if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                        let card_name = self
                            .game
                            .cards
                            .get(*attacker_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or("Unknown");
                        self.game.logger.verbose(&format!(
                            "Skipping {} ({}) as attacker - no longer on battlefield",
                            card_name, attacker_id
                        ));
                    }
                    continue;
                }

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
                    if let Some(card) = self.game.cards.try_get(*attacker_id) {
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

            // Fire AttackersDeclared triggers (batch triggers that fire once per declare attackers step)
            // Example: "Whenever one or more creatures you control with flying attack"
            // These are different from individual Attacks triggers which fire per-creature
            self.check_attackers_declared_triggers(active_player)?;
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

        // Check if the blocker declaration has already been done for this turn.
        // This flag prevents re-asking the controller for blockers when the game loop resumes
        // after a NeedInput return from priority_round(). Without this guard, re-entering this
        // function would find available blockers again and consume the wrong ChoiceRequest from
        // the server queue, causing the WASM shadow state's action_count to fall behind the server.
        let current_turn = self.game.turn.turn_number;
        let already_declared = self.game.turn.blockers_declared_turn == Some(current_turn);

        // Get available blockers and attackers.
        //
        // We then prune `available_blockers` down to those creatures that can
        // legally block at least one of the current `attackers` (per
        // `combat_rules::can_block`: Flying/Reach, Shadow, Fear/Intimidate,
        // Skulk, Protection, CantBeBlocked, etc).  This is the single
        // generation-time filter that all UIs/controllers consume — so the
        // native TUI, the WASM fancy TUI, and the heuristic AI all agree
        // with `validate_blocking_restrictions` about what's even an option.
        // Without this, the engine would silently drop a "legal-looking"
        // pick the UI offered (e.g. Knowledge Seeker blocking Glider Kids).
        //
        // Per-pair filtering (which attackers a particular blocker may block)
        // is layered on top by each interactive controller via
        // `combat_rules::legal_attackers_for_blocker`.
        let raw_available_blockers = self.get_available_blocker_creatures(defending_player);
        let attackers = self.get_current_attackers();
        let available_blockers: SmallVec<[crate::core::CardId; 8]> = raw_available_blockers
            .iter()
            .copied()
            .filter(|&blocker_id| crate::game::combat_rules::is_useful_blocker(self.game, blocker_id, &attackers))
            .collect();

        if !available_blockers.is_empty() && !attackers.is_empty() && !already_declared {
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
            let choice = self.choose_blockers_with_hook(controller, defending_player, &available_blockers, &attackers);
            let blocks = handle_choice_result_break!(choice, self.game, defending_player);

            // Log this choice point for snapshot/replay
            let replay_choice = crate::game::ReplayChoice::Blockers(blocks.clone());
            self.log_choice_point(defending_player, Some(replay_choice), prior_log_size);

            // Mark blockers as declared so that if priority_round returns NeedInput
            // and the game loop is re-entered, we skip the blocker declaration above.
            self.game.turn.blockers_declared_turn = Some(current_turn);

            // Validate blocking restrictions and remove illegal block assignments:
            // - Flying (MTG 702.9b): Can only be blocked by flying/reach
            // - Menace (MTG 702.111b): Can't be blocked except by 2+ creatures
            // - CantBeBlocked persistent effects
            let validated_blocks = self.validate_blocking_restrictions(&blocks, &attackers)?;

            // Declare each valid blocking assignment
            for (blocker_id, attacker_id) in validated_blocks.iter() {
                let mut attackers_vec = SmallVec::new();
                attackers_vec.push(*attacker_id);
                // Logged so the blocker declaration is reversible by the undo
                // log (mtg-614 hole (b)). The ai_harness now rewinds+replays
                // (mtg-610), so this is reverted on rewind and re-applied on
                // replay rather than double-counted on a no-rewind re-entry.
                self.game.declare_blocker_logged(*blocker_id, attackers_vec);

                // Log blocker declarations at Normal level (same as attacker declarations)
                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
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
                        "{} declares {} ({}) as blocker for {} ({})",
                        self.get_player_name(defending_player),
                        blocker_name,
                        blocker_id,
                        attacker_name,
                        attacker_id
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
        let current_turn = self.game.turn.turn_number;
        let has_first_strike = self.has_first_strike_combat();
        let first_strike_damage_dealt = self.game.turn.combat_first_strike_damage_dealt_turn == Some(current_turn);

        // First strike damage: deal only if applicable and not yet dealt this turn.
        // Guard prevents re-dealing on WASM step_harness re-entry after NeedInput.
        if has_first_strike && !first_strike_damage_dealt {
            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                self.game.logger.normal("--- First Strike Combat Damage ---");
            }
            // Per-direction damage lines ("X (id) deals N damage to Y (id)") and
            // unblocked-attacker player damage lines are emitted from within
            // `assign_combat_damage` itself, so the reader sees the actual damage
            // applied (including SMART multi-blocker assignments) right before any
            // resulting "X dies from combat damage" lines.
            self.game.assign_combat_damage(controller1, controller2, true)?;
            self.game.turn.combat_first_strike_damage_dealt_turn = Some(current_turn);

            // Check for game end before priority (state-based actions)
            // MTG Rule 704.3: Check state-based actions before players receive priority
            // Skip for network clients (defer_game_end_check) - server is authoritative
            if !self.defer_game_end_check {
                if let Some(result) = self.check_win_condition() {
                    return Ok(Some(result));
                }
            }
        }

        // First-strike priority round: run if first-strike damage was dealt (this call or
        // a prior step_harness call) and the priority round has not yet completed.
        // We track completion separately because has_first_strike_combat() can return false
        // on re-entry if first-strike creatures died, which would wrongly skip this round.
        let first_strike_damage_dealt = self.game.turn.combat_first_strike_damage_dealt_turn == Some(current_turn);
        let first_strike_priority_done = self.game.turn.combat_first_strike_priority_done_turn == Some(current_turn);
        if first_strike_damage_dealt && !first_strike_priority_done {
            if let Some(result) = self.priority_round(controller1, controller2)? {
                return Ok(Some(result));
            }
            self.game.turn.combat_first_strike_priority_done_turn = Some(current_turn);
        }

        // Normal combat damage step (or only step if no first strike).
        // Guard prevents re-dealing on WASM step_harness re-entry after NeedInput.
        let normal_damage_dealt = self.game.turn.combat_damage_dealt_turn == Some(current_turn);
        if !normal_damage_dealt {
            if self.verbosity >= VerbosityLevel::Normal && first_strike_damage_dealt && !self.replaying {
                self.game.logger.normal("--- Normal Combat Damage ---");
            }
            // Per-direction damage lines emitted from within assign_combat_damage (see above).
            self.game.assign_combat_damage(controller1, controller2, false)?;
            self.game.turn.combat_damage_dealt_turn = Some(current_turn);

            // Check for game end before priority (state-based actions)
            // MTG Rule 704.3: Check state-based actions before players receive priority
            // Skip for network clients (defer_game_end_check) - server is authoritative
            if !self.defer_game_end_check {
                if let Some(result) = self.check_win_condition() {
                    return Ok(Some(result));
                }
            }
        }

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

    /// Validate blocker assignments for blocking restrictions.
    ///
    /// Per-pair restrictions (Flying, Reach, Horsemanship, Shadow, Fear,
    /// Intimidate, Skulk, Protection, CantBeBlocked, Tapped) are delegated to
    /// the shared [`combat_rules::can_block`] predicate so the GUI choice menu
    /// and this validator agree on what is legal.
    ///
    /// The aggregate Menace check (CR 702.111b — at least two blockers) is
    /// applied here because it depends on the full assignment.
    ///
    /// Returns a filtered list of valid blocker assignments. Illegal entries
    /// are dropped silently in the gamelog (with a Verbose-level note) since
    /// the UI is responsible for not offering them in the first place.
    fn validate_blocking_restrictions(
        &self,
        blocks: &SmallVec<[(crate::core::CardId, crate::core::CardId); 8]>,
        _attackers: &SmallVec<[crate::core::CardId; 8]>,
    ) -> Result<SmallVec<[(crate::core::CardId, crate::core::CardId); 8]>> {
        use crate::game::combat_rules;
        use std::collections::HashMap;

        // Count how many blockers each attacker has (used for Menace).
        let mut blocker_counts: HashMap<crate::core::CardId, usize> = HashMap::new();
        for (_blocker_id, attacker_id) in blocks.iter() {
            *blocker_counts.entry(*attacker_id).or_insert(0) += 1;
        }

        let mut validated_blocks = SmallVec::new();
        for (blocker_id, attacker_id) in blocks.iter() {
            // Per-pair legality (Flying, Reach, Shadow, Fear, etc).
            if !combat_rules::can_block(self.game, *attacker_id, *blocker_id) {
                if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                    if let (Ok(attacker), Ok(blocker)) =
                        (self.game.cards.get(*attacker_id), self.game.cards.get(*blocker_id))
                    {
                        self.game.logger.verbose(&format!(
                            "Illegal block dropped: {} can't block {} (evasion or restriction)",
                            blocker.name, attacker.name,
                        ));
                    }
                }
                continue;
            }

            // Aggregate Menace (CR 702.111b): exactly-one blocker is illegal.
            let has_menace = self
                .game
                .has_keyword_with_effects(*attacker_id, crate::core::Keyword::Menace);
            let count = blocker_counts.get(attacker_id).copied().unwrap_or(0);
            if has_menace && count == 1 {
                if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                    if let (Ok(attacker), Ok(blocker)) =
                        (self.game.cards.get(*attacker_id), self.game.cards.get(*blocker_id))
                    {
                        self.game.logger.verbose(&format!(
                            "Menace prevents {} from blocking {} alone (requires 2+ blockers)",
                            blocker.name, attacker.name
                        ));
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
        // Clear combat state at end of combat, logging the prior CombatState so
        // the clear is reversible by the undo log (mtg-614 hole (b)). Without
        // this, a rewind across the end-of-combat boundary could not restore the
        // attacker/blocker declarations. Safe now that the ai_harness rewinds+
        // replays (mtg-610) rather than re-running the combat step without rewind.
        let prev_combat = self.game.combat.clone();
        self.game.combat.clear();
        let prior_log_size = self.game.logger.log_count();
        self.game.undo_log.log(
            crate::undo::GameAction::ClearCombat {
                prev: Box::new(prev_combat),
            },
            prior_log_size,
        );

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
