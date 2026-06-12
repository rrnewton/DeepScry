//! Priority system and spell resolution
//!
//! This module handles the priority system where players alternate making choices
//! until both pass in succession, then resolves spells from the stack.

use crate::core::{CardId, Effect, PlayerId};
use crate::game::controller::{format_choice_menu, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::game::GameState;
use crate::{handle_choice_result, handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

/// Resolve a `PlayerId` field on an XPaid Effect variant for **logging
/// purposes only** (mtg-521): replace the placeholder (0) sentinel with
/// the caster and the `target_opponent()` sentinel with the actual
/// opponent's id. Mirrors the runtime resolution done by
/// `resolve_x_paid_effect` + the placeholder/opponent resolution in
/// `actions/mod.rs`; without this, the post-resolution log loop in
/// `resolve_top_spell_from_stack` formats those sentinels as the
/// literal strings "Player 1" / "target opponent" instead of the real
/// player name.
fn resolve_log_player(player: PlayerId, caster: PlayerId, opponent: Option<PlayerId>) -> PlayerId {
    if player.is_placeholder() {
        caster
    } else if player.is_target_opponent() {
        opponent.unwrap_or(player)
    } else {
        player
    }
}

/// Note: Wildcards are intentional throughout - Effect enum has 24+ variants.
/// Priority system handles specific effect types (AddMana, GainLife, etc.) specially
/// and passes through others unchanged. Using exhaustive matching would be verbose.
#[allow(clippy::wildcard_enum_match_arm)]
impl<'a> GameLoop<'a> {
    /// Resolve the top spell from the stack
    ///
    /// This removes the spell from the stack and executes its effects.
    /// Implements MTG Comprehensive Rules 608 (Resolving Spells and Abilities).
    pub(super) fn resolve_top_spell_from_stack(&mut self, spell_id: CardId) -> Result<()> {
        // Look up the targets for this spell (already stored as SmallVec)
        let targets: SmallVec<[CardId; 2]> = self
            .game
            .sub_action_scratch
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        log::debug!(
            target: "priority",
            "[RESOLVE] spell_id={}, targets from spell_targets: {:?}",
            spell_id.as_u32(),
            targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
        );

        // Check if verbose logging is enabled (avoids allocations when not logging)
        let should_log = self.verbosity >= VerbosityLevel::Normal && !self.replaying;

        // Verify spell exists before proceeding
        if self.game.cards.get(spell_id).is_err() {
            return Err(crate::MtgError::EntityNotFound(spell_id.as_u32()));
        }

        // Get spell owner (needed for push_reveals even without logging)
        let spell_owner = self.game.cards.get(spell_id).unwrap().owner; // Safe: checked above

        // Get card info for logging ONLY when logging is enabled
        // This avoids String allocation and effects clone in Silent mode.
        // x_paid is captured here so the post-resolve log loop can rewrite
        // XPaid effect variants (DrawCardsXPaid / DiscardCardsXPaid /
        // DealDamageXPaid) into their resolved concrete-count variants
        // before formatting — otherwise the log shows the literal sentinel
        // "X" (e.g. "Mind Twist causes target opponent to discard X card(s)")
        // instead of the real value (mtg-521).
        let (card_name, card_effects, card_owner, card_x_paid) = if should_log {
            let card = self.game.cards.get(spell_id).unwrap(); // Safe: checked above
            (card.name.to_string(), card.effects.clone(), card.owner, card.x_paid)
        } else {
            // In Silent mode, use empty placeholders (never accessed)
            (String::new(), Vec::new(), crate::core::PlayerId::new(0), 0u8)
        };

        if should_log {
            let message = format!("{} ({}) resolves", card_name, spell_id);
            // Use gamelog for official game action
            self.game.logger.gamelog(&message);
        }

        // Resolve the spell (this modifies effects with target replacement)
        self.game.resolve_spell(spell_id, &targets)?;

        // Push reveals after spell resolution for network mode (server-side)
        // Spells can draw cards (e.g., "draw 3 cards", "target player draws 3"),
        // and network clients need the card IDs before their shadow GameLoop draws.
        // Push for both players since spells can target either player for draws.
        self.push_reveals(spell_owner);
        if let Some(opponent) = self.game.get_other_player_id(spell_owner) {
            self.push_reveals(opponent);
        }

        // Log effects for instants/sorceries (only when verbose logging is enabled)
        // Note: We need to manually replace placeholder targets for logging
        if should_log {
            use crate::core::{Effect, TargetRef};
            let mut target_index = 0;
            // Track last resolved target for SubAbility chains using Defined$ Targeted
            let mut last_resolved_target: Option<CardId> = None;
            for effect in &card_effects {
                // Replace placeholder targets with chosen targets for logging
                let effect_to_log = match effect {
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } if target_index < targets.len() => {
                        // Replace placeholder with the actual target for logging.
                        // Player-target sentinels (Lightning Bolt at a player)
                        // become TargetRef::Player; everything else is a
                        // permanent CardId.
                        let raw = targets[target_index];
                        last_resolved_target = Some(raw);
                        let replaced = Effect::DealDamage {
                            target: crate::core::target_ref_from_chosen_target(raw),
                            amount: *amount,
                        };
                        target_index += 1;
                        replaced
                    }
                    // `DB$ DealDamage | Defined$ You` self-damage rider
                    // (Psionic Blast "and 2 damage to you"). The converter
                    // encodes `Defined$ You` as TargetRef::Player(PlayerId(0)),
                    // which is_placeholder(). Resolve the LOG target to the
                    // caster (card_owner) so the gamelog names the player who
                    // actually takes the damage — matching the execution-time
                    // resolution in actions/mod.rs (mtg-533). Without this the
                    // log read "deals 2 damage to Player 1" on a cross-player
                    // cast even though the 2 correctly hit the caster.
                    Effect::DealDamage {
                        target: TargetRef::Player(player_id),
                        amount,
                    } if player_id.is_placeholder() => Effect::DealDamage {
                        target: TargetRef::Player(card_owner),
                        amount: *amount,
                    },
                    Effect::CounterSpell {
                        target,
                        spell_restriction,
                        remember_mana_value,
                    } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::CounterSpell {
                            target: targets[target_index],
                            spell_restriction: spell_restriction.clone(),
                            remember_mana_value: *remember_mana_value,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent {
                        target,
                        restriction,
                        no_regenerate,
                    } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::DestroyPermanent {
                            target: targets[target_index],
                            restriction: restriction.clone(),
                            no_regenerate: *no_regenerate,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent {
                        target,
                        restriction,
                        no_regenerate,
                    } if target.is_self_target() => Effect::DestroyPermanent {
                        target: spell_id,
                        restriction: restriction.clone(),
                        no_regenerate: *no_regenerate,
                    },
                    // `DB$ Tap | Defined$ Targeted` chained after damage
                    // (Falling Star "if it survives, tap it"): resolve the log
                    // target to the parent ability's target, gated on survival
                    // so the gamelog matches the execution path in
                    // actions/mod.rs (mtg-503). If the creature died to the
                    // damage (or left the battlefield), no tap is logged.
                    Effect::TapPermanent { target } if target.is_reuse_previous() => {
                        match last_resolved_target {
                            Some(prev)
                                if self.game.battlefield.contains(prev)
                                    && self.game.cards.try_get(prev).is_some_and(|c| {
                                        let toughness = i32::from(c.current_toughness());
                                        toughness > 0 && toughness > c.damage
                                    }) =>
                            {
                                Effect::TapPermanent { target: prev }
                            }
                            // Creature died / gone → fizzle to a placeholder so
                            // log_effect_execution emits nothing meaningful
                            // (it skips placeholder targets).
                            _ => Effect::TapPermanent {
                                target: CardId::placeholder(),
                            },
                        }
                    }
                    Effect::TapPermanent { target } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::TapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::UntapPermanent { target } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::UntapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                        keywords_granted,
                    } if target.is_placeholder() && target_index < targets.len() => {
                        let resolved_target = targets[target_index];
                        last_resolved_target = Some(resolved_target);
                        let replaced = Effect::PumpCreature {
                            target: resolved_target,
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                            keywords_granted: keywords_granted.clone(),
                        };
                        target_index += 1;
                        replaced
                    }
                    // Handle UntapPermanent with reuse_previous sentinel (from Defined$ Targeted in SubAbility)
                    Effect::UntapPermanent { target } if target.is_reuse_previous() => {
                        if let Some(prev_target) = last_resolved_target {
                            Effect::UntapPermanent { target: prev_target }
                        } else {
                            // No previous target available, use as-is (will show Unknown in log)
                            Effect::UntapPermanent {
                                target: CardId::placeholder(),
                            }
                        }
                    }
                    Effect::ExilePermanent { target } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::ExilePermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    // Player-targeting effects: resolve placeholder (0) to card owner
                    Effect::AddMana {
                        player,
                        mana,
                        produces_chosen_color,
                        amount_var,
                    } if player.is_placeholder() => Effect::AddMana {
                        player: card_owner,
                        mana: *mana,
                        produces_chosen_color: *produces_chosen_color,
                        amount_var: amount_var.clone(),
                    },
                    Effect::DrawCards { player, count } if player.is_placeholder() => Effect::DrawCards {
                        player: card_owner,
                        count: *count,
                    },
                    Effect::GainLife { player, amount } if player.is_placeholder() => Effect::GainLife {
                        player: card_owner,
                        amount: *amount,
                    },
                    Effect::Mill { player, count } if player.is_placeholder() => Effect::Mill {
                        player: card_owner,
                        count: *count,
                    },
                    // AddTurn (Time Walk): the controller takes the extra turn.
                    // Resolve the placeholder player (0) to card_owner so the
                    // post-resolution logger names the correct player, matching
                    // the resolution done in actions/mod.rs (mtg-551).
                    Effect::AddTurn { player, num_turns } if player.is_placeholder() => Effect::AddTurn {
                        player: card_owner,
                        num_turns: *num_turns,
                    },
                    Effect::SearchLibrary {
                        player,
                        card_type_filter,
                        destination,
                        enters_tapped,
                        shuffle,
                    } if player.is_placeholder() => Effect::SearchLibrary {
                        player: card_owner,
                        card_type_filter: card_type_filter.clone(),
                        destination: *destination,
                        enters_tapped: *enters_tapped,
                        shuffle: *shuffle,
                    },
                    Effect::CreateToken {
                        controller,
                        token_script,
                        amount,
                        for_each_player,
                    } if *controller == crate::core::PlayerId::new(0) => Effect::CreateToken {
                        controller: card_owner,
                        token_script: token_script.clone(),
                        amount: *amount,
                        for_each_player: *for_each_player,
                    },
                    // XPaid -> concrete variants so the post-resolution log
                    // shows the real X (e.g. "discard 3 card(s)" rather than
                    // the sentinel "discard X card(s)"). Mirrors the
                    // resolve_x_paid_effect() mapping in actions/mod.rs that
                    // runs at execution time.
                    //
                    // Player resolution: the placeholder (0) means "the
                    // caster" (Defined$ You / no-defined). The
                    // target_opponent() sentinel encodes ValidTgts$ Player
                    // (Mind Twist) and should resolve to the actual
                    // opponent player_id at log time — otherwise the log
                    // reads "causes target opponent to discard 1 card(s)"
                    // (literal sentinel string from get_player_name) rather
                    // than the opponent's actual name. See mtg-521.
                    Effect::DiscardCardsXPaid {
                        player,
                        remember_discarded,
                    } => {
                        let resolved_player =
                            resolve_log_player(*player, card_owner, self.game.get_other_player_id(card_owner));
                        Effect::DiscardCards {
                            player: resolved_player,
                            count: card_x_paid,
                            remember_discarded: *remember_discarded,
                            optional: false,
                            remember_discarding_players: false,
                        }
                    }
                    Effect::DrawCardsXPaid { player } => {
                        let resolved_player =
                            resolve_log_player(*player, card_owner, self.game.get_other_player_id(card_owner));
                        Effect::DrawCards {
                            player: resolved_player,
                            count: card_x_paid,
                        }
                    }
                    // X-damage at a chosen target (Disintegrate, Fireball, Blaze,
                    // ...): NumDmg$ X parses to DealDamageXPaid with target None.
                    // It must consume the next chosen target for display exactly
                    // like the fixed-amount DealDamage { None } arm above —
                    // otherwise the target stays None and log_effect_execution's
                    // None branch invents a phantom "deals N damage to <opponent>"
                    // line even though the real damage went to the creature
                    // (display-only double-resolution, mtg-ioesm). Player-target
                    // sentinels decode to TargetRef::Player here.
                    // DivideEvenly$ RoundedDown (Fireball): consume ALL remaining
                    // chosen targets for the log, dealing floor(X/N) to each, so
                    // the gamelog shows one "deals K to <t>" line per target
                    // (matches the DealDamageDivided execution in actions/mod.rs).
                    Effect::DealDamageXPaid {
                        target: TargetRef::None,
                        divide: crate::core::DamageDivision::EvenlyRoundedDown,
                    } if target_index < targets.len() => {
                        let remaining = &targets[target_index..];
                        let resolved_targets: smallvec::SmallVec<[TargetRef; 4]> = remaining
                            .iter()
                            .map(|&t| crate::core::target_ref_from_chosen_target(t))
                            .collect();
                        last_resolved_target = remaining.last().copied();
                        target_index = targets.len();
                        let n = resolved_targets.len().max(1) as i32;
                        Effect::DealDamageDivided {
                            targets: resolved_targets,
                            amount_each: i32::from(card_x_paid) / n,
                        }
                    }
                    Effect::DealDamageXPaid {
                        target: TargetRef::None,
                        ..
                    } if target_index < targets.len() => {
                        let raw = targets[target_index];
                        last_resolved_target = Some(raw);
                        target_index += 1;
                        Effect::DealDamage {
                            target: crate::core::target_ref_from_chosen_target(raw),
                            amount: i32::from(card_x_paid),
                        }
                    }
                    Effect::DealDamageXPaid { target, .. } => Effect::DealDamage {
                        target: target.clone(),
                        amount: i32::from(card_x_paid),
                    },
                    // Disintegrate's exile-instead-of-dying marker binds to the
                    // parent DealDamage's target (reuse_previous) for the log.
                    Effect::ExileIfWouldDieThisTurn { target }
                        if target.is_reuse_previous() || target.is_placeholder() =>
                    {
                        match last_resolved_target {
                            Some(prev) => Effect::ExileIfWouldDieThisTurn { target: prev },
                            None => effect.clone(),
                        }
                    }
                    _ => effect.clone(),
                };

                self.log_effect_execution(&card_name, spell_id, &effect_to_log, card_owner);
            }

            // Check if it's a permanent entering battlefield. Guard on the card
            // actually being on the battlefield: an Adventure (instant/sorcery)
            // spell that just resolved is a creature card now sitting in EXILE
            // (CR 715.3d), so `is_creature()` is true but it did NOT enter the
            // battlefield — without this guard it would be mis-logged as entering.
            if self.game.battlefield.contains(spell_id) {
                if let Some(card) = self.game.cards.try_get(spell_id) {
                    if card.is_creature() {
                        // Get effective P/T applying all continuous effects (CR 613)
                        let power = self
                            .game
                            .get_effective_power(spell_id)
                            .unwrap_or_else(|_| i32::from(card.current_power()));
                        let toughness = self
                            .game
                            .get_effective_toughness(spell_id)
                            .unwrap_or_else(|_| i32::from(card.current_toughness()));
                        let message = format!(
                            "{} ({}) enters the battlefield as a {}/{} creature",
                            card_name, spell_id, power, toughness
                        );
                        // Use gamelog for official game action
                        self.game.logger.gamelog(&message);
                    }
                }
            }

            // Set starting loyalty counters for planeswalkers (MTG CR 306.5b)
            if let Some(card) = self.game.cards.try_get(spell_id) {
                if let Some(loyalty) = card.definition.loyalty {
                    let card_name_str = card.name.to_string();
                    if let Ok(card_mut) = self.game.cards.get_mut(spell_id) {
                        card_mut.add_counter(crate::core::CounterType::Loyalty, loyalty);
                    }
                    if should_log {
                        self.game.logger.gamelog(&format!(
                            "{} ({}) enters with {} loyalty",
                            card_name_str, spell_id, loyalty
                        ));
                    }
                }
            }
        }

        // Check state-based actions after spell effects resolve (MTG Rules 704.3)
        // This handles lethal damage from damage-dealing spells like Lightning Bolt
        if let Err(e) = self.game.check_lethal_damage() {
            if should_log {
                eprintln!("    Failed to check lethal damage: {e}");
            }
        }
        // Check legendary rule (MTG CR 704.5j)
        if let Err(e) = self.game.check_legendary_rule() {
            if should_log {
                eprintln!("    Failed to check legendary rule: {e}");
            }
        }
        // Apply originally-printed-set state-trigger sweeps (City in a Bottle).
        if let Err(e) = self.game.check_set_origin_sacrifice() {
            if should_log {
                eprintln!("    Failed to check set-origin sacrifice: {e}");
            }
        }
        // Check aura attachment (MTG CR 704.5d)
        if let Err(e) = self.game.check_aura_attachment() {
            if should_log {
                eprintln!("    Failed to check aura attachment: {e}");
            }
        }
        // Re-derive control from control-changing Auras (MTG CR 613.2, layer 2).
        if let Err(e) = self.game.recompute_aura_control() {
            if should_log {
                eprintln!("    Failed to recompute aura control: {e}");
            }
        }
        // Re-derive control from source-duration GainControl grants (Aladdin; CR 800.4a).
        if let Err(e) = self.game.recompute_source_control() {
            if should_log {
                eprintln!("    Failed to recompute source control: {e}");
            }
        }
        // Check equipment attachment (MTG CR 704.5n)
        if let Err(e) = self.game.check_equipment_attachment() {
            if should_log {
                eprintln!("    Failed to check equipment attachment: {e}");
            }
        }

        // Remove the spell from our targets tracking
        self.game
            .sub_action_scratch
            .spell_targets
            .retain(|(id, _)| *id != spell_id);

        Ok(())
    }

    /// Priority round - players get chances to act until both pass
    ///
    /// This implements the priority system where players alternate making choices
    /// until both pass in succession, then resolves spells from the stack.
    ///
    /// ## MTG Rules Implementation
    /// - Gets all available spell abilities (lands, spells, abilities)
    /// - Calls controller.choose_spell_ability_to_play() for each priority window
    /// - Handles the chosen ability appropriately:
    ///   - PlayLand: Resolves directly (no stack)
    ///   - CastSpell: Puts spell on stack (MTG Rules 601)
    ///   - ActivateAbility: TODO - should go on stack for non-mana abilities
    /// - When both players pass with spells on stack, resolves top spell (MTG Rules 117.4)
    /// - Repeats until stack is empty and both players pass
    pub(super) fn priority_round(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;
        let non_active_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have non-active player");

        // Outer loop: resolve stack until empty
        loop {
            // Use persistent priority state from TurnStructure (for WASM NeedInput resumption)
            // If priority_player is None, this is a fresh priority round - start with active player
            let mut current_priority = self.game.turn.priority_player.unwrap_or(active_player);
            let mut consecutive_passes = self.game.turn.consecutive_passes;
            let mut action_count = 0;
            const MAX_ACTIONS_PER_PRIORITY: usize = 1000;

            // Persist initial state
            self.game.turn.priority_player = Some(current_priority);

            // Inner loop: pass priority until both players pass
            while consecutive_passes < 2 {
                // Safety check to prevent infinite loops
                action_count += 1;
                if action_count > MAX_ACTIONS_PER_PRIORITY {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Priority round exceeded max actions ({MAX_ACTIONS_PER_PRIORITY}), possible infinite loop"
                    )));
                }

                // Get the appropriate controller
                let controller: &mut dyn PlayerController = if current_priority == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // NETWORK SYNC PROTOCOL:
                // For network controllers (Remote, Network), we need to:
                // 1. Call prepare_for_priority_choice() to block on MVar and receive ChoiceRequest/OpponentChoice
                // 2. Then call sync_to_action() to process CardRevealed messages that are now guaranteed buffered
                // 3. Then compute abilities (now correct, includes newly drawn cards)
                //
                // This solves a race condition where sync_to_action() might run before the WS reader
                // has buffered the CardRevealed messages, causing abilities to be computed with stale data.
                let is_network_controlled = matches!(
                    controller.get_controller_type(),
                    ControllerType::Remote | ControllerType::Network
                );

                if is_network_controlled {
                    // For network controllers: prepare first (blocks until network data received)
                    if !controller.prepare_for_priority_choice() {
                        // Game ended or error - return ExitGame
                        return Ok(None);
                    }
                }

                // Sync network state now that we know reveals are buffered
                // For network controllers, the prepare call above guarantees ChoiceRequest/OpponentChoice
                // has been received, which means all preceding CardRevealed are buffered
                self.sync_to_action();

                // State-based-action-like sweep BEFORE a player receives priority
                // (CR 704.3): apply any `Mode$ Always` originally-printed-set
                // sweep (City in a Bottle). This covers the quiescent board (e.g.
                // a permanent already in play at game/puzzle start) that the
                // post-resolution SBA sites don't reach. Early-returns with one
                // battlefield scan when no sweeper is present (the common case),
                // so it is cheap and deterministic.
                if let Err(e) = self.game.check_set_origin_sacrifice() {
                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                        eprintln!("    Failed to check set-origin sacrifice: {e}");
                    }
                }

                // WASM RESUMPTION: Complete a pending typecycling library search.
                //
                // When the WASM game loop is interrupted (NeedInput) during the library
                // search phase of typecycling, `pending_cycling_search` is set. On the
                // next game loop invocation we bypass `choose_spell_ability_to_play` and
                // call `choose_from_library_with_hook` directly. Without this, the queued
                // LibrarySearchByName OpponentChoice would be mistakenly consumed by
                // `choose_spell_ability_to_play`, corrupting the RNG and causing a desync.
                if let Some((search_player, ref land_type)) =
                    self.game.sub_action_scratch.pending_cycling_search.clone()
                {
                    if search_player == current_priority {
                        let land_type = land_type.clone();
                        log::debug!(
                            "[WASM RESUME] Resuming cycling library search for player {:?}, land_type={}",
                            search_player,
                            land_type.as_str()
                        );
                        // Rebuild valid_cards with the same filter used in the cycling handler.
                        let filter = format!("Land.{}", land_type.as_str());
                        let library_cards = self
                            .game
                            .player_zones
                            .iter()
                            .find(|(id, _)| *id == search_player)
                            .map(|(_, zones)| zones.library.cards.clone())
                            .unwrap_or_default();
                        let valid_cards: Vec<CardId> = library_cards
                            .iter()
                            .copied()
                            .filter(|&card_id| {
                                self.game
                                    .cards
                                    .get(card_id)
                                    .map(|card| {
                                        crate::game::state::GameState::card_matches_search_filter(card, &filter)
                                    })
                                    .unwrap_or(false)
                            })
                            .collect();
                        // Capture log size BEFORE the library search choice (for log_choice_point).
                        let prior_log_size = self.game.logger.log_count();

                        // Ask controller to complete the library search.
                        let lib_choice = self.choose_from_library_with_hook(controller, search_player, &valid_cards);
                        let chosen_card_opt = handle_choice_result_break!(lib_choice, self.game, search_player);

                        // Library search succeeded — clear the pending state.
                        self.game.sub_action_scratch.pending_cycling_search = None;

                        // Log this choice point for undo/replay (mirrors the cycling handler).
                        // Record the AUTHORITATIVE fetched CardId (not a shadow-fragile
                        // positional index) so rewind+replay applies the exact move.
                        let replay_choice = crate::game::ReplayChoice::LibrarySearch(chosen_card_opt);
                        self.log_choice_point(search_player, Some(replay_choice), prior_log_size);

                        // Move chosen card from library to hand.
                        // The card may already be in hand (moved via CardRevealed); if so,
                        // move_card fails gracefully — the shadow game tolerates this.
                        if let Some(chosen_card) = chosen_card_opt {
                            let _ = self.game.move_card(
                                chosen_card,
                                crate::zones::Zone::Library,
                                crate::zones::Zone::Hand,
                                search_player,
                            );
                        }

                        // Shuffle library after searching (MTG CR 702.29).
                        self.game.shuffle_library(search_player);

                        // Cycling is now fully complete — switch priority to the other player.
                        consecutive_passes = 0;
                        self.game.turn.consecutive_passes = 0;
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                        self.game.turn.priority_player = Some(current_priority);

                        continue; // Re-enter while loop with new current_priority.
                    }
                }

                // WASM RESUMPTION: Bypass spell ability selection when resuming pending activation.
                //
                // When the WASM game loop is interrupted (NeedInput) during target selection
                // of an activated ability, `pending_activation` is set. On the next
                // step_harness() call, we skip `choose_spell_ability_to_play` (which would
                // misroute the queued target ChoiceRequest) and resume directly in the
                // ActivateAbility arm where the target choice is completed.
                let choice = 'ability_choice: {
                    if let Some((act_player, act_card, act_idx)) = self.game.sub_action_scratch.pending_activation {
                        if act_player == current_priority {
                            log::debug!(
                                "[WASM RESUME] Resuming pending activation of {:?} ability {} for player {:?}",
                                act_card,
                                act_idx,
                                act_player
                            );
                            // Sync reveals before resuming (mirrors sync_to_action() in normal loop)
                            self.sync_to_action();
                            break 'ability_choice Some(crate::core::SpellAbility::ActivateAbility {
                                card_id: act_card,
                                ability_index: act_idx,
                            });
                        }
                    }

                    // Loop to allow undo/retry for spell ability choices
                    loop {
                        // Get all available spell abilities for this player.
                        //
                        // OPTIMIZATION: get_available_spell_abilities now returns &[SpellAbility] from a
                        // reused internal buffer, eliminating repeated Vec allocations. We check emptiness
                        // first (no copy needed), then copy into SmallVec only when there are abilities
                        // (avoiding heap allocation for typical hand sizes up to 16 cards).
                        let available_count = self.get_available_spell_abilities(current_priority).len();

                        // Log abilities for debugging network sync issues
                        // OPTIMIZATION: Only format abilities when debug logging is enabled
                        // The format!("{:?}", a) calls were allocating ~2% of CPU time even when
                        // debug logging was disabled at runtime.
                        if log::log_enabled!(log::Level::Debug) && available_count > 0 && available_count <= 5 {
                            // Log the actual abilities available (the detail branch — kept at debug!).
                            let abilities: smallvec::SmallVec<[String; 8]> =
                                self.abilities_buffer.iter().map(|a| format!("{:?}", a)).collect();
                            log::debug!(
                                "Priority check: player {:?} has {} available abilities at action_count={}: {:?}",
                                current_priority,
                                available_count,
                                self.game.action_count(),
                                abilities
                            );
                        } else if log::log_enabled!(log::Level::Trace) {
                            // No-detail branch (0 abilities, or >5): demoted to trace! — this is
                            // the per-priority "nothing to choose" echo that dominated debug logs.
                            log::trace!(
                                "Priority check: player {:?} has {} available abilities, action_count={}",
                                current_priority,
                                available_count,
                                self.game.action_count()
                            );
                        }

                        // If no actions available, automatically pass priority without asking controller
                        // Only invoke controller when there's an actual choice to make
                        //
                        // EXCEPTION: Remote/Network controllers MUST always be asked:
                        // - Remote: Client-side opponent controller, we don't know hidden hand contents
                        // - Network: Server-side controller, must notify clients even on 0-ability pass
                        // The server will send the actual ability via OpponentChoice.
                        let ctrl_type = controller.get_controller_type();
                        let is_network_controlled =
                            matches!(ctrl_type, ControllerType::Remote | ControllerType::Network);
                        if available_count == 0 && !is_network_controlled {
                            // No available actions - automatically pass priority
                            break None;
                        }

                        // Copy abilities into SmallVec now that we know there are some.
                        // SmallVec<[_; 16]> covers typical hand sizes without heap allocation.
                        // We need a copy because self.game is accessed later in the loop.
                        let available: smallvec::SmallVec<[_; 16]> = self.abilities_buffer.iter().cloned().collect();

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
                                "🔍 [REPLAY_CLEAR_BEFORE_CHOICE] choice_counter={}, baseline={}, CLEARING replay mode",
                                self.choice_counter, self.baseline_choice_count
                            );
                            self.replaying = false;
                            if self.verbosity >= VerbosityLevel::Verbose {
                                println!("✅ REPLAY MODE COMPLETE - will present new choice to controller");
                            }
                        } else if self.replaying {
                            eprintln!(
                                "🔍 [REPLAY_STILL_ACTIVE] choice_counter={}, baseline={}, remaining={}",
                                self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                            );
                        }

                        // Print choice menu BEFORE checking stop conditions so
                        // the available actions for the current choice point always
                        // appear in the text output.  External agents (agentplay)
                        // parse the LAST "available actions:" block to learn what
                        // options the choosing player has.
                        {
                            let view = GameStateView::new(self.game, current_priority);
                            if view.logger().should_show_choice_menu() && !available.is_empty() {
                                let wants_ctx = controller.wants_context();
                                print!("{}", format_choice_menu(&view, &available, wants_ctx));
                            }
                        } // Drop view before mutable borrow

                        // Check stop conditions AFTER printing the menu.
                        // Snapshots are still taken before the controller acts.
                        if let Some(result) = self.check_stop_conditions(controller, current_priority)? {
                            return Ok(Some(result));
                        }

                        // Ask controller to choose one (or None to pass)
                        // Capture log size BEFORE asking controller (before controller logs its choice)
                        let prior_log_size = self.game.logger.log_count();
                        // Use network-aware helper (creates view internally, handles pre-choice hook)
                        let choice_result =
                            self.choose_spell_ability_with_hook(controller, current_priority, &available);
                        let choice_value = handle_choice_result!(choice_result, self.game, current_priority);

                        // IMPORTANT: Sync network state after receiving opponent choice
                        // During wait_for_choice(), reveals may have arrived via WebSocket for cards
                        // that the opponent is about to play. We need to process those reveals NOW
                        // before the game tries to act on the choice (e.g., cast a spell from hand).
                        self.sync_to_action();

                        // Log this choice point for snapshot/replay
                        let replay_choice = crate::game::ReplayChoice::SpellAbility(choice_value.clone());
                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                        break choice_value;
                    } // close inner loop
                }; // close 'ability_choice labeled block

                match choice {
                    None => {
                        // Controller chose to pass priority
                        consecutive_passes += 1;
                        self.game.turn.consecutive_passes = consecutive_passes;
                        let view = GameStateView::new(self.game, current_priority);
                        controller.on_priority_passed(&view);

                        // Switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                        self.game.turn.priority_player = Some(current_priority);
                    }
                    Some(ability) => {
                        // Controller chose an ability to play
                        consecutive_passes = 0; // Reset pass counter
                        self.game.turn.consecutive_passes = 0;

                        // Adventure cast (CR 715): the player chose to cast the
                        // instant/sorcery half of an Adventurer card from hand.
                        // Swap the card's spell-relevant characteristics to the
                        // Adventure face (snapshot-logged for rewind safety) and
                        // mark it "cast as adventure", then fall through to the
                        // standard `CastSpell` casting pipeline — no duplication
                        // of the modal/target/X/cost handling. On resolution the
                        // card is exiled (not graveyarded) and the creature half
                        // becomes castable from exile (`resolve_spell_finalize`).
                        let ability = if let crate::core::SpellAbility::CastAdventure { card_id } = ability {
                            if let Err(e) = self.game.begin_adventure_cast(card_id) {
                                log::warn!(target: "priority", "Failed to begin Adventure cast: {}", e);
                            }
                            crate::core::SpellAbility::CastSpell { card_id }
                        } else {
                            ability
                        };

                        match ability {
                            crate::core::SpellAbility::PlayLand { card_id } => {
                                // Debug: Log state hash before playing land
                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.as_str())
                                    .unwrap_or("Unknown");
                                // Only format debug message if debug state hash logging is enabled
                                if self.game.logger.debug_state_hash_enabled() {
                                    let play_msg = format!(
                                        "{} plays {} ({})",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        card_id
                                    );
                                    self.game.debug_log_state_hash(&play_msg);
                                }

                                // Play land - resolves directly (no stack)
                                if let Err(e) = self.game.play_land(current_priority, card_id) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        eprintln!("  Error playing land: {e}");
                                    }
                                    // Treat failed land play like passing priority to prevent infinite loops
                                    // This can happen if controller makes invalid choices
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                } else {
                                    let card_name = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.as_str())
                                        .unwrap_or("Unknown");

                                    if self.verbosity >= VerbosityLevel::Normal {
                                        if !self.replaying {
                                            let message = format!(
                                                "{} plays {} ({})",
                                                self.get_player_name(current_priority),
                                                card_name,
                                                card_id
                                            );
                                            // Use gamelog for official game actions
                                            self.game.logger.gamelog(&message);
                                        } else if self.verbosity >= VerbosityLevel::Verbose {
                                            let message = format!(
                                                "[SUPPRESSED] {} plays {} ({})",
                                                self.get_player_name(current_priority),
                                                card_name,
                                                card_id
                                            );
                                            self.game.logger.verbose(&message);
                                        }
                                    }

                                    // MTG Rules 116.3: "If a player takes a special action, that player receives priority afterward."
                                    // MTG Rules 117.3c: "If a player has priority when they cast a spell, activate an ability,
                                    //                     or take a special action, that player receives priority afterward."
                                    // Playing a land is a special action, so the player retains priority.
                                    // Continue loop with same current_priority to give player another action opportunity.
                                    continue;
                                }
                            }
                            crate::core::SpellAbility::CastSpell { card_id } => {
                                // Cast spell using 8-step process
                                // Mana will be tapped during step 6 (NOT here!)

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                // Debug: Log state hash before casting spell (only if enabled)
                                if self.game.logger.debug_state_hash_enabled() {
                                    let cast_msg = format!(
                                        "{} casts {} ({}) (putting on stack)",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        card_id
                                    );
                                    self.game.debug_log_state_hash(&cast_msg);
                                }

                                if self.verbosity >= VerbosityLevel::Normal {
                                    if !self.replaying {
                                        let message = format!(
                                            "{} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        // Use gamelog for official game actions
                                        self.game.logger.gamelog(&message);
                                    } else if self.verbosity >= VerbosityLevel::Verbose {
                                        let message = format!(
                                            "[SUPPRESSED] {} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        self.game.logger.verbose(&message);
                                    }
                                }

                                // MTG Rule 601.2b: Modal choice happens BEFORE targeting
                                // Check if this is a modal spell and prompt for mode selection
                                if let Ok(Some(Effect::ModalChoice {
                                    modes,
                                    num_to_choose,
                                    min_to_choose,
                                    can_repeat_modes,
                                })) = self.game.get_modal_choice_info(card_id)
                                {
                                    // Get which modes have valid targets
                                    let valid_modes = self
                                        .game
                                        .get_valid_modes_for_spell(card_id, current_priority)
                                        .unwrap_or_default();

                                    // Filter to only modes with valid targets
                                    let valid_mode_indices: Vec<usize> = valid_modes
                                        .iter()
                                        .filter(|(_, has_targets)| *has_targets)
                                        .map(|(idx, _)| *idx)
                                        .collect();

                                    // If no modes have valid targets, the spell can't be cast legally
                                    // (this shouldn't happen if the spell was offered as castable)
                                    if valid_mode_indices.is_empty() {
                                        log::warn!(
                                            target: "priority",
                                            "Modal spell has no modes with valid targets, skipping"
                                        );
                                        // Continue without applying any modes - spell will fizzle
                                    } else {
                                        // Get mode descriptions for valid modes only
                                        let mode_descriptions: Vec<String> = valid_mode_indices
                                            .iter()
                                            .filter_map(|&idx| modes.get(idx).map(|m| m.description.clone()))
                                            .collect();

                                        // Ask controller to choose from valid modes
                                        let prior_log_size = self.game.logger.log_count();
                                        let choice = self.choose_modes_with_hook(
                                            controller,
                                            current_priority,
                                            card_id,
                                            &mode_descriptions,
                                            num_to_choose as usize,
                                            min_to_choose as usize,
                                            can_repeat_modes,
                                        );
                                        let selected_modes =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        // Map the controller's selection (indices into valid_mode_indices)
                                        // back to the original mode indices
                                        let original_indices: Vec<usize> = selected_modes
                                            .iter()
                                            .filter_map(|&idx| valid_mode_indices.get(idx).copied())
                                            .collect();

                                        // Log the mode choice (using original indices)
                                        let replay_choice = crate::game::ReplayChoice::Modes(
                                            original_indices.iter().copied().collect(),
                                        );
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        // Apply selected modes to the spell (replaces ModalChoice with mode effects)
                                        if let Err(e) = self.game.apply_selected_modes(card_id, &original_indices) {
                                            log::warn!(
                                                target: "priority",
                                                "Failed to apply selected modes: {}",
                                                e
                                            );
                                        }

                                        // Log which mode was chosen
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            for &mode_idx in &original_indices {
                                                if let Some(mode) = modes.get(mode_idx) {
                                                    let message = format!(
                                                        "{} chooses mode: {}",
                                                        self.get_player_name(current_priority),
                                                        mode.description
                                                    );
                                                    self.game.logger.gamelog(&message);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Step 2b: X value selection (MTG CR 601.2b)
                                // If the spell has X in its mana cost, ask controller to choose X
                                {
                                    let has_x = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.mana_cost.has_x())
                                        .unwrap_or(false);
                                    if has_x {
                                        // Calculate maximum X the player could pay
                                        // max_x = (total untapped sources + pool - non-X cost) / x_count
                                        self.mana_engine.update_mut(self.game, current_priority);
                                        let untapped_sources =
                                            self.mana_engine
                                                .all_sources()
                                                .iter()
                                                .filter(|s| !s.is_tapped && !s.has_summoning_sickness)
                                                .count() as u8;
                                        let pool_mana = self
                                            .game
                                            .get_player(current_priority)
                                            .map(|p| p.total_available_mana().total())
                                            .unwrap_or(0);
                                        let max_mana = untapped_sources.saturating_add(pool_mana);
                                        let card = self.game.cards.get(card_id).unwrap();
                                        let colored_cost = card.mana_cost.cmc(); // colored + generic (excluding X)
                                        let x_count = card.mana_cost.x_count;
                                        // Reserve mana for a relative per-target cost (Fireball, CR
                                        // 601.2f) so the X chosen here leaves room to actually pay the
                                        // `{1}`-per-extra-target surcharge once targets are picked.
                                        // X (601.2b) is announced BEFORE targets (601.2c), so we
                                        // conservatively reserve for the worst case of hitting every
                                        // legal target (num_valid - 1 extra). This keeps the spell
                                        // castable for any chosen target count and is fully
                                        // deterministic (num_valid is public state).
                                        let target_reserve = if card.definition.cache.spell_relative_target_cost {
                                            let num_valid = self
                                                .game
                                                .get_valid_targets_for_spell(card_id)
                                                .map(|t| t.len())
                                                .unwrap_or(0);
                                            num_valid.saturating_sub(1).min(u8::MAX as usize) as u8
                                        } else {
                                            0
                                        };
                                        let reserved_cost = colored_cost.saturating_add(target_reserve);
                                        let max_x = if x_count > 0 && max_mana > reserved_cost {
                                            (max_mana - reserved_cost) / x_count
                                        } else {
                                            0
                                        };

                                        let prior_log_size = self.game.logger.log_count();
                                        let view = GameStateView::new(self.game, current_priority);
                                        let x_choice = controller.choose_x_value(&view, card_id, max_x);
                                        let x_value =
                                            handle_choice_result_break!(x_choice, self.game, current_priority);

                                        // Clamp to max
                                        let x_value = x_value.min(max_x);

                                        // Store X paid on the card, snapshotting the
                                        // prior value for undo first (mtg-728 sig-2g).
                                        self.game.set_x_paid_logged(card_id, x_value);

                                        // Log X value choice
                                        let replay_choice = crate::game::ReplayChoice::XValue(x_value);
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            let message = format!("  → X = {}", x_value);
                                            self.game.logger.gamelog(&message);
                                        }
                                    }
                                }

                                // Step 2c: Multikicker payment (CR 702.33a)
                                // If the spell has Multikicker, the caster may pay
                                // the kicker cost any number of additional times.
                                // Heuristic: pay as many times as mana allows
                                // (greedy — always kick as much as possible).
                                {
                                    let multikicker_cost = self
                                        .game
                                        .cards
                                        .try_get(card_id)
                                        .and_then(|c| c.keywords.get_args(crate::core::Keyword::Multikicker))
                                        .and_then(|args| {
                                            if let crate::core::KeywordArgs::Multikicker { cost } = args {
                                                Some(crate::core::ManaCost {
                                                    generic: cost.generic,
                                                    white: cost.white,
                                                    blue: cost.blue,
                                                    black: cost.black,
                                                    red: cost.red,
                                                    green: cost.green,
                                                    colorless: cost.colorless,
                                                    x_count: cost.x_count,
                                                })
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(kick_cost) = multikicker_cost {
                                        let kick_cmc = kick_cost.cmc();
                                        if kick_cmc > 0 {
                                            self.mana_engine.update_mut(self.game, current_priority);
                                            let untapped_sources =
                                                self.mana_engine
                                                    .all_sources()
                                                    .iter()
                                                    .filter(|s| !s.is_tapped && !s.has_summoning_sickness)
                                                    .count() as u8;
                                            let pool_mana = self
                                                .game
                                                .get_player(current_priority)
                                                .map(|p| p.total_available_mana().total())
                                                .unwrap_or(0);
                                            let total_mana = untapped_sources.saturating_add(pool_mana);
                                            // Reserve mana for the base spell cost
                                            let base_cost =
                                                self.game.cards.get(card_id).map(|c| c.mana_cost.cmc()).unwrap_or(0);
                                            let available_for_kicker = total_mana.saturating_sub(base_cost);
                                            let max_kicks = available_for_kicker / kick_cmc;
                                            if max_kicks > 0 {
                                                self.game.set_times_kicked_logged(card_id, max_kicks);
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    let msg = format!(
                                                        "  → Multikicker paid {} time{}",
                                                        max_kicks,
                                                        if max_kicks == 1 { "" } else { "s" }
                                                    );
                                                    self.game.logger.gamelog(&msg);
                                                }
                                            } else {
                                                // Ensure times_kicked is 0 (reset from any prior value)
                                                self.game.set_times_kicked_logged(card_id, 0);
                                            }
                                        }
                                    }
                                }

                                // Step 2c.5: Kicker payment (CR 702.32)
                                // If the spell has the Kicker keyword, the caster may
                                // pay the kicker cost once as an optional additional cost.
                                // Heuristic: always pay kicker when mana allows (greedy —
                                // the kicked mode always has equal or greater effect, e.g.
                                // Firebending Lesson deals 5 kicked vs 2 unkicked).
                                {
                                    let kicker_cost = self
                                        .game
                                        .cards
                                        .try_get(card_id)
                                        .and_then(|c| c.keywords.get_args(crate::core::Keyword::Kicker))
                                        .and_then(|args| {
                                            if let crate::core::KeywordArgs::Kicker { cost } = args {
                                                Some(crate::core::ManaCost {
                                                    generic: cost.generic,
                                                    white: cost.white,
                                                    blue: cost.blue,
                                                    black: cost.black,
                                                    red: cost.red,
                                                    green: cost.green,
                                                    colorless: cost.colorless,
                                                    x_count: cost.x_count,
                                                })
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(kick_cost) = kicker_cost {
                                        let kick_cmc = kick_cost.cmc();
                                        if kick_cmc > 0 {
                                            self.mana_engine.update_mut(self.game, current_priority);
                                            let untapped_sources =
                                                self.mana_engine
                                                    .all_sources()
                                                    .iter()
                                                    .filter(|s| !s.is_tapped && !s.has_summoning_sickness)
                                                    .count() as u8;
                                            let pool_mana = self
                                                .game
                                                .get_player(current_priority)
                                                .map(|p| p.total_available_mana().total())
                                                .unwrap_or(0);
                                            let total_mana = untapped_sources.saturating_add(pool_mana);
                                            // Reserve mana for the base spell cost
                                            let base_cost =
                                                self.game.cards.get(card_id).map(|c| c.mana_cost.cmc()).unwrap_or(0);
                                            let available_for_kicker = total_mana.saturating_sub(base_cost);
                                            if available_for_kicker >= kick_cmc {
                                                self.game.set_kicker_paid_logged(card_id, true);
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    let msg = "  → Kicker paid".to_string();
                                                    self.game.logger.gamelog(&msg);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Step 2c.6: Offspring payment (CR 702.198)
                                // If the creature spell has the Offspring keyword, the
                                // caster may pay the Offspring cost once as an optional
                                // additional cost. When the creature enters, a 1/1 token
                                // copy of it is created. Heuristic: always pay Offspring
                                // when mana allows (greedy — the extra token is always
                                // advantageous).
                                {
                                    let offspring_cost = self
                                        .game
                                        .cards
                                        .try_get(card_id)
                                        .and_then(|c| c.keywords.get_args(crate::core::Keyword::Offspring))
                                        .and_then(|args| {
                                            if let crate::core::KeywordArgs::Offspring { cost } = args {
                                                Some(crate::core::ManaCost {
                                                    generic: cost.generic,
                                                    white: cost.white,
                                                    blue: cost.blue,
                                                    black: cost.black,
                                                    red: cost.red,
                                                    green: cost.green,
                                                    colorless: cost.colorless,
                                                    x_count: cost.x_count,
                                                })
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(off_cost) = offspring_cost {
                                        let off_cmc = off_cost.cmc();
                                        if off_cmc > 0 {
                                            self.mana_engine.update_mut(self.game, current_priority);
                                            let untapped_sources =
                                                self.mana_engine
                                                    .all_sources()
                                                    .iter()
                                                    .filter(|s| !s.is_tapped && !s.has_summoning_sickness)
                                                    .count() as u8;
                                            let pool_mana = self
                                                .game
                                                .get_player(current_priority)
                                                .map(|p| p.total_available_mana().total())
                                                .unwrap_or(0);
                                            let total_mana = untapped_sources.saturating_add(pool_mana);
                                            // Reserve mana for the base spell cost (already paid by
                                            // the time we reach here; conservative check).
                                            let base_cost =
                                                self.game.cards.get(card_id).map(|c| c.mana_cost.cmc()).unwrap_or(0);
                                            let available_for_offspring = total_mana.saturating_sub(base_cost);
                                            if available_for_offspring >= off_cmc {
                                                self.game.set_offspring_paid_logged(card_id, true);
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    let msg = "  → Offspring paid".to_string();
                                                    self.game.logger.gamelog(&msg);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Step 2d: Bargain payment (CR 702.162)
                                // If the spell has the Bargain keyword, the caster
                                // may sacrifice an artifact, enchantment, or token as
                                // an optional additional cost. Heuristic: always pay
                                // Bargain if there is a token available (cheapest
                                // sacrifice, maximum value from the rider).
                                {
                                    let has_bargain = self
                                        .game
                                        .cards
                                        .try_get(card_id)
                                        .is_some_and(|c| c.keywords.contains(crate::core::Keyword::Bargain));
                                    if has_bargain {
                                        // Find a token on the battlefield controlled by current_priority
                                        // to sacrifice (tokens are the cheapest Bargain fodder).
                                        // Fall back to any artifact or enchantment.
                                        let sacrifice_target = self
                                            .game
                                            .battlefield
                                            .cards
                                            .iter()
                                            .find(|&&pid| {
                                                self.game.cards.try_get(pid).is_some_and(|c| {
                                                    c.controller == current_priority
                                                        && c.id != card_id
                                                        && (c.is_token
                                                            || c.is_type(&crate::core::CardType::Artifact)
                                                            || c.is_type(&crate::core::CardType::Enchantment))
                                                })
                                            })
                                            .copied();
                                        if let Some(sacrifice_id) = sacrifice_target {
                                            // Mark bargain_paid BEFORE cast_spell_8_step
                                            // (which reads it at resolution).
                                            self.game.set_bargain_paid_logged(card_id, true);
                                            // Perform the sacrifice: move to graveyard.
                                            let dest = self.game.death_destination_for_card(sacrifice_id);
                                            let _ = self.game.move_card(
                                                sacrifice_id,
                                                crate::zones::Zone::Battlefield,
                                                dest,
                                                current_priority,
                                            );
                                            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                if let Some(sac_name) = self
                                                    .game
                                                    .cards
                                                    .try_get(sacrifice_id)
                                                    .map(|c| c.name.as_str().to_string())
                                                {
                                                    self.game.logger.gamelog(&format!(
                                                        "  → Bargain: sacrificed {} as additional cost",
                                                        sac_name
                                                    ));
                                                }
                                            }
                                        } else {
                                            // No sacrifice target — cannot pay Bargain; reset flag.
                                            self.game.set_bargain_paid_logged(card_id, false);
                                        }
                                    }
                                }

                                // Get valid targets BEFORE calling cast_spell_8_step
                                // (we can't borrow controller inside the closure)
                                // Note: For modal spells, this runs AFTER mode selection
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                // Compute the (min, max) target-count bounds for
                                // this spell (CR 601.2c). Fixed single-target
                                // spells are (1, 1); a DivideEvenly X-spell
                                // (Fireball) is (0, num_valid).
                                let (min_targets, max_targets) =
                                    self.game.target_count_bounds_for_spell(card_id, valid_targets.len());

                                // Ask controller to choose targets. Auto-select
                                // ONLY when the count is forced (no real decision):
                                //   - no valid targets, or
                                //   - the bounds pin every legal target
                                //     (min == max == num_valid), or
                                //   - the trivial single fixed target
                                //     (min == max == 1 && num_valid == 1).
                                // Otherwise the count and/or which targets is a
                                // genuine choice and must route through the
                                // controller so it is logged + round-trips on the
                                // network. Use SmallVec for targets (avoids heap
                                // allocation for the common 0-2 target case).
                                let num_valid = valid_targets.len();
                                let forced_all = min_targets == max_targets && max_targets == num_valid;
                                let trivial_single = min_targets == max_targets && max_targets == 1 && num_valid == 1;
                                let chosen_targets_vec: SmallVec<[CardId; 2]> = if num_valid == 0 {
                                    // No targets needed - spell has no targeting effects
                                    SmallVec::new()
                                } else if forced_all || trivial_single {
                                    // Count + identity are forced - auto-select without
                                    // calling the controller. Not a choice, so don't
                                    // log a ChoicePoint.
                                    valid_targets.iter().copied().take(max_targets).collect()
                                } else {
                                    // Real choice (which targets and/or how many) -
                                    // ask the controller. Capture log size BEFORE
                                    // asking (before the controller logs its choice).
                                    let prior_log_size = self.game.logger.log_count();
                                    let choice = self.choose_targets_with_hook(
                                        controller,
                                        current_priority,
                                        card_id,
                                        &valid_targets,
                                        min_targets,
                                        max_targets,
                                    );
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);

                                    // Log this choice point for snapshot/replay
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                    chosen_targets.into_iter().collect()
                                };

                                // Log target selection to gamelog (if any targets were chosen)
                                if !chosen_targets_vec.is_empty()
                                    && self.verbosity >= VerbosityLevel::Normal
                                    && !self.replaying
                                {
                                    // Get target names for display. Player-target
                                    // sentinels (Lightning Bolt aimed at a
                                    // player) don't have a real CardId — fall
                                    // back to the player's display name.
                                    let target_names: Vec<String> = chosen_targets_vec
                                        .iter()
                                        .filter_map(|&tid| {
                                            if let Some(pid) = crate::core::player_target_from_sentinel(tid) {
                                                Some(self.game.player_display_name(pid))
                                            } else {
                                                self.game.cards.try_get(tid).map(|c| format!("{} ({})", c.name, tid))
                                            }
                                        })
                                        .collect();
                                    if !target_names.is_empty() {
                                        let message = format!("  → targeting {}", target_names.join(", "));
                                        self.game.logger.gamelog(&message);
                                    }
                                }

                                // Clone SmallVec for closure (which will move it)
                                let targets_for_callback = chosen_targets_vec.clone();

                                // Create targeting callback (FnOnce — no clone needed)
                                let targeting_callback =
                                    move |_game: &GameState, _spell_id: CardId| targets_for_callback;

                                // Pre-compute ManaEngine for mana payment (step 6)
                                // This avoids allocating a new ManaEngine inside cast_spell_8_step
                                self.mana_engine.update_mut(self.game, current_priority);

                                // Cast using 8-step process
                                if let Err(e) = self.game.cast_spell_8_step(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting spell: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    // Treat failed spell cast like passing priority to prevent infinite loops
                                    // This can happen if controller makes invalid choices or mana engine has stale state
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                } else {
                                    // Store targets for this spell (will be used when it resolves)
                                    self.game
                                        .sub_action_scratch
                                        .spell_targets
                                        .push((card_id, chosen_targets_vec));

                                    // Spell is now on the stack - it will resolve later
                                    // when both players pass priority
                                }
                            }
                            crate::core::SpellAbility::ActivateAbility { card_id, ability_index } => {
                                // Activate ability from a permanent
                                // TODO(mtg-70): This should go on the stack for non-mana abilities

                                // Get the card and ability
                                let card_name = self.game.cards.try_get(card_id).map(|c| c.name.clone());
                                let ability = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .ok()
                                    .and_then(|c| c.activated_abilities.get(ability_index).cloned());

                                // On WASM resumption, pending_activation is already set.
                                // Don't log the "activates ability" message again on re-entry.
                                let is_activation_resumption =
                                    self.game.sub_action_scratch.pending_activation.is_some();

                                // Check if we're resuming mid-effects (e.g., after NeedInput from
                                // DiscardCards routing). If so, skip target selection, cost payment,
                                // and already-executed effects. This prevents double-draws when
                                // abilities like Bazaar of Baghdad (draw 2, discard 3) have their
                                // DrawCards effects executed before DiscardCards returns NeedInput.
                                let effect_resume = self.game.sub_action_scratch.pending_activation_effect_idx.take();

                                if let Some(ability) = ability {
                                    // Log "activates ability" only on first entry (not WASM resumption)
                                    if !is_activation_resumption
                                        && self.verbosity >= VerbosityLevel::Normal
                                        && !self.replaying
                                    {
                                        let name = card_name.as_ref().map(|n| n.as_str()).unwrap_or("Unknown");
                                        let desc = ability.description.replace("CARDNAME", name);
                                        let message = format!("{name} activates ability: {desc}");
                                        // Use gamelog for official game action
                                        self.game.logger.gamelog(&message);
                                    }

                                    // Guard against WASM re-entry at spell ability selection.
                                    // If target selection below returns NeedInput, the next
                                    // step_harness() call will see this flag and bypass
                                    // choose_spell_ability_to_play, resuming here instead.
                                    self.game.sub_action_scratch.pending_activation =
                                        Some((current_priority, card_id, ability_index));

                                    // When resuming mid-effects, use saved targets and skip
                                    // target selection + cost payment (already done on first entry).
                                    let chosen_targets_vec: Vec<CardId>;
                                    let effect_start_idx: usize;

                                    if let Some((resume_idx, saved_targets)) = effect_resume {
                                        // WASM effect resumption: skip targets + costs, resume effects
                                        log::debug!(
                                            "[WASM EFFECT RESUME] Resuming ability effects from index {} (saved {} targets)",
                                            resume_idx,
                                            saved_targets.len()
                                        );
                                        chosen_targets_vec = saved_targets;
                                        effect_start_idx = resume_idx;
                                    } else {
                                        effect_start_idx = 0;
                                        // Get valid targets for the ability (before paying costs)
                                        let valid_targets = self
                                            .game
                                            .get_valid_targets_for_ability(card_id, ability_index)
                                            .unwrap_or_else(|_| SmallVec::new());

                                        // Ask controller to choose targets (only if there are valid targets)
                                        chosen_targets_vec = if valid_targets.is_empty() {
                                            // No targets needed - ability has no targeting effects
                                            Vec::new()
                                        } else if valid_targets.len() == 1 {
                                            // Only one valid target - auto-select without calling controller
                                            // This is not a choice, so don't log ChoicePoint
                                            vec![valid_targets[0]]
                                        } else {
                                            // Multiple valid targets - ask controller to choose
                                            // Capture log size BEFORE asking controller (before controller logs its choice)
                                            let prior_log_size = self.game.logger.log_count();
                                            // Single-target activated ability — bounds (1, 1).
                                            let choice = self.choose_targets_with_hook(
                                                controller,
                                                current_priority,
                                                card_id,
                                                &valid_targets,
                                                1,
                                                1,
                                            );
                                            let chosen_targets =
                                                handle_choice_result_break!(choice, self.game, current_priority);

                                            // Log this choice point for snapshot/replay
                                            let replay_choice =
                                                crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                            self.log_choice_point(
                                                current_priority,
                                                Some(replay_choice),
                                                prior_log_size,
                                            );

                                            chosen_targets.into_iter().collect()
                                        };

                                        // Log target selection to gamelog (if any targets were chosen)
                                        if !chosen_targets_vec.is_empty()
                                            && self.verbosity >= VerbosityLevel::Normal
                                            && !self.replaying
                                        {
                                            // Get target names for display
                                            let target_names: Vec<String> = chosen_targets_vec
                                                .iter()
                                                .filter_map(|&tid| {
                                                    self.game
                                                        .cards
                                                        .try_get(tid)
                                                        .map(|c| format!("{} ({})", c.name, tid))
                                                })
                                                .collect();
                                            if !target_names.is_empty() {
                                                let message = format!("  -> targeting {}", target_names.join(", "));
                                                self.game.logger.gamelog(&message);
                                            }
                                        }

                                        // Auto-tap lands for mana costs (if the ability has a mana cost)
                                        // This is the same logic as spell casting (step 6 of cast_spell_8_step)
                                        // SKIP when resuming mid-effects: costs were already paid on first entry.
                                        if effect_start_idx == 0 {
                                            if let Some(mana_cost) = ability.cost.get_mana_cost() {
                                                // Reuse self.mana_engine to avoid allocation on each activated ability
                                                use crate::game::mana_payment::{
                                                    GreedyManaResolver, ManaPaymentResolver, ManaSource,
                                                };

                                                self.mana_engine.update_mut(self.game, current_priority);

                                                // Get ManaSource list from engine (already built with proper production info)
                                                let all_sources = self.mana_engine.all_sources();

                                                // If the ability cost includes tapping this card, we can't use it for mana
                                                // because it will already be tapped as part of paying the activation cost.
                                                // Filter out the source card in this case.
                                                let filtered_sources: smallvec::SmallVec<[ManaSource; 8]>;
                                                let mana_sources: &[ManaSource] = if ability.cost.includes_tap() {
                                                    filtered_sources = all_sources
                                                        .iter()
                                                        .filter(|s| s.card_id != card_id)
                                                        .cloned()
                                                        .collect();
                                                    &filtered_sources
                                                } else {
                                                    all_sources
                                                };

                                                // Use GreedyManaResolver to compute proper tap order
                                                // Reuse sources_to_tap_buffer to avoid allocation
                                                let resolver = GreedyManaResolver::new();
                                                self.sources_to_tap_buffer.clear();
                                                resolver.compute_tap_order(
                                                    mana_cost,
                                                    mana_sources,
                                                    &mut self.sources_to_tap_buffer,
                                                );

                                                // Track remaining cost as hint for each land tap
                                                // This ensures dual lands produce the right color based on what's still needed
                                                let mut remaining_hint = *mana_cost;

                                                // Tap lands to add mana to pool
                                                for &source_id in &self.sources_to_tap_buffer {
                                                    if let Err(e) = self.game.tap_for_mana_and_update_hint(
                                                        current_priority,
                                                        source_id,
                                                        &mut remaining_hint,
                                                    ) {
                                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                            eprintln!("    Failed to tap land for mana: {e}");
                                                        }
                                                        // Continue to next source - partial payment might still work
                                                    }
                                                }
                                            }

                                            // Pay costs
                                            if let Err(e) =
                                                self.game.pay_ability_cost(current_priority, card_id, &ability.cost)
                                            {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    eprintln!("    Failed to pay cost: {e}");
                                                }
                                                log::warn!(
                                                    "pay_ability_cost failed for {:?} ability {} (player {:?}): {}",
                                                    card_id,
                                                    ability.description,
                                                    current_priority,
                                                    e
                                                );
                                                // Clear pending_activation — ability failed, no resumption needed
                                                self.game.sub_action_scratch.pending_activation = None;
                                                self.game.sub_action_scratch.pending_activation_effect_idx = None;
                                                // Treat failed ability activation like passing priority to prevent infinite loops
                                                consecutive_passes += 1;
                                                self.game.turn.consecutive_passes = consecutive_passes;
                                                current_priority = if current_priority == active_player {
                                                    non_active_player
                                                } else {
                                                    active_player
                                                };
                                                self.game.turn.priority_player = Some(current_priority);
                                                continue;
                                            }
                                        } // end if effect_start_idx == 0 (skip costs on effect resumption)
                                    } // end else (not resuming from effect_resume)

                                    // Set transient controller context so execute_counter_spell can
                                    // detect opponent-countered creature spells (Summoning Trap).
                                    // Cleared below after the effects loop completes.
                                    self.game.current_spell_controller = Some(current_priority);

                                    // Execute effects immediately (not on the stack)
                                    // TODO(mtg-70): Put non-mana abilities on the stack
                                    // Use enumerate to track index for WASM effect resumption.
                                    // When resuming from a NeedInput mid-effects, skip effects
                                    // that were already executed on the first entry.
                                    for (effect_idx, effect) in ability.effects.iter().enumerate() {
                                        if effect_idx < effect_start_idx {
                                            continue; // Skip already-executed effects on resumption
                                        }

                                        // B3 fix (mtg-910): ModalChoice activated abilities (e.g. Umezawa's
                                        // Jitte Charm — "remove a charge counter: choose +2/+2, -1/-1, or +2
                                        // life") were falling through to execute_effect which no-ops them.
                                        // Handle them here by routing mode selection through the controller,
                                        // then executing the chosen mode's sub-effect with placeholder
                                        // resolution. MTG CR 601.2b: mode choice happens before targeting;
                                        // for activated abilities we do it during resolution since targeting
                                        // was collected above across all modes.
                                        if let crate::core::Effect::ModalChoice {
                                            modes,
                                            num_to_choose,
                                            min_to_choose,
                                            can_repeat_modes,
                                        } = effect
                                        {
                                            let mode_descriptions: Vec<String> =
                                                modes.iter().map(|m| m.description.clone()).collect();
                                            let n_choose = *num_to_choose as usize;
                                            let n_min = *min_to_choose as usize;
                                            let can_repeat = *can_repeat_modes;

                                            let prior_log_size = self.game.logger.log_count();
                                            let choice = self.choose_modes_with_hook(
                                                controller,
                                                current_priority,
                                                card_id,
                                                &mode_descriptions,
                                                n_choose,
                                                n_min,
                                                can_repeat,
                                            );
                                            let selected_modes =
                                                handle_choice_result_break!(choice, self.game, current_priority);

                                            let replay_choice = crate::game::ReplayChoice::Modes(
                                                selected_modes.iter().copied().collect(),
                                            );
                                            self.log_choice_point(
                                                current_priority,
                                                Some(replay_choice),
                                                prior_log_size,
                                            );

                                            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                for &idx in &selected_modes {
                                                    if let Some(mode) = modes.get(idx) {
                                                        self.game
                                                            .logger
                                                            .gamelog(&format!("  Mode chosen: {}", mode.description));
                                                    }
                                                }
                                            }

                                            // Execute each selected mode's sub-effect with placeholder
                                            // resolution mirroring the main dispatch below.
                                            for &mode_idx in &selected_modes {
                                                if let Some(mode) = modes.get(mode_idx) {
                                                    let sub = mode.effect.as_ref();
                                                    // Resolve the most common placeholders in mode sub-effects.
                                                    let resolved_sub = match sub {
                                                        crate::core::Effect::GainLife { player, amount }
                                                            if player.is_placeholder() =>
                                                        {
                                                            crate::core::Effect::GainLife {
                                                                player: current_priority,
                                                                amount: *amount,
                                                            }
                                                        }
                                                        crate::core::Effect::PumpCreature {
                                                            target,
                                                            power_bonus,
                                                            toughness_bonus,
                                                            keywords_granted,
                                                        } if target.is_placeholder()
                                                            && !chosen_targets_vec.is_empty() =>
                                                        {
                                                            crate::core::Effect::PumpCreature {
                                                                target: chosen_targets_vec[0],
                                                                power_bonus: *power_bonus,
                                                                toughness_bonus: *toughness_bonus,
                                                                keywords_granted: keywords_granted.clone(),
                                                            }
                                                        }
                                                        crate::core::Effect::PumpCreature {
                                                            target,
                                                            power_bonus,
                                                            toughness_bonus,
                                                            keywords_granted,
                                                        } if target.is_placeholder() => {
                                                            // No chosen target — try the equipped creature
                                                            // (Jitte JittePump: Defined$ Equipped). Fall
                                                            // back to source card if not equipped.
                                                            let pump_target = self
                                                                .game
                                                                .cards
                                                                .try_get(card_id)
                                                                .and_then(|c| c.attached_to)
                                                                .unwrap_or(card_id);
                                                            crate::core::Effect::PumpCreature {
                                                                target: pump_target,
                                                                power_bonus: *power_bonus,
                                                                toughness_bonus: *toughness_bonus,
                                                                keywords_granted: keywords_granted.clone(),
                                                            }
                                                        }
                                                        _ => sub.clone(),
                                                    };
                                                    if let Err(e) = self.game.execute_effect(&resolved_sub) {
                                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                            eprintln!("    Failed to execute modal sub-effect: {e}");
                                                        }
                                                    }
                                                }
                                            }
                                            continue; // skip normal execute_effect for this effect slot
                                        }

                                        // Fix placeholder player IDs and targets for effects
                                        let fixed_effect = match effect {
                                            crate::core::Effect::AddMana {
                                                player,
                                                mana,
                                                produces_chosen_color,
                                                amount_var,
                                            } if player.is_placeholder() => {
                                                // Replace placeholder with current player
                                                crate::core::Effect::AddMana {
                                                    player: current_priority,
                                                    mana: *mana,
                                                    produces_chosen_color: *produces_chosen_color,
                                                    amount_var: amount_var.clone(),
                                                }
                                            }
                                            crate::core::Effect::GainLife { player, amount }
                                                if player.is_placeholder() =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::GainLife {
                                                    player: current_priority,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::DrawCards { player, count }
                                                if player.is_placeholder() =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::DrawCards {
                                                    player: current_priority,
                                                    count: *count,
                                                }
                                            }
                                            // Dynamic life gain from an activated ability (Diamond
                                            // Valley: gain life = sacrificed creature's toughness).
                                            // The recipient placeholder => the ability's controller;
                                            // the SacrificedToughness reference => the creature just
                                            // sacrificed to pay the cost (recorded in
                                            // sub_action_scratch during cost payment, CR 608.2g LKI).
                                            crate::core::Effect::GainLifeDynamic {
                                                player,
                                                amount,
                                                reference,
                                            } => {
                                                let resolved_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let resolved_reference = if reference.is_placeholder()
                                                    && matches!(amount, crate::core::DynamicAmount::SacrificedToughness)
                                                {
                                                    self.game
                                                        .sub_action_scratch
                                                        .sacrificed_for_cost
                                                        .unwrap_or(*reference)
                                                } else {
                                                    *reference
                                                };
                                                crate::core::Effect::GainLifeDynamic {
                                                    player: resolved_player,
                                                    amount: amount.clone(),
                                                    reference: resolved_reference,
                                                }
                                            }
                                            // Circle of Protection: resolve the protected player to
                                            // the ability's controller and the chosen source to the
                                            // selected red (etc.) permanent/spell. (CR 615.1)
                                            crate::core::Effect::PreventDamageFromSource { color, source, .. }
                                                if source.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::PreventDamageFromSource {
                                                    protected: current_priority,
                                                    color: *color,
                                                    source: chosen_targets_vec[0],
                                                }
                                            }
                                            // Replace placeholder targets with chosen targets
                                            crate::core::Effect::DestroyPermanent {
                                                target,
                                                restriction,
                                                no_regenerate,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::DestroyPermanent {
                                                    target: chosen_targets_vec[0],
                                                    restriction: restriction.clone(),
                                                    no_regenerate: *no_regenerate,
                                                }
                                            }
                                            // Defined$ Self: destroy the source card itself
                                            crate::core::Effect::DestroyPermanent {
                                                target,
                                                restriction,
                                                no_regenerate,
                                            } if target.is_self_target() => crate::core::Effect::DestroyPermanent {
                                                target: card_id,
                                                restriction: restriction.clone(),
                                                no_regenerate: *no_regenerate,
                                            },
                                            crate::core::Effect::TapPermanent { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::TapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::UntapPermanent { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::UntapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                                keywords_granted,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: chosen_targets_vec[0],
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                    keywords_granted: keywords_granted.clone(),
                                                }
                                            }
                                            // Self-targeting pump: "This creature gets +X/+Y"
                                            // When no targets were chosen (ability doesn't target), use source card
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                                keywords_granted,
                                            } if target.is_placeholder() && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: card_id, // Target self (the source of the ability)
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                    keywords_granted: keywords_granted.clone(),
                                                }
                                            }
                                            // Self-targeting PutCounter: "Put counter on this creature"
                                            // When Defined$ Self is used, no targets are chosen - use source card
                                            crate::core::Effect::PutCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.is_placeholder() && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PutCounter {
                                                    target: card_id, // Target self (the source of the ability)
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            // Targeted PutCounter: "Put counter on target creature"
                                            crate::core::Effect::PutCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PutCounter {
                                                    target: chosen_targets_vec[0],
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::RemoveCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::RemoveCounter {
                                                    target: chosen_targets_vec[0],
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            // Self-targeting SetBasePowerToughness (Animate): "This creature has base P/T X/Y"
                                            // When Defined$ Self is used, no targets are chosen - use source card
                                            crate::core::Effect::SetBasePowerToughness {
                                                target,
                                                power,
                                                toughness,
                                                keywords_granted,
                                                types_added,
                                                subtypes_added,
                                                remove_creature_subtypes,
                                            } if target.is_placeholder() && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::SetBasePowerToughness {
                                                    target: card_id, // Target self (the source of the ability)
                                                    power: *power,
                                                    toughness: *toughness,
                                                    keywords_granted: keywords_granted.clone(),
                                                    types_added: types_added.clone(),
                                                    subtypes_added: subtypes_added.clone(),
                                                    remove_creature_subtypes: *remove_creature_subtypes,
                                                }
                                            }
                                            // Targeted SetBasePowerToughness: "Target creature has base P/T X/Y"
                                            crate::core::Effect::SetBasePowerToughness {
                                                target,
                                                power,
                                                toughness,
                                                keywords_granted,
                                                types_added,
                                                subtypes_added,
                                                remove_creature_subtypes,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::SetBasePowerToughness {
                                                    target: chosen_targets_vec[0],
                                                    power: *power,
                                                    toughness: *toughness,
                                                    keywords_granted: keywords_granted.clone(),
                                                    types_added: types_added.clone(),
                                                    subtypes_added: subtypes_added.clone(),
                                                    remove_creature_subtypes: *remove_creature_subtypes,
                                                }
                                            }
                                            crate::core::Effect::AttachEquipment {
                                                source_equipment,
                                                target_creature,
                                            } if target_creature.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::AttachEquipment {
                                                    source_equipment: *source_equipment,
                                                    target_creature: chosen_targets_vec[0],
                                                }
                                            }
                                            // GrantCantBeBlocked: "Target creature can't be blocked this turn"
                                            // Used by Deserter's Disciple
                                            crate::core::Effect::GrantCantBeBlocked { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::GrantCantBeBlocked {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            // Self-targeting Regenerate: "Regenerate CARDNAME"
                                            crate::core::Effect::Regenerate { target }
                                                if target.is_placeholder() && chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::Regenerate {
                                                    target: card_id, // Target self
                                                }
                                            }
                                            // Targeted Regenerate: "Regenerate target creature"
                                            crate::core::Effect::Regenerate { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::Regenerate {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            // Replace placeholder targets in DealDamage effects
                                            // This is needed for ping abilities like Prodigal Sorcerer
                                            crate::core::Effect::DealDamage {
                                                target: crate::core::TargetRef::None,
                                                amount,
                                            } if !chosen_targets_vec.is_empty() => crate::core::Effect::DealDamage {
                                                target: crate::core::TargetRef::Permanent(chosen_targets_vec[0]),
                                                amount: *amount,
                                            },
                                            // SearchLibrary needs special handling - ask controller to choose card
                                            crate::core::Effect::SearchLibrary {
                                                player,
                                                card_type_filter,
                                                destination,
                                                enters_tapped,
                                                shuffle,
                                            } if player.is_placeholder() => {
                                                // Handle library search with controller input
                                                let search_player = current_priority;

                                                // Get library and filter for matching cards
                                                let library_cards = self
                                                    .game
                                                    .player_zones
                                                    .iter()
                                                    .find(|(id, _)| *id == search_player)
                                                    .map(|(_, zones)| zones.library.cards.clone())
                                                    .unwrap_or_default();

                                                // Sync network state to process pending CardRevealed messages
                                                // before filtering library cards (mtg-253 fix)
                                                self.sync_to_action();

                                                // Filter cards by type
                                                let mut valid_cards = Vec::new();
                                                for &card_id in &library_cards {
                                                    if let Some(card) = self.game.cards.try_get(card_id) {
                                                        if crate::game::state::GameState::card_matches_search_filter(
                                                            card,
                                                            card_type_filter,
                                                        ) {
                                                            valid_cards.push(card_id);
                                                        }
                                                    }
                                                }

                                                // Ask controller to choose a card (or decline to find)
                                                let prior_log_size = self.game.logger.log_count();
                                                let choice = self.choose_from_library_with_hook(
                                                    controller,
                                                    current_priority,
                                                    &valid_cards,
                                                );
                                                // Handle NeedInput: save effect index for resumption
                                                let chosen_card_opt = match choice {
                                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                        self.game.sub_action_scratch.pending_activation_effect_idx =
                                                            Some((effect_idx, chosen_targets_vec));
                                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                    }
                                                    other => {
                                                        handle_choice_result_break!(other, self.game, current_priority)
                                                    }
                                                };

                                                // Log the choice for replay — record the
                                                // AUTHORITATIVE fetched CardId, not a positional
                                                // index. On an opponent's shadow the fetched card
                                                // is hidden and absent from `valid_cards`, so an
                                                // index collapsed Some(card)->None and lost the
                                                // fetch under rewind+replay (mtg-728).
                                                let replay_choice =
                                                    crate::game::ReplayChoice::LibrarySearch(chosen_card_opt);
                                                self.log_choice_point(
                                                    current_priority,
                                                    Some(replay_choice),
                                                    prior_log_size,
                                                );

                                                // If a card was chosen, move it to destination
                                                if let Some(chosen_card) = chosen_card_opt {
                                                    if let Err(e) = self.game.move_card(
                                                        chosen_card,
                                                        crate::zones::Zone::Library,
                                                        *destination,
                                                        search_player,
                                                    ) {
                                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                            eprintln!("    Failed to move chosen card: {e}");
                                                        }
                                                    }

                                                    // If destination is battlefield and enters_tapped is true, tap the card
                                                    if *destination == crate::zones::Zone::Battlefield && *enters_tapped
                                                    {
                                                        let _ = self.game.tap_permanent(chosen_card);
                                                    }
                                                }

                                                // Shuffle library if required
                                                if *shuffle {
                                                    self.game.shuffle_library(search_player);
                                                }

                                                // Skip execute_effect for SearchLibrary - we handled it above
                                                continue;
                                            }
                                            // Earthbend: Target land becomes 0/0 creature with haste and N +1/+1 counters
                                            crate::core::Effect::Earthbend { target, num_counters }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::Earthbend {
                                                    target: chosen_targets_vec[0],
                                                    num_counters: *num_counters,
                                                }
                                            }
                                            // DiscardCards with choice (count != u8::MAX): Route through
                                            // controller for network-safe discard decisions (mtg-272).
                                            //
                                            // Without this, choose_card_to_discard() runs independently
                                            // on both server and shadow. The shadow can't see cards drawn
                                            // in the same ability (e.g., Bazaar of Baghdad: draw 2, discard 3)
                                            // because CardRevealed messages haven't arrived yet.
                                            crate::core::Effect::DiscardCards {
                                                player,
                                                count,
                                                remember_discarded,
                                                ..
                                            } if *count != u8::MAX => {
                                                let discard_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let discard_count = *count as usize;
                                                let remember = *remember_discarded;

                                                // push_reveals is a no-op under the synchronous
                                                // server/client transport (reveals ride the ChoiceRequest
                                                // buffer, not a reveal_pusher); kept for parity with the
                                                // other reveal sites. The shadow's hand MEMBERSHIP (which
                                                // CardIds) is already correct here from its own draw — only
                                                // the drawn cards' IDENTITIES are still unmaterialised, and
                                                // those are filled by the prepare→sync below.
                                                self.push_reveals(discard_player);
                                                if let Some(opp) = self.game.get_other_player_id(discard_player) {
                                                    self.push_reveals(opp);
                                                }

                                                // Get current hand
                                                let hand: SmallVec<[CardId; 8]> = self
                                                    .game
                                                    .get_player_zones(discard_player)
                                                    .map(|zones| zones.hand.cards.iter().copied().collect())
                                                    .unwrap_or_default();

                                                // Clamp count to actual hand size
                                                let actual_count = discard_count.min(hand.len());
                                                if actual_count == 0 {
                                                    // Empty hand (e.g. a forced discard vs an empty-handed
                                                    // player): the SERVER computes actual_count==0 and sends
                                                    // NO discard ChoiceRequest. We MUST bail BEFORE the
                                                    // blocking prepare below, preserving the invariant
                                                    // "a network block happens iff a request will be sent"
                                                    // (mtg-768 BLOCKER-1): prepare_for_priority_choice()
                                                    // blocks with no timeout and take_local_choice is a blind
                                                    // FIFO pop, so blocking here would hang the client forever
                                                    // OR pop the NEXT request → answer request N+1 to choice N
                                                    // → off-by-one FATAL desync.
                                                    continue;
                                                }

                                                // mtg-768: a discard choice that IMMEDIATELY follows
                                                // in-resolution draws (e.g. Bazaar of Baghdad "draw two,
                                                // then discard three") must RECEIVE the server's discard
                                                // ChoiceRequest BEFORE syncing the shadow: the just-drawn
                                                // cards' reveals ride inside THAT request's catch-up buffer
                                                // (assemble_choice_buffer), so they only land in the
                                                // state-sync log once the request arrives. This mirrors the
                                                // proven priority-loop order (see the "NETWORK SYNC PROTOCOL"
                                                // block earlier in this file): prepare (block on the choice
                                                // MVar) → sync_to_action → decide. The previous order synced
                                                // FIRST, so on a network client the drawn cards stayed
                                                // unmaterialised in the shadow and the heuristic discarded
                                                // the WRONG cards vs the server's full-state decision — an
                                                // information-independence desync (docs/NETWORK_ARCHITECTURE.md:
                                                // "Desync is ALWAYS Fatal"). prepare_for_priority_choice() is a
                                                // no-op default for non-network controllers AND for the
                                                // authoritative server (whose NetworkController already holds
                                                // the drawn cards in full state and merely relays the client's
                                                // chosen indices); only the client's shadow controllers
                                                // (NetworkLocalController / RemoteController) block to pre-cache
                                                // the request (idempotent — choose_discard_with_hook reuses it).
                                                let _ = controller.prepare_for_priority_choice();
                                                self.sync_to_action();

                                                // Route through controller protocol.
                                                // Capture log size BEFORE asking controller so the
                                                // ChoicePoint we log below can rewind cleanly to the
                                                // pre-choice state. Without logging this ChoicePoint,
                                                // rewind/replay loops infinitely on cards like
                                                // Bazaar of Baghdad ("draw 2, discard 3"): the user's
                                                // discard pick is never recorded, so on replay the
                                                // ReplayController has no Discard entry to consume,
                                                // delegates back to the (empty) human controller,
                                                // and we re-prompt for the same discard forever.
                                                let prior_log_size = self.game.logger.log_count();
                                                let choice = self.choose_discard_with_hook(
                                                    controller,
                                                    discard_player,
                                                    &hand,
                                                    actual_count,
                                                );
                                                // Handle NeedInput specially: save effect index for
                                                // WASM resumption so we don't re-execute prior effects
                                                // (especially DrawCards) on re-entry.
                                                let cards_to_discard = match choice {
                                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                        // Save effect index and targets for resumption
                                                        self.game.sub_action_scratch.pending_activation_effect_idx =
                                                            Some((effect_idx, chosen_targets_vec));
                                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                    }
                                                    other => {
                                                        handle_choice_result_break!(other, self.game, current_priority)
                                                    }
                                                };

                                                // Log the discard choice for snapshot/replay so a
                                                // later rewind can replay it deterministically.
                                                let replay_choice =
                                                    crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                                                self.log_choice_point(
                                                    discard_player,
                                                    Some(replay_choice),
                                                    prior_log_size,
                                                );

                                                for card_id in cards_to_discard {
                                                    if remember {
                                                        self.game.remembered_cards.push(card_id);
                                                    }
                                                    // Cause = the player resolving this activated
                                                    // ability (the priority holder). A Discarded
                                                    // self-trigger (Psychic Purge) attributes the
                                                    // discard to that ability's controller; a player
                                                    // discarding their own card (cause == owner) does
                                                    // not fire the opponent-only punisher.
                                                    if let Err(e) = self.game.discard_card(
                                                        discard_player,
                                                        card_id,
                                                        Some(current_priority),
                                                    ) {
                                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                            eprintln!("    Failed to discard: {e}");
                                                        }
                                                    }
                                                }

                                                // Skip execute_effect — handled above
                                                continue;
                                            }
                                            // DiscardCards with placeholder player (full hand, u8::MAX):
                                            // Just fix the placeholder, let execute_effect handle it
                                            // (no choice needed — discards everything deterministically)
                                            crate::core::Effect::DiscardCards {
                                                player,
                                                count,
                                                remember_discarded,
                                                ..
                                            } if player.is_placeholder() => crate::core::Effect::DiscardCards {
                                                player: current_priority,
                                                count: *count,
                                                remember_discarded: *remember_discarded,
                                                optional: false,
                                                remember_discarding_players: false,
                                            },
                                            // Loot (discard then draw): Route discard through controller
                                            // for network-safe decisions (mtg-272).
                                            crate::core::Effect::Loot {
                                                player,
                                                discard_count,
                                                draw_count,
                                            } => {
                                                let loot_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let d_count = *discard_count as usize;
                                                let dr_count = *draw_count;

                                                // Sync reveals before looting
                                                self.sync_to_action();

                                                // Discard phase: route through controller
                                                if d_count > 0 {
                                                    let hand: SmallVec<[CardId; 8]> = self
                                                        .game
                                                        .get_player_zones(loot_player)
                                                        .map(|zones| zones.hand.cards.iter().copied().collect())
                                                        .unwrap_or_default();

                                                    let actual_count = d_count.min(hand.len());
                                                    if actual_count > 0 {
                                                        // Capture log size BEFORE the choice so the
                                                        // ChoicePoint we log can rewind cleanly.
                                                        // See DiscardCards branch above for why this
                                                        // is mandatory (Bazaar of Baghdad rewind loop).
                                                        let prior_log_size = self.game.logger.log_count();
                                                        let choice = self.choose_discard_with_hook(
                                                            controller,
                                                            loot_player,
                                                            &hand,
                                                            actual_count,
                                                        );
                                                        // Handle NeedInput: save effect index for resumption
                                                        let cards_to_discard = match choice {
                                                            crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                                self.game
                                                                    .sub_action_scratch
                                                                    .pending_activation_effect_idx =
                                                                    Some((effect_idx, chosen_targets_vec));
                                                                return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                            }
                                                            other => {
                                                                handle_choice_result_break!(
                                                                    other,
                                                                    self.game,
                                                                    current_priority
                                                                )
                                                            }
                                                        };

                                                        // Log the discard choice for replay.
                                                        let replay_choice = crate::game::ReplayChoice::Discard(
                                                            cards_to_discard.clone(),
                                                        );
                                                        self.log_choice_point(
                                                            loot_player,
                                                            Some(replay_choice),
                                                            prior_log_size,
                                                        );

                                                        for card_id in cards_to_discard {
                                                            // Loot's discard is part of the activated
                                                            // ability; attribute the cause to its
                                                            // controller (the priority holder). Self-
                                                            // looting (cause == owner) does not fire
                                                            // the opponent-gated Discarded trigger.
                                                            if let Err(e) = self.game.discard_card(
                                                                loot_player,
                                                                card_id,
                                                                Some(current_priority),
                                                            ) {
                                                                if self.verbosity >= VerbosityLevel::Normal
                                                                    && !self.replaying
                                                                {
                                                                    eprintln!("    Failed to discard: {e}");
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                // Draw phase: execute directly
                                                for _ in 0..dr_count {
                                                    match self.game.draw_card(loot_player) {
                                                        Ok((_, draw_num)) => {
                                                            if let Err(e) = self
                                                                .game
                                                                .check_card_drawn_triggers(loot_player, draw_num)
                                                            {
                                                                if self.verbosity >= VerbosityLevel::Normal
                                                                    && !self.replaying
                                                                {
                                                                    eprintln!("    Failed draw trigger: {e}");
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            if self.verbosity >= VerbosityLevel::Normal
                                                                && !self.replaying
                                                            {
                                                                eprintln!("    Failed to draw: {e}");
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }

                                                // Skip execute_effect — handled above
                                                continue;
                                            }
                                            // Scry: snapshot → controller → apply (mirror of
                                            // the spell-resolution branch; see
                                            // resolve_top_spell_with_discard_hook above).
                                            crate::core::Effect::Scry {
                                                player,
                                                count,
                                                only_if_bargained,
                                            } => {
                                                // Condition$ Bargain: skip scry unless source
                                                // card was bargained (CR 702.162).
                                                if *only_if_bargained {
                                                    let is_bargained = self
                                                        .game
                                                        .cards
                                                        .try_get(card_id)
                                                        .is_some_and(|c| c.bargain_paid);
                                                    if !is_bargained {
                                                        continue;
                                                    }
                                                }
                                                let scry_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let revealed = self.game.scry_snapshot_top_n(scry_player, *count);
                                                if revealed.is_empty() {
                                                    continue;
                                                }
                                                self.push_reveals(scry_player);
                                                if let Some(opp) = self.game.get_other_player_id(scry_player) {
                                                    self.push_reveals(opp);
                                                }
                                                self.sync_to_action();

                                                let prior_log_size = self.game.logger.log_count();
                                                let view = crate::game::GameStateView::new(self.game, scry_player);
                                                let choice = controller.choose_scry_order(&view, &revealed);
                                                let decision = match choice {
                                                    crate::game::controller::ChoiceResult::Ok(d) => d,
                                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                        self.game.sub_action_scratch.pending_activation_effect_idx =
                                                            Some((effect_idx, chosen_targets_vec));
                                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                    }
                                                    other => {
                                                        // Treat ExitGame / Error / Undo via the
                                                        // standard helper: it returns from the step
                                                        // handler so the game loop re-evaluates.
                                                        handle_choice_result_break!(other, self.game, current_priority)
                                                    }
                                                };

                                                let replay_choice = crate::game::ReplayChoice::Scry {
                                                    top: decision.top.clone(),
                                                    bottom: decision.bottom.clone(),
                                                };
                                                self.log_choice_point(scry_player, Some(replay_choice), prior_log_size);

                                                if let Err(e) =
                                                    self.game.scry_apply_decision(scry_player, &revealed, &decision)
                                                {
                                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                        eprintln!("    Failed to apply scry decision: {e}");
                                                    }
                                                }
                                                continue;
                                            }
                                            // Surveil: same dispatch shape as scry; cards go
                                            // to graveyard instead of bottom of library.
                                            crate::core::Effect::Surveil { player, count } => {
                                                let surveil_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let revealed = self.game.surveil_snapshot_top_n(surveil_player, *count);
                                                if revealed.is_empty() {
                                                    continue;
                                                }
                                                self.push_reveals(surveil_player);
                                                if let Some(opp) = self.game.get_other_player_id(surveil_player) {
                                                    self.push_reveals(opp);
                                                }
                                                self.sync_to_action();

                                                let prior_log_size = self.game.logger.log_count();
                                                let view = crate::game::GameStateView::new(self.game, surveil_player);
                                                let choice = controller.choose_surveil(&view, &revealed);
                                                let decision = match choice {
                                                    crate::game::controller::ChoiceResult::Ok(d) => d,
                                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                        self.game.sub_action_scratch.pending_activation_effect_idx =
                                                            Some((effect_idx, chosen_targets_vec));
                                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                    }
                                                    other => {
                                                        handle_choice_result_break!(other, self.game, current_priority)
                                                    }
                                                };

                                                let replay_choice = crate::game::ReplayChoice::Surveil {
                                                    top: decision.top.clone(),
                                                    graveyard: decision.graveyard.clone(),
                                                };
                                                self.log_choice_point(
                                                    surveil_player,
                                                    Some(replay_choice),
                                                    prior_log_size,
                                                );

                                                if let Err(e) = self.game.surveil_apply_decision(
                                                    surveil_player,
                                                    &revealed,
                                                    &decision,
                                                ) {
                                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                        eprintln!("    Failed to apply surveil decision: {e}");
                                                    }
                                                }
                                                continue;
                                            }
                                            // MoveSelfBetweenZones with self_target source: patch to
                                            // the actual card_id (e.g. Earthquake Dragon's graveyard→hand
                                            // activated ability; AB$ ChangeZone | Origin$ Graveyard |
                                            // Destination$ Hand | ActivationZone$ Graveyard).
                                            crate::core::Effect::MoveSelfBetweenZones {
                                                source,
                                                origin,
                                                destination,
                                            } if source.is_self_target() => crate::core::Effect::MoveSelfBetweenZones {
                                                source: card_id,
                                                origin: *origin,
                                                destination: *destination,
                                            },
                                            // Activated GainControl (Aladdin / Old Man of the Sea):
                                            // resolve the placeholder target to the chosen permanent,
                                            // the new controller to the activating player, and the
                                            // source to this card (for a WhileControlSource duration).
                                            // Without this arm the target/new_controller stayed
                                            // placeholders and execute_effect bailed (mtg-713 B1).
                                            crate::core::Effect::GainControl {
                                                target,
                                                untap,
                                                duration,
                                                restriction,
                                                ..
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::GainControl {
                                                    target: chosen_targets_vec[0],
                                                    new_controller: current_priority,
                                                    untap: *untap,
                                                    duration: *duration,
                                                    restriction: restriction.clone(),
                                                    source: Some(card_id),
                                                }
                                            }
                                            // SetLife targeting a player (Sorin Markov -3: "Target
                                            // opponent's life total becomes 10"). The chosen target
                                            // is a player sentinel; decode it to a PlayerId.
                                            crate::core::Effect::SetLife { player, amount }
                                                if player.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                let resolved =
                                                    crate::core::player_target_from_sentinel(chosen_targets_vec[0])
                                                        .unwrap_or(current_priority);
                                                crate::core::Effect::SetLife {
                                                    player: resolved,
                                                    amount: *amount,
                                                }
                                            }
                                            // CreateTokenWithStoredPt (Phyrexian Processor): resolve
                                            // the source_card placeholder to the activating card so
                                            // execute_create_token_with_stored_pt can read stored_int.
                                            crate::core::Effect::CreateTokenWithStoredPt {
                                                source_card,
                                                controller,
                                                token_script,
                                            } => {
                                                let resolved_source = if source_card.is_placeholder() {
                                                    card_id
                                                } else {
                                                    *source_card
                                                };
                                                let resolved_controller = if controller.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *controller
                                                };
                                                crate::core::Effect::CreateTokenWithStoredPt {
                                                    source_card: resolved_source,
                                                    controller: resolved_controller,
                                                    token_script: token_script.clone(),
                                                }
                                            }
                                            _ => effect.clone(),
                                        };

                                        if let Err(e) = self.game.execute_effect(&fixed_effect) {
                                            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                eprintln!("    Failed to execute effect: {e}");
                                            }
                                        }
                                    }

                                    // Clear transient spell-controller context (set above for
                                    // Summoning Trap / execute_counter_spell detection).
                                    self.game.current_spell_controller = None;

                                    // Clear the cost-payment sacrifice scratch now that this
                                    // ability's effects have run. Keeps it provably None at
                                    // every choice / serialize boundary (Diamond Valley).
                                    self.game.sub_action_scratch.sacrificed_for_cost = None;

                                    // Mark exhaust ability as exhausted (can only be activated once per game)
                                    if ability.exhaust {
                                        if let Ok(card_mut) = self.game.cards.get_mut(card_id) {
                                            if !card_mut.exhausted_abilities.contains(&ability_index) {
                                                card_mut.exhausted_abilities.push(ability_index);
                                            }
                                        }
                                    }

                                    // Push reveals after ability effects for network mode (server-side)
                                    // Abilities can draw cards, and clients need the card IDs before drawing
                                    self.push_reveals(current_priority);
                                    if let Some(opponent) = self.game.get_other_player_id(current_priority) {
                                        self.push_reveals(opponent);
                                    }

                                    // Check state-based actions after effects resolve (MTG Rules 704.3)
                                    // This handles lethal damage from damage-dealing abilities
                                    if let Err(e) = self.game.check_lethal_damage() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check lethal damage: {e}");
                                        }
                                    }
                                    // Check legendary rule (MTG CR 704.5j)
                                    if let Err(e) = self.game.check_legendary_rule() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check legendary rule: {e}");
                                        }
                                    }
                                    // Apply originally-printed-set state-trigger sweeps (City in a Bottle).
                                    if let Err(e) = self.game.check_set_origin_sacrifice() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check set-origin sacrifice: {e}");
                                        }
                                    }
                                    // Check aura attachment (MTG CR 704.5d)
                                    if let Err(e) = self.game.check_aura_attachment() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check aura attachment: {e}");
                                        }
                                    }
                                    // Re-derive control from control-changing Auras (CR 613.2).
                                    if let Err(e) = self.game.recompute_aura_control() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to recompute aura control: {e}");
                                        }
                                    }
                                    // Re-derive source-duration GainControl grants (Aladdin; CR 800.4a).
                                    if let Err(e) = self.game.recompute_source_control() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to recompute source control: {e}");
                                        }
                                    }
                                    // Check equipment attachment (MTG CR 704.5n)
                                    if let Err(e) = self.game.check_equipment_attachment() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check equipment attachment: {e}");
                                        }
                                    }

                                    // Clear pending_activation — ability executed successfully
                                    self.game.sub_action_scratch.pending_activation = None;
                                    self.game.sub_action_scratch.pending_activation_effect_idx = None;
                                } else {
                                    log::warn!(
                                        "ActivateAbility: ability not found for card {:?} '{}' ability_index={} (player {:?})",
                                        card_id, card_name.as_ref().map(|n| n.as_str()).unwrap_or("MISSING"), ability_index, current_priority
                                    );
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        eprintln!("  Ability not found");
                                    }
                                    // Treat as pass to avoid infinite loop
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }
                            }
                            crate::core::SpellAbility::CastFromExile {
                                card_id,
                                alternative_cost,
                                effect_id,
                            } => {
                                // Cast from exile using alternative cost (Airbend, Suspend, etc.)
                                // Uses the generalized 8-step casting process from exile zone.

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    // Generic from-exile cast message — covers
                                    // Airbend, Suspend, and the Adventure creature
                                    // half (CR 715.3d), all of which route through
                                    // the MayPlayFromExile machinery.
                                    let message = format!(
                                        "{} casts {} from exile for {}",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        alternative_cost
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                // Mode selection for modal spells cast from exile (Charm spells)
                                // MTG Rule 601.2b: Modal choice happens BEFORE targeting/payment
                                if let Ok(Some(crate::core::Effect::ModalChoice {
                                    modes,
                                    num_to_choose,
                                    min_to_choose,
                                    can_repeat_modes,
                                })) = self.game.get_modal_choice_info(card_id)
                                {
                                    let valid_modes = self
                                        .game
                                        .get_valid_modes_for_spell(card_id, current_priority)
                                        .unwrap_or_default();
                                    let valid_mode_indices: Vec<usize> = valid_modes
                                        .iter()
                                        .filter(|(_, has_targets)| *has_targets)
                                        .map(|(idx, _)| *idx)
                                        .collect();

                                    if !valid_mode_indices.is_empty() {
                                        let mode_descriptions: Vec<String> = valid_mode_indices
                                            .iter()
                                            .filter_map(|&idx| modes.get(idx).map(|m| m.description.clone()))
                                            .collect();

                                        let prior_log_size = self.game.logger.log_count();
                                        let choice = self.choose_modes_with_hook(
                                            controller,
                                            current_priority,
                                            card_id,
                                            &mode_descriptions,
                                            num_to_choose as usize,
                                            min_to_choose as usize,
                                            can_repeat_modes,
                                        );
                                        let selected_modes =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        let original_indices: Vec<usize> = selected_modes
                                            .iter()
                                            .filter_map(|&idx| valid_mode_indices.get(idx).copied())
                                            .collect();

                                        let replay_choice = crate::game::ReplayChoice::Modes(
                                            original_indices.iter().copied().collect(),
                                        );
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        if let Err(e) = self.game.apply_selected_modes(card_id, &original_indices) {
                                            log::warn!(target: "priority", "Failed to apply modes: {}", e);
                                        }

                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            for &mode_idx in &original_indices {
                                                if let Some(mode) = modes.get(mode_idx) {
                                                    self.game.logger.gamelog(&format!(
                                                        "{} chooses mode: {}",
                                                        self.get_player_name(current_priority),
                                                        mode.description
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }

                                // Target selection (same as CastSpell path)
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                let chosen_targets_vec: SmallVec<[CardId; 2]> = if valid_targets.is_empty() {
                                    SmallVec::new()
                                } else if valid_targets.len() == 1 {
                                    smallvec::smallvec![valid_targets[0]]
                                } else {
                                    let prior_log_size = self.game.logger.log_count();
                                    // Single-target site — bounds (1, 1).
                                    let choice = self.choose_targets_with_hook(
                                        controller,
                                        current_priority,
                                        card_id,
                                        &valid_targets,
                                        1,
                                        1,
                                    );
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);
                                    chosen_targets.into_iter().collect()
                                };

                                let targets_for_callback = chosen_targets_vec.clone();
                                let targeting_callback =
                                    move |_game: &GameState, _spell_id: CardId| targets_for_callback;

                                // Cast using generalized 8-step process from exile with alternative cost
                                self.mana_engine.update_mut(self.game, current_priority);

                                if let Err(e) = self.game.cast_spell_8_step_from(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                    crate::zones::Zone::Exile,
                                    Some(alternative_cost),
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting from exile: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Store targets for resolution
                                if !chosen_targets_vec.is_empty() {
                                    self.game
                                        .sub_action_scratch
                                        .spell_targets
                                        .push((card_id, chosen_targets_vec));
                                }

                                // Remove the persistent effect that granted this cast permission
                                self.game.persistent_effects.remove(effect_id);

                                // Spell is now on the stack - will resolve when both players pass
                            }

                            crate::core::SpellAbility::CastFromCommand { card_id, total_cost } => {
                                // Cast commander from command zone (MTG CR 903.8)
                                // Uses the generalized 8-step casting process from command zone.

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let player_idx = self
                                        .game
                                        .players
                                        .iter()
                                        .position(|p| p.id == current_priority)
                                        .unwrap_or(0);
                                    let tax = self.game.players[player_idx].commander_tax();
                                    let message = if tax > 0 {
                                        format!(
                                            "{} casts {} from command zone for {} (includes {{{}}} commander tax)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            total_cost,
                                            tax
                                        )
                                    } else {
                                        format!(
                                            "{} casts {} from command zone for {}",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            total_cost
                                        )
                                    };
                                    self.game.logger.gamelog(&message);
                                }

                                // Target selection (same as CastSpell path)
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                let chosen_targets_vec: SmallVec<[CardId; 2]> = if valid_targets.is_empty() {
                                    SmallVec::new()
                                } else if valid_targets.len() == 1 {
                                    smallvec::smallvec![valid_targets[0]]
                                } else {
                                    let prior_log_size = self.game.logger.log_count();
                                    // Single-target site — bounds (1, 1).
                                    let choice = self.choose_targets_with_hook(
                                        controller,
                                        current_priority,
                                        card_id,
                                        &valid_targets,
                                        1,
                                        1,
                                    );
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);
                                    chosen_targets.into_iter().collect()
                                };

                                let targets_for_callback = chosen_targets_vec.clone();
                                let targeting_callback =
                                    move |_game: &GameState, _spell_id: CardId| targets_for_callback;

                                // Cast using generalized 8-step process from command zone
                                // total_cost already includes commander tax (computed at ability generation time)
                                self.mana_engine.update_mut(self.game, current_priority);

                                if let Err(e) = self.game.cast_spell_8_step_from(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                    crate::zones::Zone::Command,
                                    Some(total_cost),
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting from command zone: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Store targets for resolution
                                if !chosen_targets_vec.is_empty() {
                                    self.game
                                        .sub_action_scratch
                                        .spell_targets
                                        .push((card_id, chosen_targets_vec));
                                }

                                // Record commander cast for commander tax tracking
                                if let Some(player) = self.game.players.iter_mut().find(|p| p.id == current_priority) {
                                    let old_count = player.commander_cast_count;
                                    player.record_commander_cast();
                                    let prior_log_size = self.game.logger.log_count();
                                    self.game.undo_log.log(
                                        crate::undo::GameAction::SetCommanderCastCount {
                                            player_id: current_priority,
                                            old_value: old_count,
                                            new_value: player.commander_cast_count,
                                        },
                                        prior_log_size,
                                    );
                                }

                                // Spell is now on the stack - will resolve when both players pass
                            }

                            crate::core::SpellAbility::Cycle {
                                card_id,
                                cost,
                                search_type,
                            } => {
                                // Cycling ability from hand
                                // MTG CR 702.29: "Cycling is an activated ability that functions only
                                // while the card with cycling is in a player's hand."

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                let type_str = match &search_type {
                                    Some(st) => format!("{}cycling", st.as_str()),
                                    None => "Cycling".to_string(),
                                };

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let message = format!(
                                        "{} uses {} on {} (cost: {})",
                                        self.get_player_name(current_priority),
                                        type_str,
                                        card_name,
                                        cost
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                // 1. Pay the cycling cost
                                //
                                // Cycling is an activated ability whose cost is just mana — there
                                // is no implicit tap of the cycled card. We must (a) auto-tap
                                // lands to fill the pool, then (b) DEDUCT the cost from the pool
                                // via `pay_ability_cost`. Previously this code only tapped lands
                                // and never deducted, which made cycling effectively free; worse,
                                // it ignored the result of `compute_tap_order`, so when no
                                // untapped sources existed (e.g. after a costly activated ability
                                // emptied the board's lands) the cycling proceeded anyway —
                                // discarding the card without paying — and left the controller
                                // looping on the next "available" spell that it then couldn't
                                // afford either. (Tracked in mtg-417.)
                                self.mana_engine.update_mut(self.game, current_priority);
                                use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                // First check whether the pool already covers the cost (e.g.
                                // floating mana from Dark Ritual). If not, compute the tap order.
                                let pool_can_pay = self
                                    .game
                                    .try_get_player(current_priority)
                                    .map(|p| p.mana_pool.can_pay(&cost))
                                    .unwrap_or(false);

                                let mana_sources = self.mana_engine.all_sources();
                                let resolver = GreedyManaResolver::new();
                                self.sources_to_tap_buffer.clear();
                                let tap_order_ok =
                                    resolver.compute_tap_order(&cost, mana_sources, &mut self.sources_to_tap_buffer);

                                if !pool_can_pay && !tap_order_ok {
                                    // Nothing in pool and no way to tap for it. The action menu
                                    // should not have offered this option — bail out cleanly so
                                    // the controller can pick a different action next iteration.
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game.logger.normal(&format!(
                                            "Failed to activate {} on {}: cannot pay cost {} (no untapped lands or floating mana)",
                                            type_str, card_name, cost
                                        ));
                                    }
                                    log::warn!(
                                        "Cycling activation aborted for card {:?} (player {:?}): unpayable cost {}",
                                        card_id,
                                        current_priority,
                                        cost
                                    );
                                    self.game.sub_action_scratch.pending_activation = None;
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                let mut remaining_hint = cost;
                                let mut tap_ok = true;
                                for &source_id in &self.sources_to_tap_buffer {
                                    if let Err(e) = self.game.tap_for_mana_and_update_hint(
                                        current_priority,
                                        source_id,
                                        &mut remaining_hint,
                                    ) {
                                        tap_ok = false;
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            self.game.logger.normal(&format!("Failed to tap for cycling: {e}"));
                                        }
                                    }
                                }

                                // Now deduct the actual mana cost from the pool. This is the
                                // step that was missing — without it, the cycled mana stayed
                                // floating in the pool and no MTG cost was actually paid.
                                if let Err(e) = self.game.pay_ability_cost(
                                    current_priority,
                                    card_id,
                                    &crate::core::Cost::Mana(cost),
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game
                                            .logger
                                            .normal(&format!("Failed to pay cycling cost for {}: {e}", card_name));
                                    }
                                    log::warn!(
                                        "Cycling pay_ability_cost failed for {:?} (tap_ok={}): {}",
                                        card_id,
                                        tap_ok,
                                        e
                                    );
                                    self.game.sub_action_scratch.pending_activation = None;
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // 2. Discard the card (move from hand to graveyard)
                                let owner = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.owner)
                                    .unwrap_or(current_priority);

                                if let Err(e) = self.game.move_card(
                                    card_id,
                                    crate::zones::Zone::Hand,
                                    crate::zones::Zone::Graveyard,
                                    owner,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game.logger.normal(&format!("Failed to discard for cycling: {e}"));
                                    }
                                }

                                // 3. Perform the cycling effect
                                match search_type {
                                    Some(land_type) => {
                                        // Typecycling: Search library for card with matching type
                                        // For Mountaincycling, search for Mountain
                                        // For Swampcycling, search for Swamp
                                        log::debug!(
                                            "[TYPECYCLING] Entered typecycling branch for land_type={:?}",
                                            land_type
                                        );

                                        // Build search filter for matching land type
                                        let filter = format!("Land.{}", land_type.as_str());
                                        log::debug!("[TYPECYCLING] Filter = '{}'", filter);

                                        // Get library and filter for matching cards
                                        let library_cards = self
                                            .game
                                            .player_zones
                                            .iter()
                                            .find(|(id, _)| *id == current_priority)
                                            .map(|(_, zones)| zones.library.cards.clone())
                                            .unwrap_or_default();
                                        log::debug!("[TYPECYCLING] Library has {} cards", library_cards.len());

                                        // Sync network state to process pending CardRevealed messages
                                        // before filtering library cards (mtg-253 fix)
                                        self.sync_to_action();

                                        // Filter cards by type
                                        let mut valid_cards = Vec::new();
                                        for &lib_card_id in &library_cards {
                                            if let Some(card) = self.game.cards.try_get(lib_card_id) {
                                                if crate::game::state::GameState::card_matches_search_filter(
                                                    card, &filter,
                                                ) {
                                                    valid_cards.push(lib_card_id);
                                                }
                                            }
                                        }
                                        log::debug!(
                                            "[TYPECYCLING] Found {} valid cards matching filter",
                                            valid_cards.len()
                                        );

                                        // Ask controller to choose a card (or decline to find).
                                        // Set pending_cycling_search BEFORE calling, so that if
                                        // NeedInput is returned and the WASM game loop restarts,
                                        // priority_round() will resume the search directly instead
                                        // of routing the LibrarySearchByName OpponentChoice through
                                        // choose_spell_ability_to_play (which would misroute it).
                                        self.game.sub_action_scratch.pending_cycling_search =
                                            Some((current_priority, land_type.clone()));

                                        let prior_log_size = self.game.logger.log_count();
                                        log::debug!("[TYPECYCLING] About to call choose_from_library_with_hook");
                                        let choice = self.choose_from_library_with_hook(
                                            controller,
                                            current_priority,
                                            &valid_cards,
                                        );
                                        log::debug!(
                                            "[TYPECYCLING] choose_from_library_with_hook returned: {:?}",
                                            choice
                                        );
                                        let chosen_card_opt =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        // Library search succeeded — clear the pending state.
                                        self.game.sub_action_scratch.pending_cycling_search = None;

                                        // Log the choice for replay — record the AUTHORITATIVE
                                        // fetched CardId (not a shadow-fragile positional index).
                                        let replay_choice = crate::game::ReplayChoice::LibrarySearch(chosen_card_opt);
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        // If a card was chosen, move it to hand
                                        if let Some(chosen_card) = chosen_card_opt {
                                            if let Err(e) = self.game.move_card(
                                                chosen_card,
                                                crate::zones::Zone::Library,
                                                crate::zones::Zone::Hand,
                                                current_priority,
                                            ) {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    self.game
                                                        .logger
                                                        .normal(&format!("Failed to move card to hand: {e}"));
                                                }
                                            }
                                        }

                                        // Shuffle library after searching (MTG CR 702.29)
                                        self.game.shuffle_library(current_priority);
                                    }
                                    None => {
                                        // Regular cycling: Draw a card
                                        match self.game.draw_card(current_priority) {
                                            Ok((_, draw_count)) => {
                                                // Check for "second card drawn" triggers
                                                let _ =
                                                    self.game.check_card_drawn_triggers(current_priority, draw_count);
                                            }
                                            Err(e) => {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    self.game
                                                        .logger
                                                        .normal(&format!("Failed to draw from cycling: {e}"));
                                                }
                                            }
                                        }
                                    }
                                }

                                // Cycling is complete (doesn't use the stack)
                            }

                            crate::core::SpellAbility::CastFromGraveyard {
                                card_id,
                                effect_id: _,
                                add_finality_counter,
                            } => {
                                // Cast creature from graveyard (Leonardo, Sewer Samurai)
                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let message = format!(
                                        "{} casts {} from graveyard (with finality counter)",
                                        self.get_player_name(current_priority),
                                        card_name,
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                // Move card from graveyard to stack
                                let owner = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.owner)
                                    .unwrap_or(current_priority);

                                if let Err(e) = self.game.move_card(
                                    card_id,
                                    crate::zones::Zone::Graveyard,
                                    crate::zones::Zone::Stack,
                                    owner,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game.logger.normal(&format!("Error moving from graveyard: {e}"));
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Pay the card's mana cost normally
                                let mana_cost = self.game.cards.get(card_id).map(|c| c.mana_cost).unwrap_or_default();

                                self.mana_engine.update_mut(self.game, current_priority);
                                use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                let mana_sources = self.mana_engine.all_sources();
                                self.sources_to_tap_buffer.clear();
                                let resolver = GreedyManaResolver::new();
                                resolver.compute_tap_order(&mana_cost, mana_sources, &mut self.sources_to_tap_buffer);

                                let mut remaining_hint = mana_cost;
                                for &source_id in &self.sources_to_tap_buffer {
                                    if let Err(e) = self.game.tap_for_mana_and_update_hint(
                                        current_priority,
                                        source_id,
                                        &mut remaining_hint,
                                    ) {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            self.game.logger.normal(&format!("Failed to tap: {e}"));
                                        }
                                    }
                                }

                                // If add_finality_counter, mark the card so it gets a finality counter on ETB
                                if add_finality_counter {
                                    if let Ok(card) = self.game.cards.get_mut(card_id) {
                                        card.add_counter(crate::core::CounterType::Finality, 1);
                                        log::debug!(
                                            "Added finality counter to {} ({}) cast from graveyard",
                                            card_name,
                                            card_id
                                        );
                                    }
                                }

                                // Spell is now on the stack - will resolve when both players pass
                            }

                            // CastAdventure is converted to CastSpell (with the
                            // card swapped to its Adventure face) BEFORE this
                            // match, so it is never reached here.
                            crate::core::SpellAbility::CastAdventure { .. } => {
                                unreachable!("CastAdventure is rewritten to CastSpell before dispatch")
                            }

                            crate::core::SpellAbility::CastFromHandWithAltCost {
                                card_id,
                                alternative_cost,
                            } => {
                                // Cast from hand with an alternative cost (e.g. Summoning Trap for {0}).
                                // Uses the same 8-step casting process as CastFromExile, but from
                                // Zone::Hand with the override cost.
                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let card_name = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.to_string())
                                        .unwrap_or_else(|_| "Unknown".to_string());
                                    let message = format!(
                                        "{} casts {} for {} (alternative cost)",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        alternative_cost
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                self.mana_engine.update_mut(self.game, current_priority);

                                if let Err(e) = self.game.cast_spell_8_step_from(
                                    current_priority,
                                    card_id,
                                    |_, _| smallvec::smallvec![],
                                    &self.mana_engine,
                                    crate::zones::Zone::Hand,
                                    Some(alternative_cost),
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting with alt cost: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Spell is now on the stack - will resolve when both players pass
                            }

                            crate::core::SpellAbility::CastFromLibrary { card_id } => {
                                // Cast the top card of the library (Experimental Frenzy).
                                // Uses the standard 8-step casting process from Zone::Library.
                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let card_name = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.to_string())
                                        .unwrap_or_else(|_| "Unknown".to_string());
                                    let message = format!(
                                        "{} casts {} from the top of their library",
                                        self.get_player_name(current_priority),
                                        card_name,
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                self.mana_engine.update_mut(self.game, current_priority);

                                if let Err(e) = self.game.cast_spell_8_step_from(
                                    current_priority,
                                    card_id,
                                    |_, _| smallvec::smallvec![],
                                    &self.mana_engine,
                                    crate::zones::Zone::Library,
                                    None, // pay printed mana cost
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting from library: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }
                                // Spell is now on the stack
                            }

                            crate::core::SpellAbility::PlayLandFromLibrary { card_id } => {
                                // Play the top card of the library as a land (Experimental Frenzy).
                                // Behaves exactly like playing a land from hand: no stack, immediate
                                // battlefield entry, consumes the land play for the turn.
                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let card_name = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.to_string())
                                        .unwrap_or_else(|_| "Unknown".to_string());
                                    let message = format!(
                                        "{} plays {} from the top of their library",
                                        self.get_player_name(current_priority),
                                        card_name,
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                if let Err(e) = self.game.play_land_from_library(current_priority, card_id) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error playing land from library: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                }
                                // Land play complete — no stack entry
                            }
                        }

                        // After taking an action, switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                        self.game.turn.priority_player = Some(current_priority);
                    }
                }
            }

            // Both players passed priority - reset priority state for the next round
            // (either after stack resolution or for next phase)
            self.game.turn.priority_player = None;
            self.game.turn.consecutive_passes = 0;

            // Check if there are spells on the stack to resolve
            if self.game.stack.is_empty() {
                // Stack is empty, priority round is complete
                break;
            }

            // Resolve the top spell from the stack (MTG Rules 608: Resolving Spells and Abilities)
            // In MTG, the stack is LIFO (Last In, First Out)
            if let Some(&spell_id) = self.game.stack.cards.last() {
                self.resolve_top_spell_from_stack_interactive(spell_id, controller1, controller2)?;
                // After resolving a spell, players get priority again
                // (loop continues). But FIRST, state-based actions are checked
                // (MTG CR 704.3): if a player is at 0 or less life (CR 704.5a)
                // they lose the game immediately. This must happen between stack
                // resolutions, not just at turn boundaries — otherwise a second
                // damage spell on the stack could take the surviving opponent
                // negative too, leaving the "winner" at negative life (and
                // making a strict winner-life invariant unobservable). This is
                // newly reachable now that Lightning Bolt can target players
                // (mtg-565).
                if let Some(result) = self.check_win_condition() {
                    return Ok(Some(result));
                }
                // Loop continues to give priority
            } else {
                // Stack was reported non-empty but has no cards (shouldn't happen)
                break;
            }
        }

        Ok(None)
    }

    /// Resolve a spell from the stack with interactive effect handling
    ///
    /// This wraps `resolve_top_spell_from_stack` and handles interactive effects
    /// like Balance and DiscardCards that require player choices routed through
    /// the controller protocol (mtg-272 network desync fix).
    fn resolve_top_spell_from_stack_interactive(
        &mut self,
        spell_id: CardId,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Check if this spell has any Balance or choice-based discard effects before resolving
        // Also capture SVars for SubAbility resolution
        let (balance_effects, has_choice_discard, svars): (Vec<_>, bool, std::collections::HashMap<String, String>) =
            if let Some(card) = self.game.cards.try_get(spell_id) {
                let balances = card
                    .effects
                    .iter()
                    .filter_map(|e| {
                        if let crate::core::Effect::Balance {
                            card_type,
                            zone,
                            sub_ability,
                        } = e
                        {
                            Some((card_type.clone(), zone.clone(), sub_ability.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();
                let needs_interactive = card.effects.iter().any(|e| {
                    matches!(
                        e,
                        crate::core::Effect::DiscardCards { count, .. } if *count != u8::MAX
                    ) || matches!(
                        e,
                        crate::core::Effect::Loot { .. }
                            | crate::core::Effect::Scry { .. }
                            | crate::core::Effect::Surveil { .. }
                    )
                    // mtg-415: Dig effects on the active player's own library
                    // (e.g. Seismic Sense) need pre-reveal + sync so the network
                    // shadow client knows the card identities before filtering.
                    // Without this, the client's filter (ChangeValid$) sees an
                    // unknown card and short-circuits to "no valid pick", desyncing
                    // hand/library against the server.
                        || matches!(
                            e,
                            crate::core::Effect::Dig { target_self: true, .. }
                        )
                    // Clone (Copy Artifact, etc.): the controller chooses which
                    // permanent to copy (and, if Optional, whether to copy at
                    // all). Route through the controller protocol for
                    // network-safe operation.
                        || matches!(e, crate::core::Effect::Clone { .. })
                    // mtg-589: SearchLibrary tutors (Demonic Tutor, etc.) let
                    // the searcher pick a card from their own library. The naive
                    // execute_effect path picks `library_cards[0]` by iterating
                    // and calling `cards.try_get()` — which works on the server
                    // (all cards materialized) but returns the WRONG card (or
                    // None → no move) on the shadow client, whose own library
                    // cards are reserved-but-unrevealed. Route through
                    // choose_from_library_with_hook so the server picks the
                    // CardId and the client uses the server-authoritative
                    // library_search_result.
                        || matches!(e, crate::core::Effect::SearchLibrary { .. })
                    // PutCardsFromHandOnTopOfLibrary (Brainstorm sub-ability): the
                    // controller must choose which cards to put back.  Route through
                    // the same discard-style protocol (choose_discard_with_hook) but
                    // move the chosen cards to the library instead of the graveyard.
                    // Must be interactive so the network-shadow client receives the
                    // ChoiceRequest carrying the hand reveal before deciding (same
                    // ordering guarantee as the DiscardCards interactive path,
                    // mtg-768 BLOCKER-1/2 pattern).
                        || matches!(e, crate::core::Effect::PutCardsFromHandOnTopOfLibrary { .. })
                });
                (balances, needs_interactive, card.svars.clone())
            } else {
                (Vec::new(), false, std::collections::HashMap::new())
            };

        if has_choice_discard {
            // Use effect-by-effect resolution so we can intercept discard choices
            // (mtg-272) or pre-reveal Dig cards (mtg-415) to route everything
            // through the controller protocol for network-safe operation.
            self.resolve_top_spell_with_discard_hook(spell_id, controller1, controller2)?;
        } else {
            // Resolve the spell normally (Balance effects are no-ops in execute_effect)
            self.resolve_top_spell_from_stack(spell_id)?;
        }

        // Now handle any Balance effects interactively, including SubAbility chains
        for (card_type, zone, sub_ability) in balance_effects {
            self.resolve_balance_effect_chain(
                &card_type,
                &zone,
                sub_ability.as_deref(),
                &svars,
                controller1,
                controller2,
            )?;
        }

        Ok(())
    }

    /// Resolve a spell effect-by-effect, intercepting discard choices through
    /// the controller protocol for network-safe operation (mtg-272).
    ///
    /// This is used instead of `resolve_top_spell_from_stack` when a spell
    /// contains choice-based DiscardCards or Loot effects.
    fn resolve_top_spell_with_discard_hook(
        &mut self,
        spell_id: CardId,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Look up targets
        let targets: SmallVec<[CardId; 2]> = self
            .game
            .sub_action_scratch
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        let should_log = self.verbosity >= VerbosityLevel::Normal && !self.replaying;

        if self.game.cards.get(spell_id).is_err() {
            return Err(crate::MtgError::EntityNotFound(spell_id.as_u32()));
        }

        let spell_owner = self.game.cards.get(spell_id).unwrap().owner;

        // Get card info for logging
        let (card_name, card_effects, card_owner) = if should_log {
            let card = self.game.cards.get(spell_id).unwrap();
            (card.name.to_string(), card.effects.clone(), card.owner)
        } else {
            (String::new(), Vec::new(), crate::core::PlayerId::new(0))
        };

        if should_log {
            let message = format!("{} ({}) resolves", card_name, spell_id);
            self.game.logger.gamelog(&message);
        }

        // Collect resolved effects without executing
        let effects = self.game.resolve_spell_collect_effects(spell_id, &targets)?;

        if let Some(effects) = effects {
            for effect in &effects {
                match effect {
                    // Choice-based discard: route through controller
                    crate::core::Effect::DiscardCards {
                        player,
                        count,
                        remember_discarded,
                        ..
                    } if *count != u8::MAX => {
                        let discard_count = *count as usize;
                        let remember = *remember_discarded;

                        // push_reveals is a no-op under the synchronous transport
                        // (reveals ride the ChoiceRequest buffer). The shadow hand
                        // MEMBERSHIP is already correct from its own draw; the drawn
                        // cards' IDENTITIES are materialised by the prepare→sync below,
                        // placed INSIDE the actual_count>0 guard so an empty-hand
                        // discard never blocks (mtg-768 BLOCKER-1).
                        self.push_reveals(*player);
                        if let Some(opp) = self.game.get_other_player_id(*player) {
                            self.push_reveals(opp);
                        }

                        let hand: SmallVec<[CardId; 8]> = self
                            .game
                            .get_player_zones(*player)
                            .map(|zones| zones.hand.cards.iter().copied().collect())
                            .unwrap_or_default();

                        let actual_count = discard_count.min(hand.len());
                        if actual_count > 0 {
                            let controller: &mut dyn PlayerController = if *player == controller1.player_id() {
                                controller1
                            } else {
                                controller2
                            };
                            // mtg-768 BLOCKER-2: the structurally identical SPELL-
                            // resolution draw-then-discard (Careful Study, Frantic
                            // Search, Thirst for Knowledge, Compulsive Research,
                            // Blast of Genius, Ancient Excavation, Artificer's
                            // Epiphany, ...) needs the SAME prepare→sync→decide
                            // ordering as the activated-ability path: receive the
                            // discard ChoiceRequest (whose buffer carries the
                            // just-drawn cards' reveals) BEFORE syncing the shadow,
                            // else the controller decides on unmaterialised cards
                            // (information-independence desync). Inside the
                            // actual_count>0 guard so an empty-hand discard never
                            // blocks. prepare_for_priority_choice() is a no-op for the
                            // authoritative server / non-network controllers.
                            let _ = controller.prepare_for_priority_choice();
                            self.sync_to_action();
                            // Capture log size BEFORE the choice so the ChoicePoint
                            // we log below can rewind cleanly to the pre-choice
                            // state. Mandatory for replay determinism — without
                            // this, a rewind that lands inside the spell will
                            // re-prompt the user (infinite loop). See the matching
                            // discard site in the activated-ability branch.
                            let prior_log_size = self.game.logger.log_count();
                            let choice = self.choose_discard_with_hook(controller, *player, &hand, actual_count);
                            let cards_to_discard = match choice {
                                crate::game::controller::ChoiceResult::Ok(v) => v,
                                crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                    return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                }
                                crate::game::controller::ChoiceResult::ExitGame => {
                                    return Err(crate::MtgError::InvalidAction(
                                        "Game exit requested during spell discard".into(),
                                    ));
                                }
                                crate::game::controller::ChoiceResult::Error(e) => {
                                    return Err(crate::MtgError::InvalidAction(format!(
                                        "Controller error during spell discard: {e}"
                                    )));
                                }
                                crate::game::controller::ChoiceResult::UndoRequest(_) => {
                                    // Undo during spell resolution: skip this discard
                                    SmallVec::new()
                                }
                            };

                            // Log the discard choice for replay.
                            let replay_choice = crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                            self.log_choice_point(*player, Some(replay_choice), prior_log_size);

                            for card_id in cards_to_discard {
                                if remember {
                                    self.game.remembered_cards.push(card_id);
                                }
                                // Cause = the resolving spell's controller (CR
                                // 701.8): this discard is forced by that spell,
                                // so a Discarded self-trigger (Psychic Purge)
                                // sees `spell_owner` as the cause controller. If
                                // the spell's owner is discarding their OWN card
                                // (cause == owner) the opponent-only gate keeps
                                // the punisher from firing.
                                if let Err(e) = self.game.discard_card(*player, card_id, Some(spell_owner)) {
                                    if should_log {
                                        eprintln!("    Failed to discard: {e}");
                                    }
                                }
                            }
                        }
                    }
                    // Loot: discard through controller, then draw
                    crate::core::Effect::Loot {
                        player,
                        discard_count,
                        draw_count,
                    } => {
                        let d_count = *discard_count as usize;

                        // Sync reveals before looting
                        self.sync_to_action();

                        if d_count > 0 {
                            let hand: SmallVec<[CardId; 8]> = self
                                .game
                                .get_player_zones(*player)
                                .map(|zones| zones.hand.cards.iter().copied().collect())
                                .unwrap_or_default();

                            let actual_count = d_count.min(hand.len());
                            if actual_count > 0 {
                                let controller: &mut dyn PlayerController = if *player == controller1.player_id() {
                                    controller1
                                } else {
                                    controller2
                                };
                                // Capture log size BEFORE the choice so the
                                // ChoicePoint we log can rewind cleanly.
                                let prior_log_size = self.game.logger.log_count();
                                let choice = self.choose_discard_with_hook(controller, *player, &hand, actual_count);
                                let cards_to_discard = choice.into_result().map_err(|e| {
                                    crate::MtgError::InvalidAction(format!(
                                        "Loot discard choice failed during spell resolution: {e}"
                                    ))
                                })?;

                                // Log the discard choice for replay.
                                let replay_choice = crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                                self.log_choice_point(*player, Some(replay_choice), prior_log_size);

                                for card_id in cards_to_discard {
                                    // Loot's discard is part of the resolving
                                    // spell/ability; attribute the cause to its
                                    // controller. A player looting themselves
                                    // (cause == owner) does not fire an
                                    // opponent-gated Discarded trigger.
                                    if let Err(e) = self.game.discard_card(*player, card_id, Some(spell_owner)) {
                                        if should_log {
                                            eprintln!("    Failed to discard: {e}");
                                        }
                                    }
                                }
                            }
                        }

                        for _ in 0..*draw_count {
                            match self.game.draw_card(*player) {
                                Ok((_, draw_num)) => {
                                    let _ = self.game.check_card_drawn_triggers(*player, draw_num);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    // Brainstorm sub-ability: put N cards from hand on top of library.
                    // The controller chooses which cards; we reuse `choose_discard_with_hook`
                    // for the selection protocol and then move the chosen cards to the
                    // library instead of the graveyard (CR 701.19b: put on top in any order).
                    // The cards are put back in reverse-chosen order so the last card chosen
                    // ends up on top (matching typical "last put = top" convention).
                    crate::core::Effect::PutCardsFromHandOnTopOfLibrary { player, count } => {
                        let put_count = *count as usize;

                        self.push_reveals(*player);
                        if let Some(opp) = self.game.get_other_player_id(*player) {
                            self.push_reveals(opp);
                        }

                        let hand: SmallVec<[CardId; 8]> = self
                            .game
                            .get_player_zones(*player)
                            .map(|zones| zones.hand.cards.iter().copied().collect())
                            .unwrap_or_default();

                        let actual_count = put_count.min(hand.len());
                        if actual_count > 0 {
                            let controller: &mut dyn PlayerController = if *player == controller1.player_id() {
                                controller1
                            } else {
                                controller2
                            };
                            let _ = controller.prepare_for_priority_choice();
                            self.sync_to_action();
                            let prior_log_size = self.game.logger.log_count();
                            // Reuse choose_discard_with_hook — the semantics are
                            // "choose N cards from hand"; the destination differs.
                            let choice = self.choose_discard_with_hook(controller, *player, &hand, actual_count);
                            let cards_to_put = match choice {
                                crate::game::controller::ChoiceResult::Ok(v) => v,
                                crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                    return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                }
                                crate::game::controller::ChoiceResult::ExitGame => {
                                    return Err(crate::MtgError::InvalidAction(
                                        "Game exit requested during put-back".into(),
                                    ));
                                }
                                crate::game::controller::ChoiceResult::Error(e) => {
                                    return Err(crate::MtgError::InvalidAction(format!(
                                        "Controller error during put-back: {e}"
                                    )));
                                }
                                crate::game::controller::ChoiceResult::UndoRequest(_) => {
                                    // Undo during spell resolution: fall back to heuristic
                                    self.game.pick_cards_to_put_back_heuristic(&hand, actual_count)
                                }
                            };
                            // Log as a Discard-style ChoicePoint for replay determinism
                            // (same encoding: ordered list of CardIds chosen from hand).
                            let replay_choice = crate::game::ReplayChoice::Discard(cards_to_put.clone());
                            self.log_choice_point(*player, Some(replay_choice), prior_log_size);

                            // Move chosen cards to top of library.  Put them in order
                            // [0..n] — last-pushed card ends up on top (the controller
                            // chose them as "first on top" order when we asked).
                            if let Err(e) = self
                                .game
                                .execute_put_cards_from_hand_on_top_of_library(*player, &cards_to_put)
                            {
                                if should_log {
                                    eprintln!("    Failed to put cards back on library: {e}");
                                }
                            }
                        }
                    }
                    // mtg-415: Dig from your own library (e.g. Seismic Sense).
                    //
                    // The naive `execute_effect` path looks at top-N cards, then
                    // filters them by ChangeValid$ before moving any to hand.
                    // In network mode the shadow client doesn't know the
                    // identities of those top-N cards yet (RevealCard messages
                    // only get pushed at the next ChoiceRequest), so its filter
                    // always fizzles, every card goes to "rest", and hand /
                    // library zones diverge from the server (FATAL state hash
                    // mismatch).
                    //
                    // Fix: route the "which of these N cards do you keep"
                    // decision through `choose_from_library_with_hook` — the
                    // same mechanism cycling/tutoring use. The server's call
                    // builds a ChoiceRequest with `library_search_cards` so the
                    // ChoiceAccepted reply tells the client the
                    // server-authoritative CardId. Both sides then perform the
                    // same move_card operations using only top_ids (which is
                    // synced via LibraryReordered) and the chosen CardId.
                    //
                    // We always iterate `min(change_count, top_ids.len())`
                    // times so that the per-side ChoicePoint sequences match,
                    // even when one side has empty `valid_ids`.
                    crate::core::Effect::Dig {
                        target_self: true,
                        dig_count,
                        change_count,
                        change_all,
                        destination,
                        rest_destination,
                        optional: _,
                        rest_random,
                        reveal,
                        change_valid,
                        ..
                    } => {
                        use crate::zones::Zone;
                        let digger = self.game.turn.active_player;
                        let take_count = *dig_count as usize;

                        // Snapshot top N library CardIds (top is end of Vec).
                        let top_ids: SmallVec<[CardId; 8]> = self
                            .game
                            .get_player_zones(digger)
                            .map(|z| z.library.cards.iter().rev().take(take_count).copied().collect())
                            .unwrap_or_default();

                        if !top_ids.is_empty() {
                            // No explicit pre-reveal needed: the server's
                            // network handler (server.rs handles
                            // library_search_cards) sends CardRevealed for
                            // every CardId in `library_search_cards` BEFORE
                            // forwarding the ChoiceRequest, and the auto-reveal
                            // inside `move_card` (Library->Hand path) logs the
                            // matching RevealCard action symmetrically on both
                            // sides. Adding a manual maybe_reveal_to_player
                            // here would diverge action_counts because on the
                            // server the card is already in EntityStore (mask
                            // gets set, single RevealCard logged) while on the
                            // client it's late-binding (RevealCard logged
                            // without setting mask, then move_card's auto-
                            // reveal logs ANOTHER RevealCard once the card is
                            // materialized).

                            let digger_name = self
                                .game
                                .get_player(digger)
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| format!("Player {}", digger.as_u32()));
                            let verb = if *reveal { "reveals" } else { "looks at" };
                            self.game.logger.gamelog(&format!(
                                "{} {} the top {} card{} of their library",
                                digger_name,
                                verb,
                                top_ids.len(),
                                if top_ids.len() == 1 { "" } else { "s" }
                            ));

                            // Maximum number of choose_from_library iterations.
                            // Driven by top_ids.len() (same on both sides) so
                            // the per-side ChoicePoint sequence matches even
                            // when local ChangeValid$ filtering yields zero
                            // candidates on the shadow client.
                            let max_iterations = if *change_all {
                                top_ids.len()
                            } else {
                                (*change_count as usize).min(top_ids.len())
                            };

                            let has_filter = !change_valid.is_empty();
                            let mut chosen: SmallVec<[CardId; 8]> = SmallVec::new();
                            let mut remaining_top: SmallVec<[CardId; 8]> = top_ids.clone();

                            for _ in 0..max_iterations {
                                // Filter remaining_top by ChangeValid$. On the
                                // server this is non-empty; on the client it
                                // typically ends up empty (cards not yet
                                // materialized) but choose_from_library_with_hook
                                // still works because NLC falls back to
                                // server-provided library_search_names.
                                let valid_ids: SmallVec<[CardId; 8]> = if has_filter {
                                    remaining_top
                                        .iter()
                                        .copied()
                                        .filter(|&id| {
                                            self.game
                                                .cards
                                                .try_get(id)
                                                .is_some_and(|card| change_valid.iter().any(|f| f.matches(card)))
                                        })
                                        .collect()
                                } else {
                                    remaining_top.clone()
                                };

                                let prior_log_size = self.game.logger.log_count();
                                let controller: &mut dyn PlayerController = if digger == controller1.player_id() {
                                    controller1
                                } else {
                                    controller2
                                };
                                let pick = self.choose_from_library_with_hook(controller, digger, &valid_ids);
                                let chosen_card_opt = match pick {
                                    crate::game::controller::ChoiceResult::Ok(v) => v,
                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                    }
                                    crate::game::controller::ChoiceResult::ExitGame => {
                                        return Err(crate::MtgError::InvalidAction(
                                            "Game exit requested during Dig".into(),
                                        ));
                                    }
                                    crate::game::controller::ChoiceResult::Error(e) => {
                                        return Err(crate::MtgError::InvalidAction(format!(
                                            "Controller error during Dig: {e}"
                                        )));
                                    }
                                    crate::game::controller::ChoiceResult::UndoRequest(_) => None,
                                };

                                // Log the choice for snapshot/replay (mirrors
                                // typecycling). Record the AUTHORITATIVE fetched
                                // CardId (not a positional index) — None means declined.
                                let replay_choice = crate::game::ReplayChoice::LibrarySearch(chosen_card_opt);
                                self.log_choice_point(digger, Some(replay_choice), prior_log_size);

                                if let Some(c) = chosen_card_opt {
                                    chosen.push(c);
                                    if let Some(pos) = remaining_top.iter().position(|&x| x == c) {
                                        remaining_top.remove(pos);
                                    }
                                } else {
                                    // Declined: stop picking. Leftover top_ids
                                    // go to rest_destination below.
                                    break;
                                }
                            }

                            // Move chosen cards to destination via move_card
                            // (handles auto-reveal + undo logging on both
                            // sides).
                            for card_id in &chosen {
                                let card_name = self
                                    .game
                                    .cards
                                    .get(*card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));
                                let _ = self.game.move_card(*card_id, Zone::Library, *destination, digger);
                                // mtg-212: reveal-timing-independent verifier key
                                // (see the Effect::Dig branch in actions/mod.rs for
                                // the async-reveal rationale). The DISPLAYED name
                                // can be `card#<id>` on a network shadow's forward
                                // pass and the real name on a rewind replay; the
                                // stable id form keeps the verifier from flagging
                                // it as a fatal desync.
                                let action = if *reveal { "reveals and puts" } else { "puts" };
                                let stable = format!(
                                    "{} {} card#{} into {:?}",
                                    digger_name,
                                    action,
                                    card_id.as_u32(),
                                    destination
                                );
                                self.game.logger.gamelog_reveal_stable(
                                    &format!("{} {} {} into {:?}", digger_name, action, card_name, destination),
                                    &stable,
                                );
                            }

                            // Move "rest" (everything not chosen) to
                            // rest_destination. remaining_top is already in
                            // top-of-library order; both sides agree on it
                            // because top_ids comes from the synced library.
                            let mut rest = remaining_top;
                            if !rest.is_empty() {
                                if *rest_random {
                                    // Same deterministic shuffle as the legacy
                                    // execute_effect path (driven by CardId
                                    // alone so both sides agree).
                                    let len = rest.len();
                                    for i in (1..len).rev() {
                                        let j = (rest[i].as_u32() as usize + i) % (i + 1);
                                        rest.swap(i, j);
                                    }
                                }
                                if *rest_destination == Zone::Library {
                                    // Capture pre-reorder library order so a
                                    // rewind can restore it (mtg-ba6uq #2): the
                                    // raw remove/add_to_bottom below is not
                                    // otherwise undo-logged.
                                    self.game.log_library_reorder(digger, false);
                                    if let Some(zones) = self.game.get_player_zones_mut(digger) {
                                        for &card_id in &rest {
                                            zones.library.remove(card_id);
                                            zones.library.add_to_bottom(card_id);
                                        }
                                    }
                                    self.game.logger.gamelog(&format!(
                                        "{} puts {} card{} on the bottom of their library",
                                        digger_name,
                                        rest.len(),
                                        if rest.len() == 1 { "" } else { "s" }
                                    ));
                                } else {
                                    for &card_id in &rest {
                                        let card_name = self
                                            .game
                                            .cards
                                            .get(card_id)
                                            .map(|c| c.name.to_string())
                                            .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));
                                        let _ = self.game.move_card(card_id, Zone::Library, *rest_destination, digger);
                                        let dest_name = match rest_destination {
                                            Zone::Graveyard => "their graveyard",
                                            Zone::Exile => "exile",
                                            Zone::Hand => "their hand",
                                            Zone::Library | Zone::Battlefield | Zone::Stack | Zone::Command => {
                                                "another zone"
                                            }
                                        };
                                        // mtg-212: reveal-timing-independent verifier key.
                                        let stable = format!(
                                            "{} puts card#{} into {}",
                                            digger_name,
                                            card_id.as_u32(),
                                            dest_name
                                        );
                                        self.game.logger.gamelog_reveal_stable(
                                            &format!("{} puts {} into {}", digger_name, card_name, dest_name),
                                            &stable,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Scry: snapshot top N → controller decides → apply.
                    // Routes through PlayerController instead of the engine
                    // heuristic so non-Heuristic controllers (Random, Network)
                    // can make their own scry decisions. HeuristicController's
                    // override produces identical decisions to the legacy
                    // engine heuristic, so heuristic games stay byte-identical.
                    //
                    // ## Network-correctness note (Phase D audit)
                    //
                    // For server-LOCAL controller decisions (e.g. P1 scries on
                    // a server where P1 is HeuristicController), this intercept
                    // applies the decision to the server's library but does NOT
                    // currently push a side-channel message to the client. The
                    // client's shadow GameLoop runs its own intercept and asks
                    // its WasmRemoteController, which expects an OpponentChoice
                    // — so a scry triggered by a server-local player can stall
                    // the client's shadow loop in mixed local/remote setups.
                    //
                    // This was previously masked by the engine-baked heuristic
                    // running consistently on both sides (when the heuristic's
                    // inputs happened to match — see mtg-420 for the case
                    // where they didn't, fixed for the legacy path by 3b052c70
                    // on a never-merged branch). With Phase B/C the engine no
                    // longer carries that heuristic, so the consistency
                    // accident is gone. The architecturally correct fix is to
                    // either (a) emit OpponentChoice for server-local choices
                    // (uniform with client-prompted choices), or (b) broadcast
                    // an authoritative LibraryReordered after applying the
                    // server's decision (analogous to 3b052c70 but carrying a
                    // real player choice, not a heuristic accident).
                    //
                    // Tracked separately; not blocking Phase D since:
                    //   - existing network fuzz pass rate is 4/5 on both this
                    //     branch and integration baseline (no regression from
                    //     Phase A/B/C);
                    //   - the failing seed (=2) trips a different state-hash
                    //     mismatch unrelated to the scry path;
                    //   - no currently-shipping deck triggers scry on a path
                    //     that hits this gap (server-local + client-shadow)
                    //     before some other unrelated desync fires.
                    crate::core::Effect::Scry {
                        player,
                        count,
                        only_if_bargained,
                    } => {
                        // Condition$ Bargain: skip scry unless the source spell
                        // was bargained (CR 702.162). Check bargain_paid on the
                        // resolving spell (spell_id) — same check as execute_effect.
                        if *only_if_bargained {
                            let is_bargained = self.game.cards.try_get(spell_id).is_some_and(|c| c.bargain_paid);
                            if !is_bargained {
                                // Condition not met — skip the scry silently.
                                continue;
                            }
                        }
                        let scry_player = *player;
                        let revealed = self.game.scry_snapshot_top_n(scry_player, *count);
                        if !revealed.is_empty() {
                            // Sync reveals before consulting the controller.
                            self.push_reveals(scry_player);
                            if let Some(opp) = self.game.get_other_player_id(scry_player) {
                                self.push_reveals(opp);
                            }
                            self.sync_to_action();

                            let controller: &mut dyn PlayerController = if scry_player == controller1.player_id() {
                                controller1
                            } else {
                                controller2
                            };
                            let prior_log_size = self.game.logger.log_count();
                            let view = crate::game::GameStateView::new(self.game, scry_player);
                            let choice = controller.choose_scry_order(&view, &revealed);
                            let decision = match choice {
                                crate::game::controller::ChoiceResult::Ok(d) => d,
                                crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                    return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                }
                                crate::game::controller::ChoiceResult::ExitGame => {
                                    return Err(crate::MtgError::InvalidAction(
                                        "Game exit requested during scry".into(),
                                    ));
                                }
                                crate::game::controller::ChoiceResult::Error(e) => {
                                    return Err(crate::MtgError::InvalidAction(format!(
                                        "Controller error during scry: {e}"
                                    )));
                                }
                                crate::game::controller::ChoiceResult::UndoRequest(_) => {
                                    // Undo during scry: fall back to engine heuristic
                                    // (matches legacy behaviour for the no-controller path).
                                    crate::game::ScryDecision::keep_all_on_top(&revealed)
                                }
                            };

                            // Log the scry choice for snapshot/replay determinism.
                            let replay_choice = crate::game::ReplayChoice::Scry {
                                top: decision.top.clone(),
                                bottom: decision.bottom.clone(),
                            };
                            self.log_choice_point(scry_player, Some(replay_choice), prior_log_size);

                            if let Err(e) = self.game.scry_apply_decision(scry_player, &revealed, &decision) {
                                if should_log {
                                    eprintln!("    Failed to apply scry decision: {e}");
                                }
                            }
                        }
                    }
                    // Surveil: same dispatch shape as scry; apply moves cards to
                    // graveyard instead of bottom of library.
                    crate::core::Effect::Surveil { player, count } => {
                        let surveil_player = *player;
                        let revealed = self.game.surveil_snapshot_top_n(surveil_player, *count);
                        if !revealed.is_empty() {
                            self.push_reveals(surveil_player);
                            if let Some(opp) = self.game.get_other_player_id(surveil_player) {
                                self.push_reveals(opp);
                            }
                            self.sync_to_action();

                            let controller: &mut dyn PlayerController = if surveil_player == controller1.player_id() {
                                controller1
                            } else {
                                controller2
                            };
                            let prior_log_size = self.game.logger.log_count();
                            let view = crate::game::GameStateView::new(self.game, surveil_player);
                            let choice = controller.choose_surveil(&view, &revealed);
                            let decision = match choice {
                                crate::game::controller::ChoiceResult::Ok(d) => d,
                                crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                    return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                }
                                crate::game::controller::ChoiceResult::ExitGame => {
                                    return Err(crate::MtgError::InvalidAction(
                                        "Game exit requested during surveil".into(),
                                    ));
                                }
                                crate::game::controller::ChoiceResult::Error(e) => {
                                    return Err(crate::MtgError::InvalidAction(format!(
                                        "Controller error during surveil: {e}"
                                    )));
                                }
                                crate::game::controller::ChoiceResult::UndoRequest(_) => {
                                    crate::game::SurveilDecision::keep_all_on_top(&revealed)
                                }
                            };

                            let replay_choice = crate::game::ReplayChoice::Surveil {
                                top: decision.top.clone(),
                                graveyard: decision.graveyard.clone(),
                            };
                            self.log_choice_point(surveil_player, Some(replay_choice), prior_log_size);

                            if let Err(e) = self.game.surveil_apply_decision(surveil_player, &revealed, &decision) {
                                if should_log {
                                    eprintln!("    Failed to apply surveil decision: {e}");
                                }
                            }
                        }
                    }
                    // Clone (Copy Artifact, etc.): the SOURCE spell enters the
                    // battlefield as a copy of a permanent the controller
                    // chooses (CR 707). The choice (and the optional "you may")
                    // is routed through the controller via choose_targets so it
                    // is network-safe and information-independent.
                    crate::core::Effect::Clone {
                        choices_filter,
                        add_types,
                        optional,
                        ..
                    } => {
                        // Enumerate legal copy targets on the battlefield,
                        // excluding the cloning permanent itself (CR 707 — you
                        // copy *another* object; Choices$ Artifact.Other).
                        let valid_targets: SmallVec<[CardId; 4]> = self
                            .game
                            .battlefield
                            .cards
                            .iter()
                            .copied()
                            .filter(|&cid| cid != spell_id)
                            .filter(|&cid| self.game.cards.try_get(cid).is_some_and(|c| choices_filter.matches(c)))
                            .collect();

                        // Sync reveals so the shadow client sees the battlefield
                        // identities before the choice is requested.
                        self.push_reveals(spell_owner);
                        if let Some(opp) = self.game.get_other_player_id(spell_owner) {
                            self.push_reveals(opp);
                        }
                        self.sync_to_action();

                        let chosen: Option<CardId> = if valid_targets.is_empty() {
                            None
                        } else {
                            let controller: &mut dyn PlayerController = if spell_owner == controller1.player_id() {
                                controller1
                            } else {
                                controller2
                            };
                            let prior_log_size = self.game.logger.log_count();
                            let choice =
                                // Single-target site (copy-spell retarget) — bounds (1, 1).
                                self.choose_targets_with_hook(controller, spell_owner, spell_id, &valid_targets, 1, 1);
                            let picked = match choice {
                                crate::game::controller::ChoiceResult::Ok(v) => v.first().copied(),
                                crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                    return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                }
                                crate::game::controller::ChoiceResult::ExitGame => {
                                    return Err(crate::MtgError::InvalidAction(
                                        "Game exit requested during Clone".into(),
                                    ));
                                }
                                crate::game::controller::ChoiceResult::Error(e) => {
                                    return Err(crate::MtgError::InvalidAction(format!(
                                        "Controller error during Clone: {e}"
                                    )));
                                }
                                crate::game::controller::ChoiceResult::UndoRequest(_) => None,
                            };

                            // Log exactly what the controller returned, using the
                            // standard Targets choice point (this branch routes
                            // through choose_targets_with_hook, identical to the
                            // cast-time targeting path). Replay reproduces the
                            // controller's pick verbatim; the deterministic
                            // !optional fallback below is a pure function of the
                            // already-synced valid_targets, so it reproduces
                            // identically on every side without extra logging.
                            let replay_targets: SmallVec<[CardId; 4]> = picked.into_iter().collect();
                            let replay_choice = crate::game::ReplayChoice::Targets(replay_targets);
                            self.log_choice_point(spell_owner, Some(replay_choice), prior_log_size);

                            // For non-optional Clone (e.g. a plain "enters as a
                            // copy"), a controller that declines still must copy
                            // *something* if a legal choice exists (CR 707 — the
                            // copy is not optional). Optional Clone (Copy
                            // Artifact's "You may ...") honours the decline.
                            if picked.is_none() && !*optional {
                                valid_targets.first().copied()
                            } else {
                                picked
                            }
                        };

                        if let Some(copy_target) = chosen {
                            if let Err(e) = self.game.apply_clone(spell_id, copy_target, add_types) {
                                if should_log {
                                    eprintln!("    Failed to apply Clone: {e}");
                                }
                            }
                        } else if should_log {
                            let src_name = self
                                .game
                                .cards
                                .try_get(spell_id)
                                .map(|c| c.name.to_string())
                                .unwrap_or_default();
                            self.game
                                .logger
                                .gamelog(&format!("{} enters the battlefield without copying", src_name));
                        }
                    }
                    // mtg-589: SearchLibrary tutor (Demonic Tutor, etc.).
                    //
                    // The searcher picks a card matching `card_type_filter` from
                    // their OWN library and moves it to `destination`, then
                    // (optionally) shuffles. The legacy execute_effect path picks
                    // `library_cards[0]` by iterating with `cards.try_get()`.
                    // That works on the server (every library card is
                    // materialized) but on the shadow client the searcher's own
                    // library is reserved-but-unrevealed, so `try_get` returns
                    // None for every card → `found_card = None` → no move. The
                    // server moves a card (library -1, hand +1) while the client
                    // moves nothing → FATAL state-hash mismatch on the next
                    // choice (hand/library sizes differ by one).
                    //
                    // Fix: route the pick through `choose_from_library_with_hook`
                    // (the same mechanism cycling/Dig/tutoring use). On the
                    // server the controller picks the CardId; the network handler
                    // sends `library_search_result` + a `CardRevealed(Searched)`
                    // so the client materializes and moves the exact same CardId.
                    // Both sides then run identical move_card + shuffle_library.
                    crate::core::Effect::SearchLibrary {
                        player,
                        card_type_filter,
                        destination,
                        enters_tapped,
                        shuffle,
                    } => {
                        use crate::zones::Zone;
                        // The parsed effect's `player` is a placeholder (PlayerId
                        // 0) when the card script has no explicit Defined$ — it
                        // means "the controller of this spell" (e.g. Demonic
                        // Tutor searches the CASTER's own library). The regular
                        // resolve path resolves this in its log_effect_execution
                        // mapping; we must do the same here, or the search would
                        // wrongly target player 0's library.
                        let search_player = if player.is_placeholder() { spell_owner } else { *player };

                        // Build the set of matching library CardIds. On the
                        // server this is the real, filtered library; on the
                        // shadow client the searcher's library cards are
                        // unrevealed (`try_get` → None), so this is empty and
                        // choose_from_library_with_hook falls back to the
                        // server-authoritative library_search_result.
                        let library_cards: SmallVec<[CardId; 8]> = self
                            .game
                            .get_player_zones(search_player)
                            .map(|z| z.library.cards.iter().copied().collect())
                            .unwrap_or_default();
                        let valid_cards: SmallVec<[CardId; 8]> = library_cards
                            .iter()
                            .copied()
                            .filter(|&card_id| {
                                self.game.cards.try_get(card_id).is_some_and(|card| {
                                    crate::game::state::GameState::card_matches_search_filter(card, card_type_filter)
                                })
                            })
                            .collect();

                        // Note: the "searches ... library for a ... card" gamelog
                        // is emitted by the trailing display-logging loop at the
                        // end of this method (log_effect_execution), which now
                        // resolves the SearchLibrary `player` placeholder.

                        // Sync reveals so the shadow client has up-to-date zones
                        // before the choice is requested.
                        self.push_reveals(search_player);
                        if let Some(opp) = self.game.get_other_player_id(search_player) {
                            self.push_reveals(opp);
                        }
                        self.sync_to_action();

                        let prior_log_size = self.game.logger.log_count();
                        let controller: &mut dyn PlayerController = if search_player == controller1.player_id() {
                            controller1
                        } else {
                            controller2
                        };
                        let pick = self.choose_from_library_with_hook(controller, search_player, &valid_cards);
                        let chosen_card_opt = match pick {
                            crate::game::controller::ChoiceResult::Ok(v) => v,
                            crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                            }
                            crate::game::controller::ChoiceResult::ExitGame => {
                                return Err(crate::MtgError::InvalidAction(
                                    "Game exit requested during library search".into(),
                                ));
                            }
                            crate::game::controller::ChoiceResult::Error(e) => {
                                return Err(crate::MtgError::InvalidAction(format!(
                                    "Controller error during library search: {e}"
                                )));
                            }
                            crate::game::controller::ChoiceResult::UndoRequest(_) => None,
                        };

                        // Log the choice for snapshot/replay determinism. Record
                        // the AUTHORITATIVE fetched CardId (not a positional index;
                        // None == declined / not found, legal per CR 701.19c
                        // "may fail to find").
                        let replay_choice = crate::game::ReplayChoice::LibrarySearch(chosen_card_opt);
                        self.log_choice_point(search_player, Some(replay_choice), prior_log_size);

                        // Move the chosen card from library to destination. The
                        // CardRevealed(Searched) the server sends for this CardId
                        // materializes it in the client shadow before move_card.
                        if let Some(chosen_card) = chosen_card_opt {
                            self.game
                                .move_card(chosen_card, Zone::Library, *destination, search_player)?;
                            if *destination == Zone::Battlefield && *enters_tapped {
                                let _ = self.game.tap_permanent(chosen_card);
                            }
                        }

                        // Shuffle the library if required (MTG CR 701.19b).
                        if *shuffle {
                            self.game.shuffle_library(search_player);
                        }
                    }
                    // All other effects: execute normally
                    _ => {
                        if let Err(e) = self.game.execute_effect(effect) {
                            if should_log {
                                eprintln!("    Failed to execute effect: {e}");
                            }
                        }
                    }
                }
            }
        }

        // Finalize the spell (move from stack to destination, ETB, etc.)
        self.game.resolve_spell_finalize(spell_id, &targets)?;

        // Push reveals after spell resolution
        self.push_reveals(spell_owner);
        if let Some(opponent) = self.game.get_other_player_id(spell_owner) {
            self.push_reveals(opponent);
        }

        // Log effects for display
        if should_log {
            use crate::core::{Effect, TargetRef};
            let mut target_index = 0;
            for effect in &card_effects {
                let effect_to_log = match effect {
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } if target_index < targets.len() => {
                        let replaced = Effect::DealDamage {
                            target: TargetRef::Permanent(targets[target_index]),
                            amount: *amount,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::CounterSpell {
                        target,
                        spell_restriction,
                        remember_mana_value,
                    } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::CounterSpell {
                            target: targets[target_index],
                            spell_restriction: spell_restriction.clone(),
                            remember_mana_value: *remember_mana_value,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent {
                        target,
                        restriction,
                        no_regenerate,
                    } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::DestroyPermanent {
                            target: targets[target_index],
                            restriction: restriction.clone(),
                            no_regenerate: *no_regenerate,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DrawCards { player, count } if player.is_placeholder() => Effect::DrawCards {
                        player: card_owner,
                        count: *count,
                    },
                    Effect::DiscardCards {
                        player,
                        count,
                        remember_discarded,
                        ..
                    } if player.is_placeholder() => Effect::DiscardCards {
                        player: card_owner,
                        count: *count,
                        remember_discarded: *remember_discarded,
                        optional: false,
                        remember_discarding_players: false,
                    },
                    // mtg-589: resolve the SearchLibrary `player` placeholder
                    // for display, mirroring the regular resolve path's mapping
                    // (otherwise the log would name player 0 instead of the
                    // actual searcher).
                    Effect::SearchLibrary {
                        player,
                        card_type_filter,
                        destination,
                        enters_tapped,
                        shuffle,
                    } if player.is_placeholder() => Effect::SearchLibrary {
                        player: card_owner,
                        card_type_filter: card_type_filter.clone(),
                        destination: *destination,
                        enters_tapped: *enters_tapped,
                        shuffle: *shuffle,
                    },
                    // AddTurn (Time Walk): resolve placeholder player (0) to the
                    // controller for display, matching actions/mod.rs (mtg-551).
                    Effect::AddTurn { player, num_turns } if player.is_placeholder() => Effect::AddTurn {
                        player: card_owner,
                        num_turns: *num_turns,
                    },
                    Effect::CreateToken {
                        controller,
                        token_script,
                        amount,
                        for_each_player,
                    } if *controller == crate::core::PlayerId::new(0) => Effect::CreateToken {
                        controller: card_owner,
                        token_script: token_script.clone(),
                        amount: *amount,
                        for_each_player: *for_each_player,
                    },
                    _ => effect.clone(),
                };
                self.log_effect_execution(&card_name, spell_id, &effect_to_log, card_owner);
            }

            // Guard on battlefield membership: an Adventure spell resolves to a
            // creature card in EXILE (CR 715.3d), not the battlefield. (Mirrors
            // the guard in the other resolution path so both log identically —
            // log divergence between paths is a desync risk.)
            if self.game.battlefield.contains(spell_id) {
                if let Some(card) = self.game.cards.try_get(spell_id) {
                    if card.is_creature() {
                        let power = self
                            .game
                            .get_effective_power(spell_id)
                            .unwrap_or_else(|_| i32::from(card.current_power()));
                        let toughness = self
                            .game
                            .get_effective_toughness(spell_id)
                            .unwrap_or_else(|_| i32::from(card.current_toughness()));
                        let message = format!(
                            "{} ({}) enters the battlefield as a {}/{} creature",
                            card_name, spell_id, power, toughness
                        );
                        self.game.logger.gamelog(&message);
                    }
                }
            }

            // Set starting loyalty counters for planeswalkers (MTG CR 306.5b)
            if let Some(card) = self.game.cards.try_get(spell_id) {
                if let Some(loyalty) = card.definition.loyalty {
                    let card_name_str = card.name.to_string();
                    if let Ok(card_mut) = self.game.cards.get_mut(spell_id) {
                        card_mut.add_counter(crate::core::CounterType::Loyalty, loyalty);
                    }
                    if should_log {
                        self.game.logger.gamelog(&format!(
                            "{} ({}) enters with {} loyalty",
                            card_name_str, spell_id, loyalty
                        ));
                    }
                }
            }
        }

        // Check state-based actions
        if let Err(e) = self.game.check_lethal_damage() {
            if should_log {
                eprintln!("    Failed to check lethal damage: {e}");
            }
        }
        if let Err(e) = self.game.check_legendary_rule() {
            if should_log {
                eprintln!("    Failed to check legendary rule: {e}");
            }
        }
        // Apply originally-printed-set state-trigger sweeps (City in a Bottle).
        if let Err(e) = self.game.check_set_origin_sacrifice() {
            if should_log {
                eprintln!("    Failed to check set-origin sacrifice: {e}");
            }
        }
        if let Err(e) = self.game.check_aura_attachment() {
            if should_log {
                eprintln!("    Failed to check aura attachment: {e}");
            }
        }
        if let Err(e) = self.game.recompute_aura_control() {
            if should_log {
                eprintln!("    Failed to recompute aura control: {e}");
            }
        }
        if let Err(e) = self.game.recompute_source_control() {
            if should_log {
                eprintln!("    Failed to recompute source control: {e}");
            }
        }
        if let Err(e) = self.game.check_equipment_attachment() {
            if should_log {
                eprintln!("    Failed to check equipment attachment: {e}");
            }
        }

        Ok(())
    }

    /// Resolve a Balance effect interactively, asking each player to choose sacrifices
    ///
    /// Balance equalizes permanents/cards of a specified type across all players.
    /// Each player with more than the minimum must choose which permanents to sacrifice.
    fn resolve_balance_effect_interactive(
        &mut self,
        card_type: &str,
        zone: &str,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        use crate::zones::Zone;

        let player_ids: Vec<_> = self.game.players.iter().map(|p| p.id).collect();

        if zone == "Hand" {
            // Hand balancing - use existing non-interactive implementation for now
            // TODO: Make hand discard interactive too
            self.game.execute_balance_effect(card_type, zone)?;
        } else {
            // Battlefield - interactive sacrifice
            // First, compute what each player needs to sacrifice
            let counts_and_permanents: Vec<_> = player_ids
                .iter()
                .map(|&pid| {
                    let matching_permanents: Vec<CardId> = self
                        .game
                        .battlefield
                        .cards
                        .iter()
                        .filter(|&&card_id| {
                            if let Some(card) = self.game.cards.try_get(card_id) {
                                if card.controller != pid {
                                    return false;
                                }
                                match card_type {
                                    "Creature" => card.is_creature(),
                                    "Land" => card.is_land(),
                                    "Artifact" => card.is_artifact(),
                                    "Enchantment" => card.is_enchantment(),
                                    "" => true,
                                    _ => true,
                                }
                            } else {
                                false
                            }
                        })
                        .copied()
                        .collect();
                    let count = matching_permanents.len();
                    (pid, count, matching_permanents)
                })
                .collect();

            let min_count = counts_and_permanents.iter().map(|(_, c, _)| *c).min().unwrap_or(0);

            // Log the balance action
            let type_str = if card_type.is_empty() { "permanents" } else { card_type };
            self.game
                .logger
                .gamelog(&format!("Balance: {} equalize to {}", type_str, min_count));

            // Process each player who needs to sacrifice
            for (player_id, current_count, valid_permanents) in counts_and_permanents {
                if current_count <= min_count {
                    continue; // This player doesn't need to sacrifice
                }

                let sacrifice_count = current_count - min_count;

                // Get the appropriate controller for this player
                let controller: &mut dyn PlayerController = if player_id == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Ask player to choose which permanents to sacrifice
                let prior_log_size = self.game.logger.log_count();

                let choice_result = self.choose_sacrifice_with_hook(
                    controller,
                    player_id,
                    &valid_permanents,
                    sacrifice_count,
                    type_str,
                );

                let to_sacrifice = handle_choice_result!(choice_result, self.game, player_id);

                // Log this choice point for snapshot/replay
                let replay_choice = crate::game::ReplayChoice::Sacrifice(to_sacrifice.clone());
                self.log_choice_point(player_id, Some(replay_choice), prior_log_size);

                // Verify correct count
                if to_sacrifice.len() != sacrifice_count {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Must sacrifice exactly {} {}, selected {}",
                        sacrifice_count,
                        type_str,
                        to_sacrifice.len()
                    )));
                }

                // Verify all selected permanents are valid
                for &perm_id in &to_sacrifice {
                    if !valid_permanents.contains(&perm_id) {
                        return Err(crate::MtgError::InvalidAction(format!(
                            "Selected permanent {:?} is not a valid {} to sacrifice",
                            perm_id, type_str
                        )));
                    }
                }

                // Sacrifice the selected permanents
                for card_id in to_sacrifice {
                    let owner = self.game.cards.get(card_id)?.owner;

                    // Log before moving
                    if let Some(card) = self.game.cards.try_get(card_id) {
                        let player_name = self
                            .game
                            .get_player(player_id)
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|_| "Player".to_string());
                        self.game
                            .logger
                            .gamelog(&format!("{} sacrifices {} to Balance", player_name, card.name));
                    }

                    // Check death triggers
                    let _ = self.game.check_death_triggers(card_id);

                    // Move to graveyard (or exile if finality counter)
                    let dest = self.game.death_destination_for_card(card_id);
                    self.game.move_card(card_id, Zone::Battlefield, dest, owner)?;
                }
            }
        }

        Ok(())
    }

    /// Resolve a Balance effect with SubAbility chaining
    ///
    /// This executes the Balance effect for the specified card_type/zone, then
    /// looks up and executes any chained SubAbility from the card's SVars.
    ///
    /// Example chain: Land → Hand → Creature
    /// - First Balance effect: Land on Battlefield (sub_ability: "BalanceHands")
    /// - Look up SVar "BalanceHands": "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures"
    /// - Execute: Hand balancing (sub_ability: "BalanceCreatures")
    /// - Look up SVar "BalanceCreatures": "DB$ Balance | Valid$ Creature"
    /// - Execute: Creature balancing (no sub_ability)
    fn resolve_balance_effect_chain(
        &mut self,
        card_type: &str,
        zone: &str,
        sub_ability: Option<&str>,
        svars: &std::collections::HashMap<String, String>,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Execute this Balance effect
        self.resolve_balance_effect_interactive(card_type, zone, controller1, controller2)?;

        // If there's a SubAbility reference, look it up and execute it
        if let Some(sub_ability_name) = sub_ability {
            if let Some(svar_body) = svars.get(sub_ability_name) {
                // Parse the SVar body to get the next Balance effect parameters
                // Format: "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures"
                // or "DB$ Balance | Valid$ Creature"
                if let Some((next_card_type, next_zone, next_sub_ability)) = Self::parse_balance_svar(svar_body) {
                    // Recursively execute the chained effect
                    self.resolve_balance_effect_chain(
                        &next_card_type,
                        &next_zone,
                        next_sub_ability.as_deref(),
                        svars,
                        controller1,
                        controller2,
                    )?;
                }
            }
        }

        Ok(())
    }

    /// Parse a Balance SVar body to extract card_type, zone, and sub_ability
    ///
    /// Input: "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures"
    /// Output: Some(("Land", "Hand", Some("BalanceCreatures")))
    ///
    /// Input: "DB$ Balance | Valid$ Creature"
    /// Output: Some(("Creature", "Battlefield", None))
    fn parse_balance_svar(svar_body: &str) -> Option<(String, String, Option<String>)> {
        // Only process Balance SVars
        if !svar_body.contains("DB$ Balance") && !svar_body.contains("DB$Balance") {
            return None;
        }

        // Parse parameters by splitting on |
        let mut card_type = "Land".to_string(); // Default for Balance
        let mut zone = "Battlefield".to_string(); // Default for Balance
        let mut sub_ability = None;

        for param in svar_body.split('|') {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('$') {
                match key.trim() {
                    "Valid" => card_type = value.trim().to_string(),
                    "Zone" => zone = value.trim().to_string(),
                    "SubAbility" => sub_ability = Some(value.trim().to_string()),
                    _ => {}
                }
            }
        }

        Some((card_type, zone, sub_ability))
    }
}

/// mtg-768 BLOCKER-1 regression: the discard handler must call the BLOCKING
/// `prepare_for_priority_choice()` (which, on a real network client, waits for
/// the server's discard ChoiceRequest) ONLY when a request will actually be
/// sent — i.e. only when `actual_count > 0`. On an empty hand the server sends
/// NO request, so blocking there would hang the client forever, and because
/// `take_local_choice` is a blind FIFO pop it could instead answer the NEXT
/// request to THIS choice → off-by-one FATAL desync (docs/NETWORK_ARCHITECTURE.md).
///
/// These tests drive the SPELL-resolution discard path
/// (`resolve_top_spell_with_discard_hook`, the path team-lead's "discard N
/// (choice) spell on the stack" describes) with a `RecordingNetController` that
/// reports itself as `ControllerType::Network` and counts every
/// `prepare_for_priority_choice()` call. The empty-hand guard lives in BOTH
/// discard sites; the activated-ability site shares the identical guard pattern
/// and is exercised (non-empty) by the rogerbrand desync canary.
///
/// PROVE-IT-BITES (done manually 2026-06-04, restored): moving the spell-path
/// `prepare_for_priority_choice()` ABOVE its `if actual_count > 0` guard makes
/// `empty_hand_discard_does_not_block_on_prepare` FAIL (prep count 1, expected
/// 0) — confirming the test pins the regression.
#[cfg(test)]
mod discard_prepare_ordering_tests {
    use super::*;
    use crate::core::{Card, CardType, ManaCost, SpellAbility};
    use crate::game::controller::ChoiceResult;
    use crate::game::snapshot::ControllerType;
    use crate::game::ZeroController;
    use crate::loader::CardDefinition;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Delegates every decision to an inner `ZeroController` but RECORDS each
    /// `prepare_for_priority_choice()` call and reports as a Network controller.
    struct RecordingNetController {
        inner: ZeroController,
        prepare_calls: Rc<Cell<u32>>,
    }

    impl RecordingNetController {
        fn new(player_id: PlayerId, prepare_calls: Rc<Cell<u32>>) -> Self {
            Self {
                inner: ZeroController::new(player_id),
                prepare_calls,
            }
        }
    }

    impl PlayerController for RecordingNetController {
        // --- the two methods that matter for this regression ---
        fn prepare_for_priority_choice(&mut self) -> bool {
            self.prepare_calls.set(self.prepare_calls.get() + 1);
            true
        }
        fn get_controller_type(&self) -> ControllerType {
            ControllerType::Network
        }

        // --- everything else delegates to the inner ZeroController ---
        fn player_id(&self) -> PlayerId {
            self.inner.player_id()
        }
        fn choose_spell_ability_to_play(
            &mut self,
            view: &GameStateView,
            available: &[SpellAbility],
        ) -> ChoiceResult<Option<SpellAbility>> {
            self.inner.choose_spell_ability_to_play(view, available)
        }
        fn choose_targets(
            &mut self,
            view: &GameStateView,
            spell: CardId,
            valid_targets: &[CardId],
            min_targets: usize,
            max_targets: usize,
        ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
            self.inner
                .choose_targets(view, spell, valid_targets, min_targets, max_targets)
        }
        fn choose_mana_sources_to_pay(
            &mut self,
            view: &GameStateView,
            cost: &ManaCost,
            available_sources: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
            self.inner.choose_mana_sources_to_pay(view, cost, available_sources)
        }
        fn choose_attackers(
            &mut self,
            view: &GameStateView,
            available_creatures: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
            self.inner.choose_attackers(view, available_creatures)
        }
        fn choose_blockers(
            &mut self,
            view: &GameStateView,
            available_blockers: &[CardId],
            attackers: &[CardId],
        ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
            self.inner.choose_blockers(view, available_blockers, attackers)
        }
        fn choose_damage_assignment_order(
            &mut self,
            view: &GameStateView,
            attacker: CardId,
            blockers: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
            self.inner.choose_damage_assignment_order(view, attacker, blockers)
        }
        fn choose_cards_to_discard(
            &mut self,
            view: &GameStateView,
            hand: &[CardId],
            count: usize,
        ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
            self.inner.choose_cards_to_discard(view, hand, count)
        }
        fn choose_from_library(
            &mut self,
            view: &GameStateView,
            valid_cards: &[&CardDefinition],
        ) -> ChoiceResult<Option<usize>> {
            self.inner.choose_from_library(view, valid_cards)
        }
        fn choose_permanents_to_sacrifice(
            &mut self,
            view: &GameStateView,
            valid_permanents: &[CardId],
            count: usize,
            card_type_description: &str,
        ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
            self.inner
                .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
        }
        fn choose_permanents_to_not_untap(
            &mut self,
            view: &GameStateView,
            may_not_untap_permanents: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
            self.inner
                .choose_permanents_to_not_untap(view, may_not_untap_permanents)
        }
        fn choose_modes(
            &mut self,
            view: &GameStateView,
            spell_id: CardId,
            mode_descriptions: &[String],
            mode_count: usize,
            min_modes: usize,
            can_repeat: bool,
        ) -> ChoiceResult<SmallVec<[usize; 4]>> {
            self.inner
                .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
        }
        fn on_priority_passed(&mut self, view: &GameStateView) {
            self.inner.on_priority_passed(view)
        }
        fn on_game_end(&mut self, view: &GameStateView, won: bool) {
            self.inner.on_game_end(view, won)
        }
    }

    /// Build a game where p0 owns a "you discard two cards (choose)" sorcery that
    /// is ON THE STACK, with `hand_size` cards already in p0's hand.
    fn setup(hand_size: usize) -> (GameState, CardId, PlayerId, PlayerId) {
        let mut game = GameState::new_two_player("P0".to_string(), "P1".to_string(), 20);
        let p0 = game.players[0].id;
        let p1 = game.players[1].id;

        for _ in 0..hand_size {
            let cid = game.next_card_id();
            let mut c = Card::new(cid, "Forest".to_string(), p0);
            c.add_type(CardType::Land);
            game.cards.insert(cid, c);
            game.get_player_zones_mut(p0).unwrap().hand.add(cid);
        }

        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Test Discard Two".to_string(), p0);
        spell.add_type(CardType::Sorcery);
        // count != u8::MAX -> the choice (TgtChoose) discard path; player is
        // concrete (self-target) so the handler discards from p0's hand.
        spell.effects.push(Effect::DiscardCards {
            player: p0,
            count: 2,
            remember_discarded: false,
            optional: false,
            remember_discarding_players: false,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        (game, spell_id, p0, p1)
    }

    #[test]
    fn empty_hand_discard_does_not_block_on_prepare() {
        // EMPTY hand -> actual_count == 0 -> server sends NO ChoiceRequest -> the
        // handler MUST NOT call the blocking prepare_for_priority_choice().
        let (mut game, spell_id, p0, p1) = setup(0);
        let prep = Rc::new(Cell::new(0u32));
        let mut c0 = RecordingNetController::new(p0, Rc::clone(&prep));
        let mut c1 = ZeroController::new(p1);
        {
            let mut gl = GameLoop::new(&mut game);
            gl.resolve_top_spell_with_discard_hook(spell_id, &mut c0, &mut c1)
                .expect("empty-hand discard spell should resolve cleanly");
        }
        assert_eq!(
            prep.get(),
            0,
            "empty-hand discard must NOT call prepare_for_priority_choice \
             (a real network client would hang / mis-pop the next request → FATAL desync)"
        );
    }

    #[test]
    fn nonempty_hand_discard_calls_prepare_once() {
        // NON-EMPTY hand -> a discard IS requested -> prepare is called exactly
        // once before the choice (proves the empty-hand guard did not over-gate
        // the normal prepare → sync → decide path).
        let (mut game, spell_id, p0, p1) = setup(3);
        let prep = Rc::new(Cell::new(0u32));
        let mut c0 = RecordingNetController::new(p0, Rc::clone(&prep));
        let mut c1 = ZeroController::new(p1);
        {
            let mut gl = GameLoop::new(&mut game);
            gl.resolve_top_spell_with_discard_hook(spell_id, &mut c0, &mut c1)
                .expect("non-empty discard spell should resolve cleanly");
        }
        assert_eq!(
            prep.get(),
            1,
            "non-empty discard must call prepare_for_priority_choice exactly once \
             (prepare → sync → decide)"
        );
    }
}
