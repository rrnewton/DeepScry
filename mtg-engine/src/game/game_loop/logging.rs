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
        // Handle sentinel player IDs (Wheel of Fortune-style "each player" effects)
        // before falling through to GameState::get_player. The post-resolution
        // logger receives the unresolved Effect, which still carries the
        // ALL_PLAYERS / REMEMBERED_PLAYERS sentinel; display them as "each player"
        // / "remembered players" instead of the raw u32::MAX value.
        if player_id.is_all_players() {
            return "each player".to_string();
        }
        if player_id.is_remembered_players() {
            return "remembered players".to_string();
        }
        if player_id.is_target_opponent() {
            return "target opponent".to_string();
        }
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

    /// Returns whether the logger will USE a `gamelog()` line at all — i.e. it
    /// either prints to stdout (native CLI) OR captures to the in-memory buffer
    /// (WASM / replay comparison). This is the correct gate for official
    /// game-action logging (`gamelog()` calls), as opposed to
    /// [`Self::should_print_to_stdout`], which is stdout-only and is the right
    /// gate only for `println!`-style writes.
    ///
    /// Gating effect-execution `gamelog()` lines on `should_print_to_stdout()`
    /// silently dropped every "exiles / destroys / counters / returns-to-hand"
    /// line in WASM's Memory output mode while native (stdout) emitted them,
    /// diverging the two gamelog streams (mtg-ofl2i: native-vs-WASM
    /// determinism). Effect logging must use THIS predicate instead.
    pub(super) fn logger_captures_or_prints(&self) -> bool {
        use crate::game::logger::OutputMode;
        matches!(
            self.game.logger.output_mode(),
            OutputMode::Stdout | OutputMode::Both | OutputMode::Memory
        )
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
    ///
    /// Currently unused at GameLoop level — see `log_gamelog!` macro comment
    /// in `mod.rs`. Kept around for future GameLoop-level gamelog calls.
    #[allow(dead_code)]
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

        // Gate on captures-OR-prints, NOT stdout-only: every line below is a
        // gamelog() call that must be captured in WASM (Memory output mode) as
        // well as printed on native. The old `!should_print_to_stdout()`
        // early-return suppressed all effect-execution gamelog lines (exiles /
        // destroys / counters / ...) in WASM, diverging it from the
        // byte-identical native run (mtg-ofl2i). gamelog() itself short-circuits
        // when neither capture nor stdout is active, so Silent mode stays
        // allocation-free.
        if !self.logger_captures_or_prints() {
            return;
        }

        // All effect logs use gamelog() for official game action tagging
        match effect {
            Effect::DealDamage { target, amount } => match target {
                TargetRef::Player(target_player_id) => {
                    let target_name = self.get_player_name(*target_player_id);
                    // This logging hook runs AFTER resolve_spell has already applied
                    // the damage, so the player's stored life is the post-damage
                    // total. Do NOT subtract `amount` again (that double-counted,
                    // e.g. Lightning Bolt / Chain Lightning at a 20-life player
                    // logged "(life: 14)" instead of 17).
                    let life_after = self.game.get_player(*target_player_id).map(|p| p.life).unwrap_or(0);
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
                    // Target will be filled in by resolve_spell - log against opponent.
                    // Post-resolution: the opponent's stored life is already the
                    // post-damage total (see the Player arm above), so do not
                    // subtract `amount` again.
                    if let Some(opponent_id) = self.game.players.iter().map(|p| p.id).find(|id| *id != _source_owner) {
                        let target_name = self.get_player_name(opponent_id);
                        let life_after = self.game.get_player(opponent_id).map(|p| p.life).unwrap_or(0);
                        let message = format!(
                            "{source_name} ({source_id}) deals {amount} damage to {target_name} (life: {life_after})"
                        );
                        self.game.logger.gamelog(&message);
                    }
                }
            },
            Effect::DealDamageXPaid { target, .. } => {
                // XPaid damage - will be resolved before actual execution
                let message = format!("{source_name} ({source_id}) deals X damage to {:?}", target);
                self.game.logger.gamelog(&message);
            }
            // DivideEvenly$ RoundedDown resolved form (Fireball): one log line per
            // target, dealing amount_each to each — same wording as the concrete
            // DealDamage arm above so divided burn reads naturally in the gamelog.
            Effect::DealDamageDivided { targets, amount_each } => {
                for target in targets {
                    match target {
                        TargetRef::Player(target_player_id) => {
                            let target_name = self.get_player_name(*target_player_id);
                            // Post-resolution: stored life already reflects this
                            // player's divided share (do not subtract again).
                            let life_after = self.game.get_player(*target_player_id).map(|p| p.life).unwrap_or(0);
                            let message = format!(
                                "{source_name} ({source_id}) deals {amount_each} damage to {target_name} (life: {life_after})"
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
                                "{source_name} ({source_id}) deals {amount_each} damage to {target_name} ({target_card_id})"
                            );
                            self.game.logger.gamelog(&message);
                        }
                        TargetRef::None => {}
                    }
                }
            }
            Effect::DealDamageToTriggeredPlayer { .. } => {
                // Phase-trigger damage (Karma, Black Vise) is resolved into a
                // concrete Effect::DealDamage by check_triggers_for_controller and
                // logged there via logger.normal(); it never reaches this
                // spell-resolution effect logger. No-op to keep the match
                // exhaustive.
            }
            Effect::DrawCards { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to draw {count} card(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::DrawCardsXPaid { player } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to draw X card(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::DiscardCards { player, count, .. } => {
                let player_name = self.get_player_name(*player);
                // Mode$ Hand uses count=u8::MAX as a sentinel meaning "entire hand"
                // (Wheel of Fortune: "Each player discards their hand"). Format it
                // textually rather than as the raw 255 value.
                let message = if *count == u8::MAX {
                    format!("{source_name} ({source_id}) causes {player_name} to discard their hand")
                } else {
                    format!("{source_name} ({source_id}) causes {player_name} to discard {count} card(s)")
                };
                self.game.logger.gamelog(&message);
            }
            Effect::DiscardCardsXPaid { player, .. } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to discard X card(s)");
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
            Effect::GainLifeDynamic { .. } => {
                // Intentionally no log here. This pre-resolution logging hook
                // sees the card's stored effect with an UNRESOLVED player /
                // reference (e.g. the `TargetedController` sentinel), so it
                // cannot name the recipient or amount. The GainLifeDynamic
                // execution path (`execute_effect`) emits the precise
                // "<player> gains <N> life" gamelog line after resolving the
                // amount from last-known game state.
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
            Effect::TapOrUntapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) taps or untaps {target_name} ({target})");
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
            Effect::DebuffCreature {
                target,
                keywords_removed,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) removes {:?} from {target_name} ({target})",
                    keywords_removed
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
            Effect::Surveil { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to surveil {count}");
                self.game.logger.gamelog(&message);
            }
            Effect::AddTurn { player, num_turns } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) grants {player_name} {num_turns} extra turn(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::CounterSpell { target, .. } => {
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
                // `Defined$ Remembered` (e.g. All Hallow's Eve's chained
                // PutCounter) — the *static* effect carries the
                // remembered_card sentinel; the actual target is each card
                // currently in `remembered_cards` and is substituted at
                // execution time. Resolve it here so the log message names
                // the real card instead of "Unknown (4294967291)".
                if target.is_remembered_card() {
                    if self.game.remembered_cards.is_empty() {
                        // This logging pass runs after the spell fully resolves,
                        // which for a `RememberChanged$ True` self-exile chain
                        // (All Hallow's Eve) means the trailing DBCleanup already
                        // cleared remembered_cards. The counters actually landed
                        // on the source card itself (now in exile), so name it
                        // rather than emitting a misleading "(none)".
                        let message = format!(
                            "{source_name} ({source_id}) puts {amount} {counter_type:?} counter(s) on {source_name} ({source_id})"
                        );
                        self.game.logger.gamelog(&message);
                    } else {
                        for &cid in &self.game.remembered_cards {
                            let target_name = self.game.cards.get(cid).map(|c| c.name.as_str()).unwrap_or("Unknown");
                            let message = format!(
                                "{source_name} ({source_id}) puts {amount} {counter_type:?} counter(s) on {target_name} ({cid})"
                            );
                            self.game.logger.gamelog(&message);
                        }
                    }
                    return;
                }
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
            Effect::MultiplyCounter {
                target,
                counter_type,
                multiplier,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let ct_desc = counter_type
                    .map(|ct| format!("{ct:?}"))
                    .unwrap_or_else(|| "all".to_string());
                let message = format!(
                    "{source_name} ({source_id}) multiplies {ct_desc} counters on {target_name} ({target}) by {multiplier}"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::PutCounterAll {
                restriction,
                counter_type,
                amount,
            } => {
                let message = format!(
                    "{source_name} ({source_id}) puts {amount} {counter_type:?} counter(s) on all {}",
                    restriction.describe()
                );
                self.game.logger.gamelog(&message);
            }
            Effect::Proliferate => {
                let message = format!("{source_name} ({source_id}) proliferates");
                self.game.logger.gamelog(&message);
            }
            Effect::ChangeZoneAll {
                restriction,
                origins,
                destination,
                shuffle: _,
            } => {
                // Render origins as "Hand+Graveyard" etc. for a readable log.
                let origin_desc = origins.iter().map(|z| format!("{z:?}")).collect::<Vec<_>>().join("+");
                let message = format!(
                    "{source_name} ({source_id}) moves all {} from {origin_desc} to {destination:?}",
                    restriction.describe()
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
            Effect::ExileIfWouldDieThisTurn { target } => {
                // Disintegrate: log the "if it would die this turn, exile it
                // instead" marking only when bound to a real creature (the
                // parent DealDamage may have hit a player, leaving a sentinel).
                if let Some(card) = self.game.cards.try_get(*target) {
                    if card.is_creature() {
                        let message = format!(
                            "{source_name} ({source_id}): {} ({target}) will be exiled if it would die this turn",
                            card.name
                        );
                        self.game.logger.gamelog(&message);
                    }
                }
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
                ..
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
            Effect::Regenerate { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{target_name} ({target}) gains a regeneration shield");
                self.game.logger.gamelog(&message);
            }
            Effect::PreventDamage { target, amount } => {
                let target_desc = match target {
                    TargetRef::Player(pid) => self.get_player_name(*pid),
                    TargetRef::Permanent(cid) => {
                        let name = self.game.cards.get(*cid).map(|c| c.name.as_str()).unwrap_or("Unknown");
                        format!("{} ({})", name, cid)
                    }
                    TargetRef::None => "target".to_string(),
                };
                let message = format!(
                    "{source_name} ({source_id}) prevents the next {} damage to {} this turn",
                    amount, target_desc
                );
                self.game.logger.gamelog(&message);
            }
            Effect::PreventDamageFromSource { .. } => {
                // The shield-installation line is emitted by execute_effect (it
                // needs the resolved source/player names); nothing to add here.
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
            Effect::AnimateAll {
                controller,
                filter,
                power,
                toughness,
                keywords_granted,
            } => {
                let controller_name = self.get_player_name(*controller);
                let target_desc = if filter.contains("YouCtrl") {
                    format!("{}'s permanents", controller_name)
                } else if filter.contains("OppCtrl") {
                    "opponent's permanents".to_string()
                } else {
                    "all permanents".to_string()
                };
                let pt_str = match (power, toughness) {
                    (Some(p), Some(t)) => format!(" become {}/{}", p, t),
                    (Some(p), None) => format!(" become {}/X", p),
                    (None, Some(t)) => format!(" become X/{}", t),
                    (None, None) => String::new(),
                };
                let kw_str = if keywords_granted.is_empty() {
                    String::new()
                } else {
                    let kws: Vec<_> = keywords_granted.iter().map(|k| format!("{:?}", k)).collect();
                    format!(" gain {}", kws.join(", "))
                };
                let message = format!(
                    "{source_name} ({source_id}) animates {}{}{} until end of turn",
                    target_desc, pt_str, kw_str
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
            Effect::CopySpellAbility {
                may_choose_targets,
                defined_source,
                ..
            } => {
                // Only log a copy for the implemented TriggeredSpellAbility path
                // (e.g. Jeong Jeong via a delayed SpellCast trigger). The
                // SubAbility `Defined$ Parent` path (e.g. Chain Lightning) is not
                // yet implemented (mtg-152) and creates no copy, so claiming a
                // copy here would be a misleading sentinel gamelog
                // (compatibility_tracking SKILL §2.2). Suppress it until the copy
                // actually lands on the stack.
                use crate::core::effects::CopySpellSource;
                if matches!(defined_source, CopySpellSource::TriggeredSpellAbility) {
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
            Effect::ChooseColor { player, .. } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) {player_name} chooses a color");
                self.game.logger.gamelog(&message);
            }
            Effect::Clone { .. } => {
                // The detailed "enters as a copy of X" line is logged by the
                // interactive resolution path (priority.rs) once the controller
                // has chosen which permanent to copy. Nothing to pre-log here.
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
            Effect::LoseLife { player, amount } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to lose {amount} life");
                self.game.logger.gamelog(&message);
            }
            Effect::DestroyAll { no_regenerate, .. } => {
                let regen_note = if *no_regenerate { " (can't be regenerated)" } else { "" };
                let message = format!("{source_name} ({source_id}) destroys all matching permanents{regen_note}");
                self.game.logger.gamelog(&message);
            }
            Effect::SacrificeAll { .. } => {
                let message =
                    format!("{source_name} ({source_id}) forces all players to sacrifice matching permanents");
                self.game.logger.gamelog(&message);
            }
            Effect::DamageAll {
                amount, damage_players, ..
            } => {
                let players_note = if *damage_players { " and each player" } else { "" };
                let message = format!(
                    "{source_name} ({source_id}) deals {amount} damage to each matching creature{players_note}"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::ForceSacrifice {
                player,
                sac_type,
                count,
            } => {
                let player_name = self.get_player_name(*player);
                let message =
                    format!("{source_name} ({source_id}) forces {player_name} to sacrifice {count} {sac_type}");
                self.game.logger.gamelog(&message);
            }
            Effect::TapAll { .. } => {
                let message = format!("{source_name} ({source_id}) taps all matching permanents");
                self.game.logger.gamelog(&message);
            }
            Effect::UntapAll { .. } => {
                let message = format!("{source_name} ({source_id}) untaps all matching permanents");
                self.game.logger.gamelog(&message);
            }
            Effect::DrainMana { .. } => {
                // The "loses all unspent mana" line is emitted by execute_effect
                // once the player sentinel is resolved and the actual drained
                // amount is known; nothing to add at the pre-execution stage.
            }
            Effect::SetLife { player, amount } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) sets {player_name}'s life total to {amount}");
                self.game.logger.gamelog(&message);
            }
            Effect::GainControl {
                target,
                new_controller,
                untap,
                ..
            } => {
                // Skip logging if target is still placeholder - execute_effect logs the resolved target
                if target.is_placeholder() {
                    return;
                }
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let controller_name = self.get_player_name(*new_controller);
                let untap_text = if *untap { " (untapped)" } else { "" };
                let message = format!(
                    "{source_name} ({source_id}) gives {controller_name} control of {target_name} ({target}){untap_text}"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::Fight { fighter, target } => {
                // Skip logging if targets are still placeholders - execute_effect logs the resolved fight
                if fighter.is_placeholder() || target.is_placeholder() {
                    return;
                }
                let fighter_name = self
                    .game
                    .cards
                    .get(*fighter)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) causes {fighter_name} ({fighter}) to fight {target_name} ({target})"
                );
                self.game.logger.gamelog(&message);
            }
            Effect::AddPhase { count } => {
                let message = format!("{source_name} ({source_id}) adds {count} extra combat phase(s)");
                self.game.logger.gamelog(&message);
            }
            Effect::Unimplemented { api_type } => {
                let message = format!("{source_name} ({source_id}) has unimplemented effect '{api_type}'");
                self.game.logger.gamelog(&message);
            }
            Effect::SelfExileFromStack { remember_changed, .. } => {
                // Surface the self-exile so users can see e.g. All Hallow's Eve
                // moving from the stack to exile (and being remembered for the
                // chained PutCounter).
                let message = if *remember_changed {
                    format!("{source_name} ({source_id}) is exiled (remembered)")
                } else {
                    format!("{source_name} ({source_id}) is exiled")
                };
                self.game.logger.gamelog(&message);
            }
            Effect::MoveSelfBetweenZones {
                origin, destination, ..
            } => {
                // e.g. All Hallow's Eve moving itself exile→graveyard once its
                // final scream counter is removed.
                let message = format!("{source_name} ({source_id}) moves from {origin:?} to {destination:?}");
                self.game.logger.gamelog(&message);
            }
            Effect::ReturnCardsFromGraveyardToHand { .. } => {
                // Individual card-return log lines are emitted inside execute_effect
                // (one line per card returned). Nothing to surface at the top level.
            }
            Effect::PreventAllCombatDamageThisTurn { .. } => {
                // The combat-damage prevention log line is emitted inside execute_effect
                // ("Prevent all combat damage ... this turn"). Nothing to surface here.
            }
            Effect::ConditionalSelfCounter { .. } => {
                // The wrapper itself produces no log; the inner effect logs when
                // (and if) it executes. Nothing to surface here.
            }
        }
    }
}
