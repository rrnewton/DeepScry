//! Step handler functions for turn progression
//!
//! This module contains the individual step handlers (untap, upkeep, draw, main, end, cleanup)
//! that execute during each turn of the game.

use crate::core::{CardId, Keyword, PlayerId, TriggerEvent};
use crate::game::controller::{format_discard_prompt, ChoiceResult, GameStateView, PlayerController};
use crate::{handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

impl<'a> GameLoop<'a> {
    /// Untap step - untap all permanents controlled by active player
    ///
    /// If permanents have "You may choose not to untap CARDNAME during your untap step"
    /// (MayNotUntap keyword), the controller is asked which permanents to keep tapped.
    pub(super) fn untap_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Reset draw count for the active player at the start of their turn
        // This tracks "cards drawn this turn" for triggers like Knowledge Seeker
        let prior_log_size = self.game.logger.log_count();
        if let Ok(player) = self.game.get_player_mut(active_player) {
            let old_count = player.cards_drawn_this_turn;
            player.reset_cards_drawn();
            // Reset spells cast counter for new turn
            player.spells_cast_this_turn = 0;
            // Log for undo
            self.game.undo_log.log(
                crate::undo::GameAction::SetCardsDrawnThisTurn {
                    player_id: active_player,
                    old_value: old_count,
                    new_value: 0,
                },
                prior_log_size,
            );
        }

        // Stasis: "Players skip their untap steps." A `R:Event$ BeginPhase |
        // Phase$ Untap | Skip$ True` replacement on any battlefield permanent
        // (precomputed into cache.skips_untap_step) means NO permanent untaps
        // this step (CR 614 skip-replacement on the untap step). The lock is
        // symmetric; since this step only ever untaps the active player's
        // permanents, skipping it whenever any such permanent is in play is
        // correct for every player's turn.
        let untap_skipped = self.game.battlefield.cards.iter().any(|&id| {
            self.game
                .cards
                .try_get(id)
                .is_some_and(|c| c.definition.cache.skips_untap_step)
        });
        if untap_skipped {
            if self.verbosity >= VerbosityLevel::Normal {
                self.log_normal("Players skip their untap steps");
            }
            return Ok(None);
        }

        // Winter Orb (CR 502 untap-restriction): while an UNTAPPED permanent
        // with the `UntapAdjust:Land:N` lock is on the battlefield, a player may
        // untap at most N lands during their untap step. The lock is re-derived
        // here from current board state (an untapped lock permanent), so it is a
        // pure function of the battlefield — no per-turn flag, hence rewind-safe.
        // A *tapped* Winter Orb does not lock (the `IsPresent$ Card.Self+untapped`
        // self-condition), which is exactly why it is checked at this site.
        let land_untap_limit: Option<u8> = self
            .game
            .battlefield
            .cards
            .iter()
            .filter_map(|&id| {
                let card = self.game.cards.try_get(id)?;
                if card.tapped {
                    return None;
                }
                card.definition.cache.limits_land_untap
            })
            .min();

        // Collect tapped permanents controlled by active player
        // Separate into: normal permanents and MayNotUntap permanents
        let mut normal_to_untap: SmallVec<[CardId; 8]> = SmallVec::new();
        let mut may_not_untap: SmallVec<[CardId; 8]> = SmallVec::new();

        // Permanents that are forced to stay tapped (CR 302.6 doesn't-untap
        // effects: Paralyze, Exhaustion, ...). The keyword may be printed or,
        // far more commonly, granted to the affected creature by a host Aura's
        // GrantKeyword(DoesNotUntap) static — so consult granted keywords too.
        let mut forced_stay_tapped: SmallVec<[CardId; 8]> = SmallVec::new();

        // Normal-untap tapped LANDS, set aside when the Winter Orb lock is active
        // so the controller picks which (up to N) actually untap.
        let mut limited_lands: SmallVec<[CardId; 8]> = SmallVec::new();

        for &card_id in &self.game.battlefield.cards {
            if let Some(card) = self.game.cards.try_get(card_id) {
                if card.controller == active_player && card.tapped {
                    if self.game.has_keyword_with_effects(card_id, Keyword::DoesNotUntap) {
                        // Forced not to untap — does not even reach the
                        // MayNotUntap optional-choice path. (A doesn't-untap land
                        // can't untap anyway, so it never consumes the Winter Orb
                        // land budget.)
                        forced_stay_tapped.push(card_id);
                    } else if card.keywords.contains(Keyword::MayNotUntap) {
                        may_not_untap.push(card_id);
                    } else if land_untap_limit.is_some() && card.definition.cache.is_land {
                        // Untap-limited land: resolved below via a count-capped
                        // controller choice instead of unconditional untap.
                        limited_lands.push(card_id);
                    } else {
                        normal_to_untap.push(card_id);
                    }
                }
            }
        }

        // Log the forced-tapped permanents (for game-log evidence) but do not
        // untap them.
        if !forced_stay_tapped.is_empty() && self.verbosity >= VerbosityLevel::Normal {
            for &card_id in &forced_stay_tapped {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    let name = card.name.clone();
                    self.log_normal(&format!("{} doesn't untap (locked tapped)", name));
                }
            }
        }

        // Untap all normal permanents
        for card_id in normal_to_untap {
            let _ = self.game.untap_permanent(card_id);
        }

        // If there are MayNotUntap permanents, ask controller which to keep tapped
        if !may_not_untap.is_empty() {
            let controller: &mut dyn PlayerController = if active_player == self.game.players[0].id {
                controller1
            } else {
                controller2
            };

            let choice = self.choose_not_untap_with_hook(controller, active_player, &may_not_untap);

            let stay_tapped: SmallVec<[CardId; 8]> = match choice {
                ChoiceResult::Ok(ids) => ids,
                ChoiceResult::ExitGame => {
                    return Ok(Some(GameResult {
                        winner: self.game.get_other_player_id(active_player),
                        turns_played: self.turns_elapsed,
                        end_reason: super::GameEndReason::Manual,
                        action_count: self.game.action_count(),
                    }));
                }
                ChoiceResult::Error(e) => {
                    log::error!("Error in choose_permanents_to_not_untap: {}", e);
                    SmallVec::new() // Default to untapping everything
                }
                ChoiceResult::UndoRequest(_) => SmallVec::new(), // Can't undo untap step
                ChoiceResult::NeedInput(_) => SmallVec::new(),   // Not supported in untap
            };

            // Untap MayNotUntap permanents that weren't chosen to stay tapped
            for card_id in may_not_untap {
                if !stay_tapped.contains(&card_id) {
                    let _ = self.game.untap_permanent(card_id);
                } else if self.verbosity >= VerbosityLevel::Normal {
                    // Log that this permanent stays tapped
                    if let Some(card) = self.game.cards.try_get(card_id) {
                        let player_name = self.get_player_name(active_player);
                        self.log_normal(&format!("{} chooses not to untap {}", player_name, card.name));
                    }
                }
            }
        }

        // Winter Orb land-untap limit (CR 502 / CR 102.4-style restriction): the
        // active player untaps AT MOST `limit` of their tapped lands. If they
        // hold no more tapped lands than the budget, all untap normally; only
        // when over budget do we solicit a choice.
        if let Some(limit) = land_untap_limit {
            let limit = limit as usize;
            if limited_lands.len() <= limit {
                // Within budget: untap all of them.
                for card_id in limited_lands {
                    let _ = self.game.untap_permanent(card_id);
                }
            } else {
                // Over budget: the controller chooses which lands to keep TAPPED
                // (CR 502.3 — the active player decides which of their permanents
                // untap). Reuse the existing not-untap choice on the land set;
                // the engine then ENFORCES the cap deterministically so the
                // result is controller-agnostic and rewind-safe regardless of how
                // many the controller tried to untap.
                let controller: &mut dyn PlayerController = if active_player == self.game.players[0].id {
                    controller1
                } else {
                    controller2
                };
                let choice = self.choose_not_untap_with_hook(controller, active_player, &limited_lands);
                let chosen_stay_tapped: SmallVec<[CardId; 8]> = match choice {
                    ChoiceResult::Ok(ids) => ids,
                    ChoiceResult::ExitGame => {
                        return Ok(Some(GameResult {
                            winner: self.game.get_other_player_id(active_player),
                            turns_played: self.turns_elapsed,
                            end_reason: super::GameEndReason::Manual,
                            action_count: self.game.action_count(),
                        }));
                    }
                    ChoiceResult::Error(e) => {
                        log::error!("Error in choose_permanents_to_not_untap (land limit): {}", e);
                        SmallVec::new()
                    }
                    ChoiceResult::UndoRequest(_) => SmallVec::new(),
                    ChoiceResult::NeedInput(_) => SmallVec::new(),
                };

                // Lands the controller wants to untap (not chosen to stay tapped),
                // in stable battlefield order.
                let mut untap_candidates: SmallVec<[CardId; 8]> = limited_lands
                    .iter()
                    .copied()
                    .filter(|id| !chosen_stay_tapped.contains(id))
                    .collect();
                // Enforce the cap: at most `limit` lands untap. If the controller
                // selected too many (or made no selection at all), keep the lowest
                // battlefield-order lands and force the rest to stay tapped. This
                // is deterministic and identical on server and client.
                if untap_candidates.len() > limit {
                    untap_candidates.truncate(limit);
                }

                for &card_id in &limited_lands {
                    if untap_candidates.contains(&card_id) {
                        let _ = self.game.untap_permanent(card_id);
                    } else if self.verbosity >= VerbosityLevel::Normal {
                        if let Some(card) = self.game.cards.try_get(card_id) {
                            let player_name = self.get_player_name(active_player);
                            self.log_normal(&format!(
                                "{} can't untap {} (untap limited to {} land{})",
                                player_name,
                                card.name,
                                limit,
                                if limit == 1 { "" } else { "s" }
                            ));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check and execute phase-triggered abilities
    pub(super) fn check_phase_triggers(&mut self, trigger_event: TriggerEvent) -> Result<()> {
        let active_player = self.game.turn.active_player;

        // No WASM re-entry guard needed here (mtg-610): both the human and AI
        // network paths now RESUME via undo-log rewind+replay (fancy_tui.rs
        // run_network_mode_human_v2 / run_network_ai_replay) — a re-entry after a
        // NeedInput block rewinds to the turn start and replays from there, so
        // begin-of-phase triggers fire exactly once per phase from a clean
        // turn-start state instead of being double-fired on a no-rewind re-run.
        // The previous per-turn guard family (upkeep/end-step/draw triggers,
        // draw-step, combat, blockers) papered over the old no-rewind re-run and
        // has been deleted now that the rewind+replay path makes it dead.

        // A phase trigger fires from the battlefield (the overwhelmingly common
        // case) or, for a card whose trigger declares a non-battlefield
        // `TriggerZones$` (e.g. All Hallow's Eve's `TriggerZones$ Exile`), from
        // that zone (CR 603.6e). To keep the battlefield path byte-for-byte
        // identical to its long-standing behaviour — and so the network
        // state-hash determinism is untouched for games without exile-resident
        // triggers — we scan the battlefield exactly as before, then ADD only
        // the specific exile cards whose triggers explicitly opt into Exile.
        let matches_trigger = |card: &crate::core::Card, in_zone: crate::zones::Zone| {
            card.triggers.iter().any(|t| {
                if t.event != trigger_event {
                    return false;
                }
                // Trigger-zone gate: empty `trigger_zones` means battlefield (the
                // historical default); otherwise the current zone must be listed.
                let zone_ok = if t.trigger_zones.is_empty() {
                    in_zone == crate::zones::Zone::Battlefield
                } else {
                    t.trigger_zones.contains(&in_zone)
                };
                if !zone_ok {
                    return false;
                }
                // Check controller_turn_only flag - if set, only fire on controller's turn
                // OPTIMIZATION: Use pre-parsed boolean flag instead of runtime string check
                if t.controller_turn_only && card.controller != active_player {
                    return false;
                }
                // ValidPlayer$ Player.Opponent (Sorin emblem): fire only on opponents'
                // turns — never on the controller's own upkeep. This is the inverse of
                // controller_turn_only: skip if the active player IS the controller.
                if t.opponent_turn_only && card.controller == active_player {
                    return false;
                }
                // ValidPlayer$ Player.Chosen (Black Vise): fire only on the
                // chosen player's turn. If no player has been chosen yet (card
                // not yet ETB'd through the ChoosePlayer replacement), it cannot
                // fire.
                if t.chosen_player_turn_only && card.chosen_player != Some(active_player) {
                    return false;
                }
                // ValidPlayer$ Player.EnchantedController (Paralyze): fire only on
                // the upkeep of the ENCHANTED permanent's controller. If the Aura
                // is not attached to anything, or the attached permanent's
                // controller is not the active player, the trigger does not fire.
                if t.enchanted_controller_turn_only {
                    let fires = card
                        .attached_to
                        .and_then(|host| self.game.cards.try_get(host))
                        .is_some_and(|host| host.controller == active_player);
                    if !fires {
                        return false;
                    }
                }
                // Intervening-if condition (CR 603.4): the source must satisfy the
                // self-state condition right now (All Hallow's Eve: >= 1 scream
                // counter; Howling Mine: the source must be untapped).
                if let Some(cond) = &t.present_self_condition {
                    use crate::core::PresentSelfCondition;
                    let satisfied = match cond {
                        PresentSelfCondition::Counter(c) => c.evaluate(card.get_counter(c.counter_type)),
                        PresentSelfCondition::Untapped => !card.tapped,
                        PresentSelfCondition::Tapped => card.tapped,
                    };
                    if !satisfied {
                        return false;
                    }
                }
                // Intervening-if condition (CR 603.4): Whirling Dervish's end-step
                // counter trigger fires only if the source dealt damage to an
                // opponent this turn. Without this gate the +1/+1 counter would be
                // placed unconditionally (mtg-713 B9 TRAP).
                if t.present_self_dealt_damage_to_opponent && !card.dealt_damage_to_opponent_this_turn {
                    return false;
                }
                true
            })
        };

        // Collect card IDs with matching triggers (avoid String allocation in hot path).
        // Battlefield scan — unchanged from the original implementation.
        let mut triggered_cards: SmallVec<[CardId; 4]> = self
            .game
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.game
                    .cards
                    .try_get(card_id)
                    .is_some_and(|card| matches_trigger(card, crate::zones::Zone::Battlefield))
            })
            .collect();

        // Exile-resident triggers (rare). Only cards whose trigger explicitly
        // opts into Exile via `TriggerZones$` can match here, so games without
        // such cards add nothing and behave exactly as before.
        for (_, zones) in &self.game.player_zones {
            for &card_id in &zones.exile.cards {
                if self
                    .game
                    .cards
                    .try_get(card_id)
                    .is_some_and(|card| matches_trigger(card, crate::zones::Zone::Exile))
                {
                    triggered_cards.push(card_id);
                }
            }
        }

        // Command-zone triggers (emblems). Planeswalker ultimates place emblem
        // objects in the command zone (CR 113.2). Only cards/emblems whose
        // trigger explicitly opts into Command via `TriggerZones$ Command` are
        // scanned here — games without emblems add nothing.
        for (_, zones) in &self.game.player_zones {
            for &card_id in &zones.command.cards {
                if self
                    .game
                    .cards
                    .try_get(card_id)
                    .is_some_and(|card| matches_trigger(card, crate::zones::Zone::Command))
                {
                    triggered_cards.push(card_id);
                }
            }
        }

        // For each card with a matching trigger, log and execute
        for card_id in triggered_cards {
            // Log trigger activation only if verbose (avoid String allocation in hot path)
            if self.verbosity >= VerbosityLevel::Verbose {
                // Collect descriptions separately to avoid borrow conflict with log_verbose
                let descriptions: SmallVec<[String; 2]> = self
                    .game
                    .cards
                    .try_get(card_id)
                    .map(|card| {
                        card.triggers
                            .iter()
                            .filter(|t| t.event == trigger_event)
                            .map(|t| {
                                let desc = t
                                    .description
                                    .strip_prefix("[controller_only] ")
                                    .unwrap_or(&t.description);
                                format!("Trigger: {} - {}", card.name, desc)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for desc in descriptions {
                    self.log_verbose(&desc);
                }
            }

            // Use the existing check_triggers method to execute effects
            self.game
                .check_triggers_for_controller(trigger_event, card_id, active_player)?;
        }

        // Push reveals after phase triggers for network mode (server-side)
        // Triggered abilities can draw cards, and clients need the card IDs
        self.push_reveals(active_player);
        if let Some(opponent) = self.game.get_other_player_id(active_player) {
            self.push_reveals(opponent);
        }

        Ok(())
    }

    /// Check and fire AttackersDeclared triggers (batch triggers that fire once per declare attackers step)
    /// These differ from Attacks triggers which fire per-creature
    /// Example: "Whenever one or more creatures you control with flying attack"
    pub(super) fn check_attackers_declared_triggers(&mut self, attacking_player: PlayerId) -> Result<()> {
        // Get the list of attacking creatures (need to check keyword filters)
        let attackers: SmallVec<[CardId; 8]> = self.game.combat.attackers.keys().copied().collect();

        if attackers.is_empty() {
            return Ok(()); // No attackers, no triggers
        }

        // Collect all permanents with AttackersDeclared triggers that should fire
        let triggered_cards: SmallVec<[(CardId, String); 4]> = self
            .game
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    // Find matching AttackersDeclared triggers
                    for trigger in &card.triggers {
                        if trigger.event != TriggerEvent::AttackersDeclared {
                            continue;
                        }

                        // Check controller_turn_only (AttackingPlayer$ You)
                        if trigger.controller_turn_only && card.controller != attacking_player {
                            continue;
                        }

                        // Check valid_attackers_keyword filter
                        if let Some(required_keyword) = trigger.valid_attackers_keyword {
                            // At least one attacker must have the required keyword
                            let has_matching_attacker = attackers.iter().any(|&attacker_id| {
                                if let Ok(attacker) = self.game.cards.get(attacker_id) {
                                    // Check if attacker is controlled by the triggering player
                                    if attacker.controller != card.controller {
                                        return false;
                                    }
                                    // Check for the required keyword
                                    attacker.keywords.contains(required_keyword)
                                } else {
                                    false
                                }
                            });

                            if !has_matching_attacker {
                                continue;
                            }
                        }

                        // Trigger conditions met!
                        return Some((card_id, trigger.description.clone()));
                    }
                    None
                } else {
                    None
                }
            })
            .collect();

        // Fire each trigger
        for (card_id, description) in triggered_cards {
            if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    let message = format!("Trigger: {} - {}", card.name, description);
                    self.log_verbose(&message);
                }
            }

            // Execute the trigger effects
            if let Some(card) = self.game.cards.try_get(card_id) {
                let controller = card.controller;
                self.game
                    .check_triggers_for_controller(TriggerEvent::AttackersDeclared, card_id, controller)?;
            }
        }

        // Push reveals after triggers
        self.push_reveals(attacking_player);
        if let Some(opponent) = self.game.get_other_player_id(attacking_player) {
            self.push_reveals(opponent);
        }

        Ok(())
    }

    /// Upkeep step - priority round for triggers and actions
    pub(super) fn upkeep_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check for beginning of upkeep triggers
        self.check_phase_triggers(TriggerEvent::BeginningOfUpkeep)?;

        // Pass priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Draw step - active player draws a card
    pub(super) fn draw_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Skip draw on first turn (player going first doesn't draw)
        if self.game.turn.turn_number == 1 {
            // Still print battlefield state even on turn 1 (no draw)
            if !self.replaying && self.verbosity >= VerbosityLevel::Normal {
                self.print_battlefield_state();
            }
            self.log_normal("(First turn - no draw)");
            return Ok(None);
        }

        // No re-entry guard needed (mtg-610): a WASM network re-entry rewinds to
        // the turn start and replays, so the mandatory draw runs exactly once per
        // turn from a clean turn-start state rather than being re-executed on a
        // no-rewind re-run. The former `draw_step_executed_turn` guard is deleted.
        {
            // Sync network state before drawing
            // This ensures revealed cards are queued in the library before draw_card() pops them
            self.sync_to_action();

            // Debug: Log state hash before draw
            #[cfg(feature = "verbose-logging")]
            {
                let player_name = self.get_player_name(active_player);
                let draw_msg = format!("{} draws", player_name);
                self.game.debug_log_state_hash(&draw_msg);
            }

            // Draw a card
            let (_, draw_count) = self.game.draw_card(active_player)?;

            // Check for "second card drawn" triggers (e.g., Knowledge Seeker, Otter-Penguin)
            self.game.check_card_drawn_triggers(active_player, draw_count)?;

            // Push reveals immediately for network mode (server-side)
            // This ensures clients receive the draw reveal before their GameLoop needs it
            self.push_reveals(active_player);

            // Per-card "P draws CARD (id)" logging is now centralised inside
            // GameState::draw_card so every draw source — the mandatory draw
            // step, activated abilities (e.g. Bazaar of Baghdad), spells (e.g.
            // Ancestral Recall), and Loot effects — produces a consistent draw
            // log.
        }

        // CR 504.1: the turn-based mandatory draw happens first; THEN any
        // "at the beginning of your draw step" triggered abilities are put on
        // the stack (CR 603.3). Fire them after the draw above so an extra-draw
        // trigger (Grafted Skullcap, Sylvan Library, Yawgmoth's Bargain) sees
        // the post-mandatory-draw state. WASM re-entry is handled by rewind+replay
        // (no per-turn guard), the same as the upkeep/end-step phase triggers.
        self.check_phase_triggers(TriggerEvent::BeginningOfDraw)?;

        // Print battlefield state AFTER draw step completes
        // This ensures the active player's hand shows the newly drawn card
        // (Previously this was printed at turn start, before draw - see mtg-204)
        if !self.replaying && self.verbosity >= VerbosityLevel::Normal {
            self.print_battlefield_state();
        }

        // MTG Rules 504.2: After draw, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    /// Main phase - players can play spells and lands
    pub(super) fn main_phase(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Fire `Mode$ Phase` delayed triggers registered for this main phase
        // (e.g. Mana Drain's "At the beginning of your next main phase, add
        // {C}...").
        //
        // NETARCH N4 (mtg-53okw/mtg-610): the former main1/main2_delayed_fired_turn
        // guards are gone. `check_delayed_triggers_on_phase` REMOVES each trigger
        // from `delayed_triggers` as it fires (state.rs) and undo-logs that removal
        // (FireDelayedTrigger), so on a forward pass each delayed trigger fires
        // exactly once, and after a rewind the undo log restores the un-fired
        // trigger for the replay. The whole delayed-trigger lifecycle
        // (Register/Fire/SetRememberedAmount + AddMana) is undo-logged, so
        // snapshot/resume and rewind/replay both reverse it correctly. Proven by
        // the snapshot/resume E2E + STRICT native-vs-WASM DIVERGED:0 + 3-deck
        // network mirror gates.
        #[allow(clippy::wildcard_enum_match_arm)]
        let phase = match self.game.turn.current_step {
            crate::game::Step::Main2 => crate::core::TriggerPhase::Main2,
            _ => crate::core::TriggerPhase::Main1,
        };
        {
            let active_player = self.game.turn.active_player;
            self.game.check_delayed_triggers_on_phase(phase, active_player)?;
        }

        // Priority round where players can take actions
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// End step - handle end of turn triggers and priority
    pub(super) fn end_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check for beginning of end step triggers
        self.check_phase_triggers(TriggerEvent::BeginningOfEndStep)?;

        // Fire `Mode$ Phase | Phase$ End Of Turn` delayed triggers registered
        // for the end step (e.g. Berserk's "At the beginning of the next end
        // step, destroy that creature if it attacked this turn"). Mirrors the
        // main-phase delayed-trigger firing site: the call REMOVES + undo-logs
        // each fired trigger so it fires exactly once per forward pass and is
        // restored on rewind for replay (same net-determinism contract).
        {
            let active_player = self.game.turn.active_player;
            self.game
                .check_delayed_triggers_on_phase(crate::core::TriggerPhase::EndStep, active_player)?;
        }

        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Cleanup step - discard to hand size, remove damage
    pub(super) fn cleanup_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Get non-active player
        let non_active_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have non-active player");

        // CR 514.1: Only the active player discards to hand size during cleanup step
        // (The non-active player discards during their own cleanup step)
        for &player_id in &[active_player] {
            let hand_size = self.game.get_player_zones(player_id).map(|z| z.hand.len()).unwrap_or(0);

            let max_hand_size = self.game.get_player(player_id)?.max_hand_size;

            if hand_size > max_hand_size {
                let discard_count = hand_size - max_hand_size;

                log_if_verbose!(
                    self,
                    "{} must discard {} cards (hand size: {}, max: {})",
                    self.get_player_name(player_id),
                    discard_count,
                    hand_size,
                    max_hand_size
                );

                // Get the appropriate controller
                let controller: &mut dyn PlayerController = if player_id == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Create view and print prompt BEFORE checking stop conditions
                // so users see what choice was about to be made when using --stop-when-fixed-exhausted
                {
                    let view = GameStateView::new(self.game, player_id);
                    let hand = view.hand();
                    // Print discard selection prompt (controlled by show_choice_menu flag)
                    if view.logger().should_show_choice_menu() {
                        print!("{}", format_discard_prompt(&view, hand, discard_count));
                    }
                } // Drop view before mutable borrow

                // PREAMBLE: Check stop conditions before asking for choice
                if let Some(result) = self.check_stop_conditions(controller, player_id)? {
                    return Ok(Some(result));
                }

                // Ask controller which cards to discard
                // Capture log size BEFORE asking controller (before controller logs its choice)
                let prior_log_size = self.game.logger.log_count();
                // Get hand cards before calling helper (which creates view internally)
                let hand: SmallVec<[CardId; 8]> = self
                    .game
                    .get_player_zones(player_id)
                    .map(|zones| zones.hand.cards.iter().copied().collect())
                    .unwrap_or_default();
                let choice = self.choose_discard_with_hook(controller, player_id, &hand, discard_count);
                let cards_to_discard = handle_choice_result_break!(choice, self.game, player_id);

                // Log this choice point for snapshot/replay
                let replay_choice = crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                self.log_choice_point(player_id, Some(replay_choice), prior_log_size);

                // Verify correct number of cards selected
                if cards_to_discard.len() != discard_count {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Must discard exactly {discard_count} cards, got {}",
                        cards_to_discard.len()
                    )));
                }

                // Move selected cards to graveyard
                for card_id in cards_to_discard {
                    // Verify card is in hand before moving
                    if let Some(zones) = self.game.get_player_zones(player_id) {
                        if !zones.hand.contains(card_id) {
                            return Err(crate::MtgError::InvalidAction(format!(
                                "Card {card_id:?} not in player's hand"
                            )));
                        }
                    }

                    // Use move_card to properly log the action for undo
                    self.game.move_card(
                        card_id,
                        crate::zones::Zone::Hand,
                        crate::zones::Zone::Graveyard,
                        player_id,
                    )?;

                    log_if_verbose!(
                        self,
                        "{} discards {} ({})",
                        self.get_player_name(player_id),
                        self.game
                            .cards
                            .get(card_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or("Unknown"),
                        card_id
                    );
                }
            }
        }

        // Empty mana pools
        for &player_id in &[active_player, non_active_player] {
            if let Ok(player) = self.game.get_player_mut(player_id) {
                player.mana_pool.clear();
            }
        }

        // Clean up persistent effects that expire at end of turn
        let effects_to_remove = self.game.persistent_effects.find_effects_to_cleanup_at_eot();
        if !effects_to_remove.is_empty() {
            log::debug!(target: "persistent_effects", "Cleaning up {} effects at end of turn", effects_to_remove.len());
            self.game.persistent_effects.remove_many(&effects_to_remove);
        }

        // Clean up delayed triggers that expire at end of turn (ThisTurn$ True)
        // Example: Fatal Fissure's "when that creature dies this turn" expires if not triggered
        let expired_triggers = self.game.delayed_triggers.cleanup_end_of_turn();
        if !expired_triggers.is_empty() {
            log::debug!(
                target: "delayed_triggers",
                "Cleaning up {} delayed triggers at end of turn",
                expired_triggers.len()
            );
            for trigger in &expired_triggers {
                log_if_verbose!(self, "Delayed trigger {} expired (end of turn)", trigger.id.as_u32());
            }
        }

        // Remove damage from creatures and clear regeneration/prevention shields (CR 514.2)
        // Also clear `damaged_by_this_turn` so next turn's "Whenever a creature
        // dealt damage by this card this turn dies, ..." triggers (Sengir
        // Vampire et al.) start with a clean slate per CR 514.2 (3rd bullet).
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get_mut(card_id) {
                if card.is_creature() {
                    card.damage = 0;
                    card.regeneration_shields = 0;
                }
                card.damage_prevention = 0;
                // Disintegrate-style "exile instead of dying" lasts only this
                // turn (CR 614, duration "this turn"); clear at cleanup.
                card.exile_if_would_die_this_turn = false;
                // Maze of Ith's "prevent all combat damage" lasts until end of
                // turn (CR 615 replacement, duration "this turn"); clear at cleanup.
                card.prevent_all_combat_damage_this_turn = false;
                card.damaged_by_this_turn.clear();
            }
        }

        // Clear player damage prevention shields at end of turn (CR 514.2),
        // including source-filtered shields (Circle of Protection).
        for player in &mut self.game.players {
            player.damage_prevention = 0;
            player.source_prevention_shields.clear();
        }

        Ok(None)
    }
}
