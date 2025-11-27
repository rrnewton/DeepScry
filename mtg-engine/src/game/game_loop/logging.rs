//! Logging and display module for GameLoop
//!
//! Handles all formatting and output for game events, battlefield state, and effect execution.

use super::GameLoop;
use super::VerbosityLevel;
use crate::core::{CardId, Keyword, PlayerId};
use crate::game::phase::Step;

impl<'a> GameLoop<'a> {
    /// Get player name for display
    pub(super) fn get_player_name(&self, player_id: PlayerId) -> String {
        self.game
            .get_player(player_id)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|_| {
                // Use 1-based indexing for human-readable player numbers
                let player_num = player_id.as_u32() + 1;
                format!("Player {}", player_num)
            })
    }

    /// Get step name for display
    pub(super) fn step_name(&self, step: Step) -> &'static str {
        match step {
            Step::Untap => "Untap Step",
            Step::Upkeep => "Upkeep Step",
            Step::Draw => "Draw Step",
            Step::Main1 => "Main Phase 1",
            Step::BeginCombat => "Beginning of Combat",
            Step::DeclareAttackers => "Declare Attackers Step",
            Step::DeclareBlockers => "Declare Blockers Step",
            Step::CombatDamage => "Combat Damage Step",
            Step::EndCombat => "End of Combat Step",
            Step::Main2 => "Main Phase 2",
            Step::End => "End Step",
            Step::Cleanup => "Cleanup Step",
        }
    }

    /// Print detailed battlefield state for both players
    pub(super) fn print_battlefield_state(&self) {
        if !self.should_print_to_stdout() {
            return;
        }

        // Print state for each player
        for player in self.game.players.iter() {
            let is_active = player.id == self.game.turn.active_player;
            let marker = if is_active { " (active)" } else { "" };

            println!("{}{}: ", player.name, marker);
            println!("  Life: {}", player.life);

            // Zone sizes
            if let Some(zones) = self.game.get_player_zones(player.id) {
                println!(
                    "  Hand: {} | Library: {} | Graveyard: {} | Exile: {}",
                    zones.hand.len(),
                    zones.library.len(),
                    zones.graveyard.len(),
                    zones.exile.len()
                );

                // Show hand contents for active player (whose turn it is)
                if is_active && !zones.hand.is_empty() {
                    println!("  Hand contents:");
                    for &card_id in &zones.hand.cards {
                        if let Ok(card) = self.game.cards.get(card_id) {
                            println!("    - {} ({})", card.name, card.mana_cost);
                        }
                    }
                }
            }

            // Battlefield permanents controlled by this player
            let mut player_permanents: Vec<_> = self
                .game
                .battlefield
                .cards
                .iter()
                .filter_map(|&card_id| {
                    self.game.cards.get(card_id).ok().and_then(|card| {
                        if card.controller == player.id {
                            Some((card_id, card))
                        } else {
                            None
                        }
                    })
                })
                .collect();

            // Sort by card type for better readability: lands first, then creatures, then others
            player_permanents.sort_by_key(|(_, card)| {
                if card.is_land() {
                    0
                } else if card.is_creature() {
                    1
                } else {
                    2
                }
            });

            if player_permanents.is_empty() {
                println!("  Battlefield: (empty)");
            } else {
                println!("  Battlefield:");
                for (card_id, card) in player_permanents {
                    let tap_status = if card.tapped { " (tapped)" } else { "" };

                    // Check for summoning sickness (creatures that entered this turn and don't have haste)
                    let has_summoning_sickness = if card.is_creature() {
                        if let Some(entered_turn) = card.turn_entered_battlefield {
                            entered_turn == self.game.turn.turn_number && !card.has_keyword(Keyword::Haste)
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    let sickness_status = if has_summoning_sickness {
                        " (summoning sickness)"
                    } else {
                        ""
                    };

                    // Format card display based on type
                    if card.is_creature() {
                        let power = card.current_power() + card.power_bonus as i8;
                        let toughness = card.current_toughness() + card.toughness_bonus as i8;
                        println!(
                            "    {} ({}) - {}/{}{}{}",
                            card.name, card_id, power, toughness, tap_status, sickness_status
                        );
                    } else {
                        println!("    {} ({}){}", card.name, card_id, tap_status);
                    }
                }
            }
        }
        println!();
    }

    /// Print step header lazily (only when first action happens in this step)
    /// Used for Normal verbosity level
    pub(super) fn print_step_header_if_needed(&mut self) {
        if self.verbosity == VerbosityLevel::Normal
            && !self.step_header_printed
            && !self.replaying
            && self.should_print_to_stdout()
        {
            let step = self.game.turn.current_step;
            println!("--- {} ---", self.step_name(step));
            self.step_header_printed = true;
        }
    }

    // === Logging Helpers ===
    // These methods encapsulate lazy header printing + message output

    /// Check if stdout printing is allowed by the logger's output mode
    /// Returns true if OutputMode is Stdout or Both, false if Memory-only
    pub(super) fn should_print_to_stdout(&self) -> bool {
        use crate::game::logger::OutputMode;
        matches!(self.game.logger.output_mode(), OutputMode::Stdout | OutputMode::Both)
    }

    /// Log a message at Normal verbosity level (with lazy step header)
    /// Most game events use this level
    pub(super) fn log_normal(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Normal && !self.replaying && self.should_print_to_stdout() {
            self.print_step_header_if_needed();
            println!("  {message}");
        }
    }

    /// Log a message at Verbose verbosity level (with lazy step header)
    /// Used for detailed action-by-action logging
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    pub(super) fn log_verbose(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Verbose && !self.replaying && self.should_print_to_stdout() {
            self.print_step_header_if_needed();
            println!("  {message}");
        }
    }

    /// Log a message at Minimal verbosity level (no step header needed)
    /// Used for major game events like outcomes
    #[allow(dead_code)]
    pub(super) fn log_minimal(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Minimal && self.should_print_to_stdout() {
            println!("{message}");
        }
    }

    /// Log the execution of a spell effect (damage, draw, etc.)
    pub(super) fn log_effect_execution(
        &self,
        source_name: &str,
        source_id: CardId,
        effect: &crate::core::Effect,
        _source_owner: PlayerId,
    ) {
        use crate::core::{Effect, TargetRef};

        if !self.should_print_to_stdout() {
            return;
        }

        match effect {
            Effect::DealDamage { target, amount } => match target {
                TargetRef::Player(target_player_id) => {
                    let target_name = self.get_player_name(*target_player_id);
                    let new_life = self.game.get_player(*target_player_id).map(|p| p.life).unwrap_or(0);
                    let message = format!(
                        "{source_name} ({source_id}) deals {amount} damage to {target_name} - life: {new_life}"
                    );
                    self.game.logger.normal(&message);
                }
                TargetRef::Permanent(target_card_id) => {
                    let target_name = self
                        .game
                        .cards
                        .get(*target_card_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let message = format!(
                        "{source_name} ({source_id}) deals {amount} damage to {target_name} ({target_card_id})"
                    );
                    self.game.logger.normal(&message);
                }
                TargetRef::None => {
                    // Target will be filled in by resolve_spell - log against opponent
                    if let Some(opponent_id) = self.game.players.iter().map(|p| p.id).find(|id| *id != _source_owner) {
                        let target_name = self.get_player_name(opponent_id);
                        let new_life = self.game.get_player(opponent_id).map(|p| p.life).unwrap_or(0);
                        let message = format!(
                            "{source_name} ({source_id}) deals {amount} damage to {target_name} - life: {new_life}"
                        );
                        self.game.logger.normal(&message);
                    }
                }
            },
            Effect::DrawCards { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to draw {count} card(s)");
                self.game.logger.normal(&message);
            }
            Effect::GainLife { player, amount } => {
                let player_name = self.get_player_name(*player);
                let new_life = self.game.get_player(*player).map(|p| p.life).unwrap_or(0);
                let message = format!(
                    "{source_name} ({source_id}) causes {player_name} to gain {amount} life - life: {new_life}"
                );
                self.game.logger.normal(&message);
            }
            Effect::DestroyPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) destroys {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::TapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) taps {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::UntapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) untaps {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) gives {target_name} ({target}) {power_bonus:+}/{toughness_bonus:+} until end of turn"
                );
                self.game.logger.normal(&message);
            }
            Effect::Mill { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to mill {count} card(s)");
                self.game.logger.normal(&message);
            }
            Effect::CounterSpell { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) counters {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::AddMana { player, mana } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) adds {mana} to {player_name}'s mana pool");
                self.game.logger.normal(&message);
            }
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                println!(
                    "  {source_name} ({source_id}) puts {amount} {counter_type:?} counter(s) on {target_name} ({target})"
                );
            }
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                println!(
                    "  {source_name} ({source_id}) removes {amount} {counter_type:?} counter(s) from {target_name} ({target})"
                );
            }
            Effect::ExilePermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                println!("  {source_name} ({source_id}) exiles {target_name} ({target})");
            }
            Effect::SearchLibrary {
                player,
                card_type_filter,
                destination,
                enters_tapped,
                shuffle: _,
            } => {
                let player_name = self.get_player_name(*player);
                let tapped_text = if *enters_tapped { " tapped" } else { "" };
                println!(
                    "  {source_name} ({source_id}) searches {player_name}'s library for a {card_type_filter} card and puts it into {:?}{tapped_text}",
                    destination
                );
            }
            Effect::AttachEquipment {
                source_equipment,
                target_creature,
            } => {
                let equipment_name = self
                    .game
                    .cards
                    .get(*source_equipment)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let creature_name = self
                    .game
                    .cards
                    .get(*target_creature)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message =
                    format!("{equipment_name} ({source_equipment}) attaches to {creature_name} ({target_creature})");
                self.game.logger.normal(&message);
            }
            Effect::CreateToken {
                controller,
                token_script,
                amount,
            } => {
                let controller_name = self.get_player_name(*controller);
                let message = format!(
                    "{source_name} ({source_id}) creates {amount} {token_script} token(s) under {controller_name}'s control"
                );
                self.game.logger.normal(&message);
            }
        }
    }
}
