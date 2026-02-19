//! Logging and display module for GameLoop
//!
//! Handles all formatting and output for game events, battlefield state, and effect execution.

use super::GameLoop;
use super::VerbosityLevel;
use crate::core::{CardId, PlayerId};
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
    ///
    /// Delegates to the shared display function, showing the active player's hand.
    pub(super) fn print_battlefield_state(&self) {
        if !self.should_print_to_stdout() {
            return;
        }

        // Use shared display function, showing active player's hand (viewer=None)
        crate::game::display::print_battlefield_state(self.game, None);
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

    /// Log an official game action using the logger's gamelog() method
    /// This adds [GAMELOG TurnN STEP] prefix when --tag-gamelogs is enabled
    /// Use for: card draws, mana tapping, spell resolution, ETB, combat damage
    pub(super) fn log_gamelog(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
            self.print_step_header_if_needed();
            self.game.logger.gamelog(message);
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

        // All effect logs use gamelog() for official game action tagging
        match effect {
            Effect::DealDamage { target, amount } => match target {
                TargetRef::Player(target_player_id) => {
                    let target_name = self.get_player_name(*target_player_id);
                    let current_life = self.game.get_player(*target_player_id).map(|p| p.life).unwrap_or(0);
                    let life_after = current_life - *amount;
                    let message = format!(
                        "{source_name} ({source_id}) deals {amount} damage to {target_name} (life: {life_after})"
                    );
                    self.game.logger.gamelog(&message);
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
                    self.game.logger.gamelog(&message);
                }
                TargetRef::None => {
                    // Target will be filled in by resolve_spell - log against opponent
                    if let Some(opponent_id) = self.game.players.iter().map(|p| p.id).find(|id| *id != _source_owner) {
                        let target_name = self.get_player_name(opponent_id);
                        let current_life = self.game.get_player(opponent_id).map(|p| p.life).unwrap_or(0);
                        let life_after = current_life - *amount;
                        let message = format!(
                            "{source_name} ({source_id}) deals {amount} damage to {target_name} (life: {life_after})"
                        );
                        self.game.logger.gamelog(&message);
                    }
                }
            },
            Effect::DrawCards { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to draw {count} card(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::DiscardCards { player, count, .. } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to discard {count} card(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::Loot {
                player,
                discard_count,
                draw_count,
            } => {
                let player_name = self.get_player_name(*player);
                let message = format!(
                    "{source_name} ({source_id}) causes {player_name} to loot (discard {discard_count}, draw {draw_count})"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::GainLife { player, amount } => {
                let player_name = self.get_player_name(*player);
                let old_life = self.game.get_player(*player).map(|p| p.life).unwrap_or(0);
                let new_life = old_life + *amount;
                let message = format!(
                    "{source_name} ({source_id}) causes {player_name} to gain {amount} life - life: {old_life} => {new_life}"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::DestroyPermanent { target, .. } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) destroys {target_name} ({target})");
                self.game.logger.gamelog(&message);
            }
            Effect::TapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) taps {target_name} ({target})");
                self.game.logger.gamelog(&message);
            }
            Effect::UntapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) untaps {target_name} ({target})");
                self.game.logger.gamelog(&message);
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
                keywords_granted,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = if keywords_granted.is_empty() {
                    format!(
                        "{source_name} ({source_id}) gives {target_name} ({target}) {power_bonus:+}/{toughness_bonus:+} until end of turn"
                    )
                } else if *power_bonus == 0 && *toughness_bonus == 0 {
                    format!(
                        "{source_name} ({source_id}) gives {target_name} ({target}) {:?} until end of turn",
                        keywords_granted
                    )
                } else {
                    format!(
                        "{source_name} ({source_id}) gives {target_name} ({target}) {power_bonus:+}/{toughness_bonus:+} and {:?} until end of turn",
                        keywords_granted
                    )
                };
                self.game.logger.gamelog(&message);
            }
            Effect::PumpCreatureVariable {
                target,
                power_count,
                toughness_count,
                keywords_granted,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                // Note: We log the expression type since actual values depend on game state
                let message = format!(
                    "{source_name} ({source_id}) gives {target_name} ({target}) +X/+X (power: {:?}, toughness: {:?}) and {:?} until end of turn",
                    power_count, toughness_count, keywords_granted
                );
                self.game.logger.gamelog(&message);
            }
            Effect::Mill { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to mill {count} card(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::Scry { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to scry {count}");
                self.game.logger.gamelog(&message);
            }
            Effect::CounterSpell { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) counters {target_name} ({target})");
                self.game.logger.gamelog(&message);
            }
            Effect::AddMana { player, mana, .. } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) adds {mana} to {player_name}'s mana pool");
                self.game.logger.gamelog(&message);
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
                let message = format!(
                    "{source_name} ({source_id}) puts {amount} {counter_type:?} counter(s) on {target_name} ({target})"
                );
                self.game.logger.gamelog(&message);
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
                let message = format!(
                    "{source_name} ({source_id}) removes {amount} {counter_type:?} counter(s) from {target_name} ({target})"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::ExilePermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) exiles {target_name} ({target})");
                self.game.logger.gamelog(&message);
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
                let message = format!(
                    "{source_name} ({source_id}) searches {player_name}'s library for a {card_type_filter} card and puts it into {:?}{tapped_text}",
                    destination
                );
                self.game.logger.gamelog(&message);
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
                self.game.logger.gamelog(&message);
            }
            Effect::CreateToken {
                controller,
                token_script,
                amount,
                for_each_player,
            } => {
                if *for_each_player {
                    let message = format!(
                        "{source_name} ({source_id}) causes each player to create {amount} {token_script} token(s)"
                    );
                    self.game.logger.gamelog(&message);
                } else {
                    let controller_name = self.get_player_name(*controller);
                    let message = format!(
                        "{source_name} ({source_id}) creates {amount} {token_script} token(s) under {controller_name}'s control"
                    );
                    self.game.logger.gamelog(&message);
                }
            }
            Effect::Balance {
                card_type,
                zone,
                sub_ability: _,
            } => {
                let type_str = if card_type.is_empty() { "permanents" } else { card_type };
                let zone_str = if zone == "Hand" { "hands" } else { "battlefields" };
                let message = format!(
                    "{source_name} ({source_id}) balances {} across all players' {}",
                    type_str, zone_str
                );
                self.game.logger.gamelog(&message);
            }
            Effect::SetBasePowerToughness {
                target,
                power,
                toughness,
                keywords_granted,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let pt_str = match (power, toughness) {
                    (Some(p), Some(t)) => format!("base P/T to {}/{}", p, t),
                    (Some(p), None) => format!("base power to {}", p),
                    (None, Some(t)) => format!("base toughness to {}", t),
                    (None, None) => String::new(),
                };
                let kw_str = if keywords_granted.is_empty() {
                    String::new()
                } else {
                    let kws: Vec<_> = keywords_granted.iter().map(|k| format!("{:?}", k)).collect();
                    format!(" and gains {}", kws.join(", "))
                };
                let message = if pt_str.is_empty() {
                    format!(
                        "{source_name} ({source_id}) grants {target_name} ({target}){}",
                        kw_str.trim_start_matches(" and ")
                    )
                } else {
                    format!(
                        "{source_name} ({source_id}) sets {target_name} ({target}) {}{}",
                        pt_str, kw_str
                    )
                };
                self.game.logger.gamelog(&message);
            }
            Effect::Airbend { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) airbends {target_name} ({target}) (exiled, may cast for {{2}})"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::Earthbend { target, num_counters } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) earthbends {target_name} ({target}) (0/0 creature with haste, {} +1/+1 counters)",
                    num_counters
                );
                self.game.logger.gamelog(&message);
            }
            Effect::GrantCantBeBlocked { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message =
                    format!("{source_name} ({source_id}) makes {target_name} ({target}) unblockable this turn");
                self.game.logger.gamelog(&message);
            }
            Effect::Firebend { controller, amount } => {
                let player_name = self.get_player_name(*controller);
                let message = format!(
                    "{source_name} ({source_id}) triggers Firebending {amount} - {player_name} adds {amount} {{R}} to combat mana"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::ModalChoice {
                modes, num_to_choose, ..
            } => {
                // Modal choice logging happens when the mode is selected, not during effect resolution
                let mode_descriptions: Vec<&str> = modes.iter().map(|m| m.description.as_str()).collect();
                let message = format!(
                    "{source_name} ({source_id}) is a modal spell (choose {}) - modes: {:?}",
                    num_to_choose, mode_descriptions
                );
                self.game.logger.gamelog(&message);
            }
            Effect::CopyPermanent {
                target,
                controller,
                non_legendary,
                set_power,
                set_toughness,
                ref add_types,
                num_copies,
                restriction: _, // Not used for logging
            } => {
                let controller_name = self.get_player_name(*controller);
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());

                let mut mods = Vec::new();
                if *non_legendary {
                    mods.push("non-legendary".to_string());
                }
                if let Some(p) = set_power {
                    mods.push(format!("power={}", p));
                }
                if let Some(t) = set_toughness {
                    mods.push(format!("toughness={}", t));
                }
                if !add_types.is_empty() {
                    mods.push(format!("add types: {}", add_types.join(", ")));
                }

                let mods_desc = if mods.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", mods.join(", "))
                };

                let copies_desc = if *num_copies > 1 {
                    format!("{} copies of ", num_copies)
                } else {
                    String::new()
                };

                let message = format!(
                    "{source_name} ({source_id}) creates {}a token copy of {target_name}{mods_desc} for {controller_name}",
                    copies_desc
                );
                self.game.logger.gamelog(&message);
            }
            Effect::Dig {
                dig_count,
                destination,
                may_play,
                may_play_without_mana_cost,
                ..
            } => {
                let dest_name = match destination {
                    crate::zones::Zone::Exile => "exile",
                    crate::zones::Zone::Graveyard => "graveyard",
                    crate::zones::Zone::Hand => "hand",
                    crate::zones::Zone::Library
                    | crate::zones::Zone::Battlefield
                    | crate::zones::Zone::Stack
                    | crate::zones::Zone::Command => "exile", // Fallback, unlikely to happen
                };
                let may_play_text = if *may_play {
                    if *may_play_without_mana_cost {
                        ", may play without paying mana cost"
                    } else {
                        ", may play"
                    }
                } else {
                    ""
                };
                let message = format!(
                    "{source_name} ({source_id}) digs {} card(s) from opponent's library to {}{}",
                    dig_count, dest_name, may_play_text
                );
                self.game.logger.gamelog(&message);
            }
            Effect::PumpAllCreatures {
                controller,
                filter,
                power_bonus,
                toughness_bonus,
            } => {
                let controller_name = self.get_player_name(*controller);
                let target_desc = if filter.contains("YouCtrl") {
                    format!("{}'s creatures", controller_name)
                } else if filter.contains("OppCtrl") {
                    "opponent's creatures".to_string()
                } else {
                    "all creatures".to_string()
                };
                let message = format!(
                    "{source_name} ({source_id}) pumps {} (+{}/+{} until end of turn)",
                    target_desc, power_bonus, toughness_bonus
                );
                self.game.logger.gamelog(&message);
            }
            Effect::CreateDelayedTrigger {
                tracked_card,
                effect: delayed_effect,
                ..
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*tracked_card)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) creates delayed trigger on {} (effect: {:?})",
                    target_name, delayed_effect
                );
                self.game.logger.gamelog(&message);
            }
            Effect::CopySpellAbility { may_choose_targets } => {
                let message = format!(
                    "{source_name} ({source_id}) copies spell{}",
                    if *may_choose_targets {
                        " (may choose new targets)"
                    } else {
                        ""
                    }
                );
                self.game.logger.gamelog(&message);
            }
            Effect::ImmediateTrigger { condition, .. } => {
                let message = format!(
                    "{source_name} ({source_id}) checks immediate trigger condition: {:?}",
                    condition
                );
                self.game.logger.gamelog(&message);
            }
            Effect::ClearRemembered => {
                let message = format!("{source_name} ({source_id}) clears remembered cards");
                self.game.logger.gamelog(&message);
            }
            Effect::EachDamage { damagers, receiver, .. } => {
                let damager_count = damagers.len();
                let receiver_name = self
                    .game
                    .cards
                    .get(*receiver)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) causes {} creature(s) to deal damage to {} ({receiver})",
                    damager_count, receiver_name
                );
                self.game.logger.gamelog(&message);
            }
            Effect::UnlessCostWrapper {
                inner_effect,
                unless_cost,
            } => {
                // Log the UnlessCost wrapper with inner effect
                let switched = if unless_cost.switched { "if paid" } else { "unless paid" };
                let inner_desc = format!("{:?}", inner_effect.target_category());
                let message = format!("{source_name} ({source_id}) UnlessCost ({switched}): {inner_desc}");
                self.game.logger.gamelog(&message);
            }
        }
    }
}
