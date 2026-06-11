//! Game actions and mechanics

mod effects;
mod triggers;

pub use targeting::is_legal_target;
pub use triggers::{resolve_effect_placeholder, TriggerContext};

use crate::core::{CardId, CardType, Effect, PlayerId, TargetRef, TriggerEvent};
use crate::game::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};

/// Pre-resolution snapshot of a chosen target's characteristics, captured BEFORE
/// any effect in a spell's resolution runs (CR 608.2g/2h — last-known
/// information). Used to lock dynamic-amount life-gain to the target's state as
/// it existed before the spell modified it:
/// - `power` / `mana_value` — Swords to Plowshares / Divine Offering.
/// - `drain_cap` — Drain Life's cap: the target's toughness (creature) /
///   loyalty (planeswalker) / life (player) before the damage was dealt.
#[derive(Debug, Clone, Copy)]
struct TargetSnapshot {
    power: i32,
    mana_value: i32,
    drain_cap: i32,
}

/// Expand effects with `ALL_PLAYERS_ID` player target into one effect per player.
/// For effects that don't use the all-players sentinel, returns the original effect unchanged.
fn expand_all_players_effect(effect: &Effect, player_ids: &[PlayerId]) -> smallvec::SmallVec<[Effect; 4]> {
    // Check if this effect uses the all-players sentinel on its player field
    let is_all_players = match effect {
        Effect::DrawCards { player, .. }
        | Effect::DrawCardsXPaid { player, .. }
        | Effect::DiscardCards { player, .. }
        | Effect::DiscardCardsXPaid { player, .. }
        | Effect::GainLife { player, .. }
        | Effect::LoseLife { player, .. }
        | Effect::ForceSacrifice { player, .. }
        | Effect::SetLife { player, .. }
        | Effect::Mill { player, .. } => player.is_all_players(),
        // All other effect variants don't have an expandable player field
        Effect::DealDamage { .. }
        | Effect::DealDamageXPaid { .. }
        | Effect::DealDamageDivided { .. }
        | Effect::DealDamageDynamic { .. }
        | Effect::DealDamageToTriggeredPlayer { .. }
        | Effect::EachDamage { .. }
        | Effect::Loot { .. }
        | Effect::DestroyPermanent { .. }
        | Effect::DestroyAll { .. }
        | Effect::SacrificeAll { .. }
        | Effect::DamageAll { .. }
        | Effect::TapAll { .. }
        | Effect::UntapAll { .. }
        | Effect::GainControl { .. }
        | Effect::Fight { .. }
        | Effect::TapPermanent { .. }
        | Effect::UntapPermanent { .. }
        | Effect::TapOrUntapPermanent { .. }
        | Effect::PumpCreature { .. }
        | Effect::DebuffCreature { .. }
        | Effect::PumpAllCreatures { .. }
        | Effect::AnimateAll { .. }
        | Effect::PumpCreatureVariable { .. }
        | Effect::Scry { .. }
        | Effect::Surveil { .. }
        | Effect::DrainMana { .. }
        | Effect::CounterSpell { .. }
        | Effect::AddMana { .. }
        | Effect::PutCounter { .. }
        | Effect::MultiplyCounter { .. }
        | Effect::PutCounterAll { .. }
        | Effect::Proliferate
        | Effect::ChangeZoneAll { .. }
        | Effect::RemoveCounter { .. }
        | Effect::ExilePermanent { .. }
        | Effect::ExileIfWouldDieThisTurn { .. }
        | Effect::SearchLibrary { .. }
        | Effect::AttachEquipment { .. }
        | Effect::CreateToken { .. }
        | Effect::CopyPermanent { .. }
        | Effect::Balance { .. }
        | Effect::SetBasePowerToughness { .. }
        | Effect::Airbend { .. }
        | Effect::Earthbend { .. }
        | Effect::Firebend { .. }
        | Effect::GrantCantBeBlocked { .. }
        | Effect::Regenerate { .. }
        | Effect::PreventDamage { .. }
        | Effect::PreventDamageFromSource { .. }
        | Effect::ModalChoice { .. }
        | Effect::Dig { .. }
        | Effect::CreateDelayedTrigger { .. }
        | Effect::CopySpellAbility { .. }
        | Effect::ImmediateTrigger { .. }
        | Effect::ClearRemembered
        | Effect::AddTurn { .. }
        | Effect::AddPhase { .. }
        | Effect::ChooseColor { .. }
        | Effect::Clone { .. }
        | Effect::SelfExileFromStack { .. }
        | Effect::MoveSelfBetweenZones { .. }
        | Effect::ReturnCardsFromGraveyardToHand { .. }
        | Effect::ReturnGraveyardCardToHand { .. }
        | Effect::PreventAllCombatDamageThisTurn { .. }
        | Effect::ConditionalSelfCounter { .. }
        | Effect::Unimplemented { .. }
        | Effect::NoOp { .. }
        | Effect::GainLifeDynamic { .. }
        | Effect::ClassLevelUp { .. }
        | Effect::UnlessCostWrapper { .. } => false,
    };

    if !is_all_players {
        return smallvec::smallvec![effect.clone()];
    }

    // Expand: create one effect per player
    player_ids
        .iter()
        .map(|&pid| match effect {
            Effect::DrawCards { count, .. } => Effect::DrawCards {
                player: pid,
                count: *count,
            },
            Effect::DiscardCards {
                count,
                remember_discarded,
                optional,
                remember_discarding_players,
                ..
            } => Effect::DiscardCards {
                player: pid,
                count: *count,
                remember_discarded: *remember_discarded,
                optional: *optional,
                remember_discarding_players: *remember_discarding_players,
            },
            Effect::DrawCardsXPaid { .. } => Effect::DrawCardsXPaid { player: pid },
            Effect::DiscardCardsXPaid { remember_discarded, .. } => Effect::DiscardCardsXPaid {
                player: pid,
                remember_discarded: *remember_discarded,
            },
            Effect::GainLife { amount, .. } => Effect::GainLife {
                player: pid,
                amount: *amount,
            },
            Effect::LoseLife { amount, .. } => Effect::LoseLife {
                player: pid,
                amount: *amount,
            },
            Effect::Mill { count, .. } => Effect::Mill {
                player: pid,
                count: *count,
            },
            Effect::ForceSacrifice { sac_type, count, .. } => Effect::ForceSacrifice {
                player: pid,
                sac_type: sac_type.clone(),
                count: *count,
            },
            Effect::SetLife { amount, .. } => Effect::SetLife {
                player: pid,
                amount: *amount,
            },
            // Unreachable: is_all_players only true for player-targeted variants.
            Effect::DealDamage { .. }
            | Effect::DealDamageXPaid { .. }
            | Effect::DealDamageDivided { .. }
            | Effect::DealDamageDynamic { .. }
            | Effect::DealDamageToTriggeredPlayer { .. }
            | Effect::EachDamage { .. }
            | Effect::Loot { .. }
            | Effect::DestroyPermanent { .. }
            | Effect::DestroyAll { .. }
            | Effect::SacrificeAll { .. }
            | Effect::DamageAll { .. }
            | Effect::TapAll { .. }
            | Effect::UntapAll { .. }
            | Effect::GainControl { .. }
            | Effect::Fight { .. }
            | Effect::TapPermanent { .. }
            | Effect::UntapPermanent { .. }
            | Effect::TapOrUntapPermanent { .. }
            | Effect::PumpCreature { .. }
            | Effect::DebuffCreature { .. }
            | Effect::PumpAllCreatures { .. }
            | Effect::AnimateAll { .. }
            | Effect::PumpCreatureVariable { .. }
            | Effect::Scry { .. }
            | Effect::Surveil { .. }
            | Effect::DrainMana { .. }
            | Effect::CounterSpell { .. }
            | Effect::AddMana { .. }
            | Effect::PutCounter { .. }
            | Effect::MultiplyCounter { .. }
            | Effect::PutCounterAll { .. }
            | Effect::Proliferate
            | Effect::ChangeZoneAll { .. }
            | Effect::RemoveCounter { .. }
            | Effect::ExilePermanent { .. }
            | Effect::ExileIfWouldDieThisTurn { .. }
            | Effect::SearchLibrary { .. }
            | Effect::AttachEquipment { .. }
            | Effect::CreateToken { .. }
            | Effect::CopyPermanent { .. }
            | Effect::Balance { .. }
            | Effect::SetBasePowerToughness { .. }
            | Effect::Airbend { .. }
            | Effect::Earthbend { .. }
            | Effect::Firebend { .. }
            | Effect::GrantCantBeBlocked { .. }
            | Effect::Regenerate { .. }
            | Effect::PreventDamage { .. }
            | Effect::PreventDamageFromSource { .. }
            | Effect::ModalChoice { .. }
            | Effect::Dig { .. }
            | Effect::CreateDelayedTrigger { .. }
            | Effect::CopySpellAbility { .. }
            | Effect::ImmediateTrigger { .. }
            | Effect::ClearRemembered
            | Effect::AddTurn { .. }
            | Effect::AddPhase { .. }
            | Effect::ChooseColor { .. }
            | Effect::Clone { .. }
            | Effect::SelfExileFromStack { .. }
            | Effect::MoveSelfBetweenZones { .. }
            | Effect::ReturnCardsFromGraveyardToHand { .. }
            | Effect::ReturnGraveyardCardToHand { .. }
            | Effect::PreventAllCombatDamageThisTurn { .. }
            | Effect::ConditionalSelfCounter { .. }
            | Effect::Unimplemented { .. }
            | Effect::NoOp { .. }
            | Effect::GainLifeDynamic { .. }
            | Effect::ClassLevelUp { .. }
            | Effect::UnlessCostWrapper { .. } => unreachable!(),
        })
        .collect()
}

/// Predicate for `CostReductionTarget` against a spell's card.
///
/// Centralized so ReduceCost (lines ~1313+) and RaiseCost (~1359+) can share
/// the same match logic. New `CostReductionTarget` variants only need to be
/// handled in one place.
pub(crate) fn spell_matches_cost_filter(
    card: &crate::core::Card,
    valid_card: &crate::core::CostReductionTarget,
) -> bool {
    use crate::core::CostReductionTarget;
    match valid_card {
        CostReductionTarget::AllSpells => true,
        CostReductionTarget::NonCreature => !card.is_creature(),
        CostReductionTarget::Creature => card.is_creature(),
        CostReductionTarget::Subtype(subtype) => card.subtypes.contains(subtype),
        CostReductionTarget::Color(color) => card.is_color(*color),
    }
}

impl GameState {
    /// Play a land from hand to battlefield
    ///
    /// Per NETWORK_ARCHITECTURE.md, cards are revealed to ALL players before moving
    /// to battlefield (which is a public zone).
    ///
    /// # Errors
    ///
    /// Returns an error if the player cannot play more lands, the card is not a land,
    /// or the card is not in hand.
    pub fn play_land(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Check if player can play a land
        let player = self.get_player(player_id)?;
        if !player.can_play_land() {
            return Err(MtgError::InvalidAction("Cannot play more lands this turn".to_string()));
        }

        // Check if card is a land and in hand
        let card = self.cards.get(card_id)?;
        if !card.is_land() {
            return Err(MtgError::InvalidAction("Card is not a land".to_string()));
        }

        // Check if in hand
        if let Some(zones) = self.get_player_zones(player_id) {
            if !zones.hand.contains(card_id) {
                return Err(MtgError::InvalidAction("Card not in hand".to_string()));
            }
        }

        // Move card to battlefield (move_card logs the MoveCard action + auto-reveals)
        self.move_card(card_id, Zone::Hand, Zone::Battlefield, player_id)?;

        // Record the turn number when this land entered the battlefield
        if let Ok(card) = self.cards.get_mut(card_id) {
            // Capture old value and log size before mutation
            let old_value = card.turn_entered_battlefield;
            let prior_log_size = self.logger.log_count();

            let new_value = Some(self.turn.turn_number);
            card.turn_entered_battlefield = new_value;

            // Log the mutation for undo
            self.undo_log.log(
                crate::undo::GameAction::SetTurnEnteredBattlefield {
                    card_id,
                    old_value,
                    new_value,
                },
                prior_log_size,
            );
        }

        // Increment lands played
        // Capture old value and log size before mutation (before get_player_mut to avoid borrow issues)
        let old_value = self.get_player(player_id)?.lands_played_this_turn;
        let prior_log_size = self.logger.log_count();

        let player = self.get_player_mut(player_id)?;
        player.play_land();
        let new_value = player.lands_played_this_turn;

        // Log the mutation for undo
        self.undo_log.log(
            crate::undo::GameAction::SetLandsPlayedThisTurn {
                player_id,
                old_value,
                new_value,
            },
            prior_log_size,
        );

        // Apply etbCounter keyword (CR 614.1c self-replacement) before triggers fire
        self.apply_etb_counters(card_id)?;

        // Check ETB triggers (including landfall triggers on other permanents)
        self.check_triggers(TriggerEvent::EntersBattlefield, card_id)?;

        Ok(())
    }

    /// Begin casting the Adventure (instant/sorcery) half of an Adventurer card
    /// (CR 715). Swaps the card's live spell-relevant characteristics — name,
    /// mana cost, types, subtypes, colors, P/T, oracle text, parsed effects /
    /// triggers / abilities, and the embedded definition — to the Adventure
    /// face, then sets `cast_as_adventure`. The standard `CastSpell` pipeline
    /// runs next on the swapped card, so cost / targeting / modal / X / resolution
    /// are all shared with normal spell casting.
    ///
    /// REWIND SAFETY: the creature face is captured in a `CardStateSnapshot` and
    /// logged as `GameAction::RestoreCardState` BEFORE the swap, exactly like a
    /// card leaving the battlefield. A rewind that unwinds past this point
    /// restores the creature face (and `cast_as_adventure = false`) bit-identically.
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not found or has no Adventure face.
    pub fn begin_adventure_cast(&mut self, card_id: CardId) -> Result<()> {
        // Pull the Adventure-face definition out of the card's definition.
        let adventure_def = {
            let card = self.cards.get(card_id)?;
            match card.definition.adventure.as_deref() {
                Some(def) => def.clone(),
                None => {
                    return Err(MtgError::InvalidAction("Card has no Adventure face".to_string()));
                }
            }
        };

        // Instantiate the Adventure face transiently to obtain its parsed spell
        // characteristics (effects/triggers/abilities) without duplicating the
        // ability-parsing logic. Owner is copied from the live card.
        let owner = self.cards.get(card_id)?.owner;
        let adventure_card = adventure_def.instantiate(card_id, owner);

        // Snapshot the CURRENT (creature) state and log it for undo BEFORE the
        // swap (same contract as a battlefield-leave reset).
        let card = self.cards.get_mut(card_id)?;
        let snapshot = card.capture_state_snapshot();
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::RestoreCardState {
                card_id,
                snapshot: Box::new(snapshot),
            },
            prior_log_size,
        );

        // Preserve the creature (front) definition so the resolution path can
        // restore it WITHOUT depending on a `card_definitions` lookup (puzzles and
        // tests do not always populate that map). We stash it on the Adventure
        // face's own `adventure` slot — symmetric to the creature carrying the
        // Adventure face — so the swapped card is fully self-describing.
        let creature_definition = self.cards.get(card_id)?.definition.clone();
        let mut adventure_definition = adventure_card.definition.clone();
        adventure_definition.adventure = Some(Box::new(creature_definition));

        // Apply the Adventure face's spell-relevant characteristics. The creature
        // face is preserved only in the snapshot above; on resolution the card is
        // exiled and the creature face restored via that snapshot's `definition`.
        let card = self.cards.get_mut(card_id)?;
        // Move the Adventure face's fields onto the live card (adventure_card is
        // owned and discarded after this, so no clones are needed).
        card.name = adventure_card.name;
        card.mana_cost = adventure_card.mana_cost;
        card.types = adventure_card.types;
        card.subtypes = adventure_card.subtypes;
        card.colors = adventure_card.colors;
        card.text = adventure_card.text;
        card.keywords = adventure_card.keywords;
        card.effects = adventure_card.effects;
        card.triggers = adventure_card.triggers;
        card.activated_abilities = adventure_card.activated_abilities;
        card.static_abilities = adventure_card.static_abilities;
        card.svars = adventure_card.svars;
        // The creature's printed P/T must not show while on the Adventure (the
        // Adventure face is an instant/sorcery with no P/T).
        card.set_base_power(adventure_def.power);
        card.set_base_toughness(adventure_def.toughness);
        // Swap the embedded definition so cost/cache/target lookups read the
        // Adventure face. `is_adventure_face` is already true on adventure_def;
        // `adventure_definition.adventure` carries the creature def for restore.
        card.definition = adventure_definition;
        card.cast_as_adventure = true;

        Ok(())
    }

    /// Cast a spell (put it on the stack)
    ///
    /// This validates mana payment and deducts the cost from the player's mana pool.
    ///
    /// Per NETWORK_ARCHITECTURE.md, cards are revealed to ALL players before moving
    /// to stack (which is a public zone).
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not in hand or if insufficient mana to pay the cost.
    pub fn cast_spell(&mut self, player_id: PlayerId, card_id: CardId, _targets: Vec<CardId>) -> Result<()> {
        // Check if card is in hand
        if let Some(zones) = self.get_player_zones(player_id) {
            if !zones.hand.contains(card_id) {
                return Err(MtgError::InvalidAction("Card not in hand".to_string()));
            }
        }

        // Get the mana cost (need to do this before mutable borrow)
        let mana_cost = {
            let card = self.cards.get(card_id)?;
            card.mana_cost
        };

        // Pay the mana cost (from both regular and combat mana pools).
        // Snapshot combat mana for undo when it will be spent (mtg-ba6uq #7).
        if self
            .get_player(player_id)
            .map(|p| p.combat_mana_pool.is_some())
            .unwrap_or(false)
        {
            self.log_combat_mana_pool(player_id);
        }
        // Snapshot the regular mana pool for undo before the payment (mtg-733).
        self.log_mana_pool(player_id);
        let player = self.get_player_mut(player_id)?;
        player
            .pay_from_total_mana(&mana_cost)
            .map_err(MtgError::InvalidAction)?;

        // Move card to stack (move_card logs the MoveCard action + auto-reveals)
        self.move_card(card_id, Zone::Hand, Zone::Stack, player_id)?;

        Ok(())
    }

    /// Resolve a spell from the stack
    ///
    /// ## Parameters
    /// - `card_id`: The spell card on the stack to resolve
    /// - `chosen_targets`: Targets selected by the controller during casting (optional)
    ///
    /// If targets are provided, they will be used to fill in placeholder targets in effects.
    /// Otherwise, effects must already have their targets specified.
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not found or if spell resolution fails.
    pub fn resolve_spell(&mut self, card_id: CardId, chosen_targets: &[CardId]) -> Result<()> {
        self.resolve_spell_execute_effects(card_id, chosen_targets)?;
        self.resolve_spell_finalize(card_id, chosen_targets)
    }

    /// Execute the effects of a resolving spell (target resolution + effect execution).
    ///
    /// This is the first phase of spell resolution: resolve placeholder targets
    /// and execute each effect. Separated from `resolve_spell_finalize` so that
    /// the game loop can intercept specific effects (e.g., discard choices that
    /// need to go through the controller protocol for network play).
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not found or if effect execution fails.
    pub fn resolve_spell_execute_effects(&mut self, card_id: CardId, chosen_targets: &[CardId]) -> Result<()> {
        // Get card owner and effects count (without cloning effects)
        let (card_owner, effects_len) = {
            let card = self.cards.get(card_id)?;
            (card.owner, card.effects.len())
        };

        log::debug!(target: "resolve_spell", "resolve_spell card_id={}, chosen_targets={:?}, effects_len={}", card_id.as_u32(), chosen_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>(), effects_len);

        // Find opponent ID for untargeted damage (resolve once)
        let opponent_id = self.players.iter().map(|p| p.id).find(|id| *id != card_owner);

        // Check if targets are still valid before executing effects
        // MTG Rules 608.2b: If all targets are illegal, the spell doesn't resolve
        // A target is legal if it is on the battlefield/stack OR is a player-
        // target sentinel (e.g. Lightning Bolt aimed at a player) — players
        // can't leave the game during normal play except via state-based
        // actions checked separately.
        let all_targets_illegal = if !chosen_targets.is_empty() {
            let any_permanent_gone = chosen_targets.iter().any(|&target_id| {
                !self.battlefield.contains(target_id)
                    && !self.stack.contains(target_id)
                    && crate::core::player_target_from_sentinel(target_id).is_none()
            });

            // If spell has targets and they're all gone, it fizzles
            any_permanent_gone
        } else {
            false
        };

        // Execute effects only if targets are still valid
        if !all_targets_illegal {
            // Read x_paid once for resolving XPaid effect variants
            let x_paid = self.cards.get(card_id).map(|c| c.x_paid).unwrap_or(0);

            // Snapshot each target's dynamic characteristics (power / mana value)
            // BEFORE any effect runs (CR 608.2g/2h). A chained GainLifeDynamic
            // (Swords to Plowshares, Divine Offering) must use the targeted
            // permanent's power / mana value *as it last existed on the
            // battlefield* — i.e. with its continuous buffs still applied —
            // even though the preceding exile/destroy effect removes those
            // buffs by the time GainLifeDynamic resolves.
            let target_snapshots = self.snapshot_target_amounts(chosen_targets);

            // Begin accumulating non-combat damage dealt by THIS source across
            // all of its effects, so the "whenever ~ deals damage" trigger
            // (Spirit Link, CR 119.3) fires once with the aggregated total
            // rather than once per target (mtg-r9po1). Mirrors the combat path,
            // which fires once per creature-damage event.
            self.damage_dealt_by_source = Some(0);

            // Execute effects by index, resolving targets at execution time
            // This avoids cloning the entire Vec<Effect>
            let mut target_index = 0;
            let mut last_resolved_target: Option<CardId> = None;
            for effect_index in 0..effects_len {
                // Re-fetch effect each iteration (card ref can't be held across execute calls)
                let effect = self.cards.get(card_id)?.effects.get(effect_index).cloned();

                // Resolve XPaid variants to concrete amounts
                let effect = effect.map(|e| Self::resolve_x_paid_effect(e, x_paid));

                if let Some(effect) = effect {
                    log::debug!(target: "resolve_spell", "Effect[{}] before resolve: {:?}", effect_index, effect);
                    // Resolve the effect with context, advancing target_index as needed
                    let resolved = self.resolve_effect_target(
                        &effect,
                        chosen_targets,
                        &mut target_index,
                        card_owner,
                        opponent_id,
                        &mut last_resolved_target,
                    );
                    log::debug!(target: "resolve_spell", "Effect[{}] after resolve: {:?}", effect_index, resolved);

                    // Patch up ChooseColor source placeholder with the spell's card_id
                    let resolved = Self::resolve_choose_color_source(resolved, card_id);

                    // Resolve Defined$ Self sentinels to the source card
                    let resolved = Self::resolve_self_target(resolved, card_id);

                    // Patch a GainControl's source with the resolving card so a
                    // WhileControlSource duration (Aladdin) can be tracked.
                    let resolved = Self::resolve_gain_control_source(resolved, card_id);

                    // Lock a GainLifeDynamic's amount to the pre-resolution
                    // snapshot of its reference card (last-known information).
                    let resolved = Self::resolve_dynamic_gainlife_snapshot(resolved, &target_snapshots);

                    // Expand "all players" effects (e.g., Wheel of Fortune: each player discards/draws)
                    let player_ids: smallvec::SmallVec<[PlayerId; 4]> = self.players.iter().map(|p| p.id).collect();
                    let expanded = expand_all_players_effect(&resolved, &player_ids);
                    // The resolving spell is the source of any damage it deals,
                    // so source-filtered prevention (Circle of Protection) can
                    // match it (CR 609.7, 615.6). Cleared after execution.
                    self.current_damage_source = Some(card_id);
                    let mut exec_result = Ok(());
                    for e in &expanded {
                        if let Err(err) = self.execute_effect(e) {
                            exec_result = Err(err);
                            break;
                        }
                    }
                    self.current_damage_source = None;
                    exec_result?;
                }
            }

            // Fire the non-combat "whenever ~ deals damage" trigger ONCE for the
            // whole resolution with the aggregated amount (Spirit Link's
            // DamageDealtOnce, CR 119.3). `card_id` is the source on the stack
            // (the resolving spell, or the creature whose activated/triggered
            // ability is resolving — e.g. an enchanted pinger). The trigger is
            // the SAME DealsCombatDamage event the combat path uses and routes
            // through the shared check_triggers_with_damage -> check_triggers_inner
            // path, so the trigger-filter machinery (requires_attached_source for
            // Spirit Link, requires_combat_damage gating, TriggerCount$DamageAmount)
            // is identical on native, WASM, and network. Combat damage does NOT
            // double-fire here because it sets no source accumulator (the field is
            // cleared to None below before control returns).
            let dealt = self.damage_dealt_by_source.take().unwrap_or(0);
            if dealt > 0 {
                self.check_triggers_with_damage(TriggerEvent::DealsCombatDamage, card_id, Some(dealt))?;
            }
        }

        Ok(())
    }

    /// Replace XPaid effect variants with their concrete-amount equivalents.
    /// Called at resolution time when the spell's x_paid value is known.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_x_paid_effect(effect: Effect, x_paid: u8) -> Effect {
        match effect {
            // DivideEvenly$ RoundedDown (Fireball, CR 601.2d): carry the TOTAL X
            // through as a target-less DealDamageDivided whose `amount_each`
            // temporarily holds the un-divided total. resolve_effect_target then
            // gathers the N chosen targets and rewrites amount_each = floor(X/N).
            // Runs BEFORE resolve_effect_target, so it must not collapse to a
            // single target here.
            Effect::DealDamageXPaid {
                target: TargetRef::None,
                divide: crate::core::DamageDivision::EvenlyRoundedDown,
            } => Effect::DealDamageDivided {
                targets: smallvec::SmallVec::new(),
                amount_each: i32::from(x_paid),
            },
            // Single-target X-damage. The single concrete target is resolved
            // afterwards in resolve_effect_target; deal the full amount.
            Effect::DealDamageXPaid { target, .. } => Effect::DealDamage {
                target,
                amount: i32::from(x_paid),
            },
            Effect::DrawCardsXPaid { player } => Effect::DrawCards { player, count: x_paid },
            Effect::DiscardCardsXPaid {
                player,
                remember_discarded,
            } => Effect::DiscardCards {
                player,
                count: x_paid,
                remember_discarded,
                optional: false,
                remember_discarding_players: false,
            },
            other => other,
        }
    }

    /// Replace ChooseColor source placeholder with the actual spell card_id.
    /// Called at resolution time when the source card is known.
    #[inline]
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_choose_color_source(effect: Effect, source_card_id: CardId) -> Effect {
        match effect {
            Effect::ChooseColor { player, source } if source.is_placeholder() => Effect::ChooseColor {
                player,
                source: source_card_id,
            },
            other => other,
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_self_target(effect: Effect, source_card_id: CardId) -> Effect {
        match effect {
            Effect::DestroyPermanent {
                target,
                restriction,
                no_regenerate,
            } if target.is_self_target() => Effect::DestroyPermanent {
                target: source_card_id,
                restriction,
                no_regenerate,
            },
            // `SP$ ChangeZone | Origin$ Stack | Destination$ Exile` — patch in
            // the resolving spell's CardId so `Effect::SelfExileFromStack` can
            // move *this* card from the stack to exile.
            Effect::SelfExileFromStack {
                source,
                remember_changed,
            } if source.is_self_target() => Effect::SelfExileFromStack {
                source: source_card_id,
                remember_changed,
            },
            // `DB$ PutCounter | Defined$ Self` chained on a SP$ — patch in the
            // resolving spell's CardId so the counters land on the source card.
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } if target.is_self_target() => Effect::PutCounter {
                target: source_card_id,
                counter_type,
                amount,
            },
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } if target.is_self_target() => Effect::RemoveCounter {
                target: source_card_id,
                counter_type,
                amount,
            },
            Effect::MoveSelfBetweenZones {
                source,
                origin,
                destination,
            } if source.is_self_target() => Effect::MoveSelfBetweenZones {
                source: source_card_id,
                origin,
                destination,
            },
            Effect::ConditionalSelfCounter {
                source,
                condition,
                inner,
            } => Effect::ConditionalSelfCounter {
                source: if source.is_self_target() {
                    source_card_id
                } else {
                    source
                },
                condition,
                // Recurse so a `Defined$ Self` inner effect (MoveSelfBetweenZones,
                // RemoveCounter, …) is also patched to the source CardId.
                inner: Box::new(Self::resolve_self_target(*inner, source_card_id)),
            },
            other => other,
        }
    }

    /// Patch a one-shot `AB$ GainControl`'s `source` with the resolving card so a
    /// [`ControlDuration::WhileControlSource`] grant (Aladdin) knows which
    /// permanent's continued control sustains it. Mirrors [`Self::resolve_self_target`].
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_gain_control_source(effect: Effect, source_card_id: CardId) -> Effect {
        match effect {
            Effect::GainControl {
                target,
                new_controller,
                untap,
                duration,
                restriction,
                source: None,
            } => Effect::GainControl {
                target,
                new_controller,
                untap,
                duration,
                restriction,
                source: Some(source_card_id),
            },
            other => other,
        }
    }

    /// Finalize a spell after its effects have executed.
    ///
    /// This is the second phase of spell resolution: move the card from the stack
    /// to its destination (graveyard for instants/sorceries, battlefield for
    /// permanents), handle ETB triggers, and attach Auras.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found or moved.
    pub fn resolve_spell_finalize(&mut self, card_id: CardId, chosen_targets: &[CardId]) -> Result<()> {
        let card_owner = self.cards.get(card_id)?.owner;

        // CR 715.3d: when an Adventure (instant/sorcery) spell resolves, instead
        // of going to the graveyard it is EXILED "on an adventure", and its owner
        // may cast the creature half from exile. Handled here as a dedicated
        // finalize path that reuses the exile + MayPlayFromExile machinery.
        let is_adventure_spell = {
            let card = self.cards.get(card_id)?;
            card.cast_as_adventure
        };
        if is_adventure_spell && self.stack.contains(card_id) {
            return self.finalize_adventure_spell(card_id);
        }

        // Determine destination based on card type
        let destination = {
            let card = self.cards.get(card_id)?;
            if card.is_type(&CardType::Instant) || card.is_type(&CardType::Sorcery) {
                Zone::Graveyard
            } else {
                Zone::Battlefield
            }
        };

        // If the card already left the stack during effect execution (e.g. an
        // `SP$ ChangeZone | Origin$ Stack | Destination$ Exile` self-exile —
        // All Hallow's Eve), don't try to move it again. The default
        // sorcery-resolution path always sends sorceries to the graveyard,
        // which would clobber the self-exile that the effect just performed.
        // Checking `stack.contains` here lets `Effect::SelfExileFromStack`
        // (and any future effect that relocates the resolving spell) override
        // the default destination cleanly.
        if !self.stack.contains(card_id) {
            log::debug!(
                target: "resolve_spell",
                "resolve_spell_finalize: card {} no longer on stack (effect moved it), skipping default move-to-{:?}",
                card_id.as_u32(),
                destination
            );
            return Ok(());
        }

        // CR 707.10c / 111.7: a copy of a non-permanent spell (created by
        // `copy_spell_onto_stack`, flagged `is_token`) is a game object that
        // ceases to exist as a state-based action once it leaves the stack — it
        // does NOT go to the GRAVEYARD as a phantom card (which would wrongly
        // feed graveyard-count / flashback / threshold effects). Entities are
        // never deleted from the store in this engine (see EntityStore::remove),
        // so the closest faithful sink is Exile: the copy leaves the stack and
        // is gone from play. (A copy of a PERMANENT spell resolves to the
        // battlefield as a normal token, so we only intercept the
        // would-go-to-graveyard case.)
        let is_spell_copy =
            destination == Zone::Graveyard && self.cards.get(card_id).map(|c| c.is_token).unwrap_or(false);
        if is_spell_copy {
            log::debug!(
                target: "resolve_spell",
                "resolve_spell_finalize: spell copy {} ceases to exist (CR 707.10c) -> Exile",
                card_id.as_u32()
            );
            self.sub_action_scratch.spell_targets.retain(|(id, _)| *id != card_id);
            let owner = self.cards.get(card_id)?.owner;
            self.move_card(card_id, Zone::Stack, Zone::Exile, owner)?;
            return Ok(());
        }

        // Move card from stack to destination
        let owner = self.cards.get(card_id)?.owner;
        self.move_card(card_id, Zone::Stack, destination, owner)?;

        // If it entered the battlefield, record the turn number (for summoning sickness)
        if destination == Zone::Battlefield {
            if let Ok(card) = self.cards.get_mut(card_id) {
                // Capture old value and log size before mutation
                let old_value = card.turn_entered_battlefield;
                let prior_log_size = self.logger.log_count();

                let new_value = Some(self.turn.turn_number);
                card.turn_entered_battlefield = new_value;

                // Log the mutation for undo
                self.undo_log.log(
                    crate::undo::GameAction::SetTurnEnteredBattlefield {
                        card_id,
                        old_value,
                        new_value,
                    },
                    prior_log_size,
                );
            }

            // MTG Rule 303.4a: An Aura spell that resolves attaches to its target
            // The target was already chosen and validated when casting the Aura
            let is_aura = self.cards.get(card_id).map(|c| c.is_aura()).unwrap_or(false);
            if is_aura && !chosen_targets.is_empty() {
                let aura_target = chosen_targets[0];
                // Attach the Aura to its target (if target is still valid)
                if self.battlefield.contains(aura_target) {
                    self.attach_aura(card_id, aura_target)?;
                } else if self.find_card_zone(aura_target) == Some(Zone::Graveyard) {
                    // Reanimation Aura (e.g. Animate Dead — `K:Enchant:Creature.inZoneGraveyard`).
                    // The Aura's "ETB if it's on the battlefield" trigger normally walks the
                    // SVar chain `TrigReanimate → DBAnimate → DBAttach → DBDelay`, but several
                    // of those API stops (DB$ ChangeZone Graveyard→Battlefield with
                    // `Defined$ Enchanted`, DB$ Attach with `Defined$ Remembered`) are not yet
                    // implemented in the effect converter — see mtg-400. We get the same
                    // user-visible outcome (the chosen graveyard creature comes back under
                    // our control with the Aura on it, applying its continuous -1/-0 via the
                    // existing `Affected$ Creature.EnchantedBy` static-effect path) by
                    // inlining the reanimation here.
                    //
                    // Caveats not yet handled (tracked in mtg-400):
                    //   * The "when CARDNAME leaves the battlefield, that creature's
                    //     controller sacrifices it" delayed trigger (DBDelay) is skipped.
                    //   * The keyword swap (RemoveKeywords$/Keywords$ Enchant) that rewrites
                    //     the enchant restriction so the Aura survives normal Aura-attachment
                    //     SBA after the target moves to battlefield is also skipped — the
                    //     immediate `attach_aura` below points the Aura at a battlefield
                    //     creature, which keeps the SBA happy in practice.
                    self.reanimate_aura_target(card_id, aura_target, card_owner)?;
                } else {
                    // Target became invalid - move Aura to graveyard (CR 303.4a)
                    self.move_card(card_id, Zone::Battlefield, Zone::Graveyard, card_owner)?;
                }
            }

            // Apply etbCounter keyword (CR 614.1c self-replacement) before triggers fire
            self.apply_etb_counters(card_id)?;

            // Check for ETB triggers on all permanents (including the one that just entered)
            self.check_triggers(TriggerEvent::EntersBattlefield, card_id)?;
        }

        Ok(())
    }

    /// Finalize an Adventure spell that has finished resolving (CR 715.3d):
    /// exile the card "on an adventure" instead of sending it to the graveyard,
    /// restore the creature face (so the exiled card is the creature card), and
    /// grant the owner permission to cast the creature half from exile for its
    /// printed mana cost. The permission ends when the card leaves exile (cast
    /// to the stack, or removed by another effect) via `TrackedCardLeavesZone`.
    ///
    /// Reuses the same exile + `MayPlayFromExile` machinery as Airbend/Suspend.
    fn finalize_adventure_spell(&mut self, card_id: CardId) -> Result<()> {
        let owner = self.cards.get(card_id)?.owner;

        // Restore the creature face from the creature definition stashed on the
        // Adventure face during `begin_adventure_cast` (self-contained; no
        // dependency on a populated `card_definitions` map). Falls back to the
        // printed-name lookup if for some reason the stash is absent.
        let creature_def = self
            .cards
            .get(card_id)?
            .definition
            .adventure
            .as_deref()
            .cloned()
            .or_else(|| {
                self.card_definitions
                    .get(&self.cards.get(card_id).ok()?.printed_name)
                    .cloned()
            });
        let creature_cost = {
            let card = self.cards.get_mut(card_id)?;
            card.reset_transient_state(creature_def.as_ref());
            card.cast_as_adventure = false;
            card.mana_cost
        };

        let card_name = self.cards.get(card_id)?.name.to_string();

        // Move the card from the stack to exile (public zone; logged by move_card).
        self.sub_action_scratch.spell_targets.retain(|(id, _)| *id != card_id);
        self.move_card(card_id, Zone::Stack, Zone::Exile, owner)?;

        // Grant "you may cast the creature half from exile for its mana cost".
        // The permission is cleaned up when the card leaves exile (CR 715.3e).
        self.persistent_effects.add(
            crate::core::PersistentEffectKind::MayPlayFromExile {
                tracked_card: card_id,
                alternative_cost: creature_cost,
                owner,
            },
            card_id,
            owner,
            crate::core::persistent_effect::CleanupCondition::TrackedCardLeavesZone {
                card: card_id,
                zone: Zone::Exile,
            },
        );

        self.logger.gamelog(&format!(
            "{} goes on an adventure (exiled; {} may cast the creature from exile)",
            card_name,
            self.player_display_name(owner)
        ));

        Ok(())
    }

    /// Resolve a spell's effects, returning them as a Vec instead of executing.
    ///
    /// This resolves placeholder targets and expands all-player effects, but does
    /// NOT execute the effects. The caller can then iterate and selectively intercept
    /// effects that need controller input (e.g., discard choices for network play).
    ///
    /// Returns `None` if all targets are illegal (spell fizzles).
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found.
    pub fn resolve_spell_collect_effects(
        &mut self,
        card_id: CardId,
        chosen_targets: &[CardId],
    ) -> Result<Option<Vec<Effect>>> {
        let (card_owner, effects_len) = {
            let card = self.cards.get(card_id)?;
            (card.owner, card.effects.len())
        };

        log::debug!(target: "resolve_spell", "resolve_spell_collect_effects card_id={}, chosen_targets={:?}, effects_len={}", card_id.as_u32(), chosen_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>(), effects_len);

        let opponent_id = self.players.iter().map(|p| p.id).find(|id| *id != card_owner);

        // MTG Rules 608.2b: If all targets are illegal, the spell doesn't resolve
        // A target is legal if it is on the battlefield, on the stack, OR is a
        // valid player-target sentinel (Lightning Bolt aimed at a player).
        let all_targets_illegal = if !chosen_targets.is_empty() {
            chosen_targets.iter().any(|&target_id| {
                !self.battlefield.contains(target_id)
                    && !self.stack.contains(target_id)
                    && crate::core::player_target_from_sentinel(target_id).is_none()
            })
        } else {
            false
        };

        if all_targets_illegal {
            return Ok(None);
        }

        // Snapshot target characteristics now, while every target is still on
        // the battlefield (this collect phase runs before ANY effect executes).
        // The caller (the interactive game loop's choice-routing resolver)
        // executes the returned effects later, by which point a preceding
        // exile/destroy has stripped continuous buffs — so we must lock any
        // GainLifeDynamic amount to last-known information here (CR 608.2h),
        // mirroring resolve_spell_execute_effects. Keeping both paths identical
        // is required for network determinism.
        let target_snapshots = self.snapshot_target_amounts(chosen_targets);

        let mut result = Vec::new();
        let mut target_index = 0;
        let mut last_resolved_target: Option<CardId> = None;
        for effect_index in 0..effects_len {
            let effect = self.cards.get(card_id)?.effects.get(effect_index).cloned();
            if let Some(effect) = effect {
                let resolved = self.resolve_effect_target(
                    &effect,
                    chosen_targets,
                    &mut target_index,
                    card_owner,
                    opponent_id,
                    &mut last_resolved_target,
                );
                let resolved = Self::resolve_choose_color_source(resolved, card_id);
                let resolved = Self::resolve_self_target(resolved, card_id);
                let resolved = Self::resolve_gain_control_source(resolved, card_id);
                let resolved = Self::resolve_dynamic_gainlife_snapshot(resolved, &target_snapshots);
                let player_ids: smallvec::SmallVec<[PlayerId; 4]> = self.players.iter().map(|p| p.id).collect();
                let expanded = expand_all_players_effect(&resolved, &player_ids);
                result.extend(expanded.into_iter());
            }
        }

        Ok(Some(result))
    }

    /// Attach Equipment or Aura to a target card
    ///
    /// This is called when:
    /// - An Equip activated ability resolves
    /// - An Aura spell resolves (attaching to its target)
    /// - An effect moves an Equipment to attach to a new target
    ///
    /// ## Rules Implementation (CR 301.5, 303.4)
    /// - Equipment can only attach to creatures
    /// - Auras can attach based on their enchant ability
    /// - If already attached, detaches from previous target first
    /// - Updates timestamp on the Equipment/Aura (CR 613.7e)
    ///
    /// # Errors
    ///
    /// Returns an error if the equipment/target is not on battlefield,
    /// the card is not equipment/aura, or target is not a valid creature.
    pub fn attach_equipment(&mut self, equipment_id: CardId, target_id: CardId) -> Result<()> {
        // Validate Equipment is on battlefield
        if !self.battlefield.contains(equipment_id) {
            return Err(MtgError::InvalidAction(
                "Equipment must be on battlefield to attach".to_string(),
            ));
        }

        // Validate target is on battlefield
        if !self.battlefield.contains(target_id) {
            return Err(MtgError::InvalidAction("Target must be on battlefield".to_string()));
        }

        // Get Equipment and target
        let equipment = self.cards.get(equipment_id)?;
        if !equipment.is_equipment() && !equipment.is_aura() {
            return Err(MtgError::InvalidAction(
                "Only Equipment or Auras can be attached".to_string(),
            ));
        }

        let target = self.cards.get(target_id)?;
        if !target.is_creature() {
            return Err(MtgError::InvalidAction(
                "Equipment can only attach to creatures".to_string(),
            ));
        }

        // Check controller ownership (Equipment can only attach to creatures you control)
        let equipment_controller = equipment.controller;
        let target_controller = target.controller;
        if equipment_controller != target_controller {
            return Err(MtgError::InvalidAction(
                "Equipment can only attach to creatures you control".to_string(),
            ));
        }

        // Detach from previous target if needed
        let equipment = self.cards.get_mut(equipment_id)?;
        let equipment_name = equipment.name.to_string();
        if let Some(old_target) = equipment.attached_to {
            // Log detachment
            let old_target_name = self
                .cards
                .get(old_target)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.logger
                .verbose(&format!("{} detaches from {}", equipment_name, old_target_name));
        }

        // Attach to new target
        // Capture old value and log size before mutation
        let old_target = self.cards.get(equipment_id)?.attached_to;
        let prior_log_size = self.logger.log_count();

        let equipment = self.cards.get_mut(equipment_id)?;
        let new_target = Some(target_id);
        equipment.attached_to = new_target;

        // Log the mutation for undo
        self.undo_log.log(
            crate::undo::GameAction::SetAttachedTo {
                equipment_id,
                old_target,
                new_target,
            },
            prior_log_size,
        );

        // Log attachment
        let target_name = self
            .cards
            .get(target_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        self.logger
            .verbose(&format!("{} attaches to {}", equipment_name, target_name));

        Ok(())
    }

    /// Reanimate the Aura's chosen target (currently in a graveyard) onto the
    /// battlefield, then attach the Aura to it.
    ///
    /// This is the inline implementation of the Animate-Dead-style
    /// `T:Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self |
    ///  IsPresent$ Card.StrictlySelf | Execute$ TrigReanimate` chain. It runs from
    /// `resolve_spell_finalize` whenever an Aura resolves with a chosen target that
    /// is in a graveyard rather than on the battlefield, which is the only legal
    /// shape for Animate Dead's `K:Enchant:Creature.inZoneGraveyard` requirement.
    ///
    /// Steps (matching the Java SVar sequence `TrigReanimate → DBAttach`):
    ///   1. Move the target card from its current graveyard to the battlefield
    ///      under the Aura's controller (`GainControl$ True` semantics — the
    ///      reanimating player keeps the creature even if they own neither it nor
    ///      the Aura's owner originally).
    ///   2. Apply the target card's own ETB (etbCounter etc.) **first**, then fire
    ///      its ETB triggers — so Triskelion arrives with three +1/+1 counters and
    ///      any "enters with" replacement effects resolve correctly before the
    ///      Aura attaches.
    ///   3. Attach the Aura to the freshly reanimated creature.
    ///
    /// What is intentionally **not** done here (tracked in mtg-400):
    ///   * The delayed leave-the-battlefield trigger that sacrifices the
    ///     reanimated creature when the Aura goes away.
    ///   * The keyword swap that rewrites the Aura's enchant restriction so it
    ///     reads "creature put onto the battlefield with CARDNAME". In practice
    ///     this only matters for niche corner cases (e.g., the Aura blinks).
    ///
    /// # Errors
    ///
    /// Returns an error if any of the underlying zone moves or the attach fails
    /// (which would leave the game in a recoverable but unexpected state — the
    /// undo log will roll back the partial reanimation).
    fn reanimate_aura_target(&mut self, aura_id: CardId, target_id: CardId, aura_controller: PlayerId) -> Result<()> {
        // Look up the original owner so move_card removes the card from the
        // correct graveyard. (After the move we'll override controller to
        // `aura_controller` to honour `GainControl$ True`.)
        let target_owner = self.cards.get(target_id)?.owner;
        let aura_name = self.cards.get(aura_id)?.name.clone();
        let target_name = self.cards.get(target_id)?.name.clone();

        self.logger
            .gamelog(&format!("{} reanimates {} from graveyard", aura_name, target_name));

        // Step 1: move target from owner's graveyard to battlefield
        self.move_card(target_id, Zone::Graveyard, Zone::Battlefield, target_owner)?;

        // Step 1b: gain control (Animate Dead's `GainControl$ True`). If the Aura's
        // controller differs from the dead creature's owner, switch the controller.
        if aura_controller != target_owner {
            // Mirror what `GainControl` effects do: stash old controller for SBA/undo,
            // overwrite, log. Reuse the existing `take_control_of` helper if present;
            // otherwise mutate `card.controller` directly with an undo entry.
            let prior_log_size = self.logger.log_count();
            let card = self.cards.get_mut(target_id)?;
            let old_controller = card.controller;
            card.controller = aura_controller;
            self.undo_log.log(
                crate::undo::GameAction::ChangeController {
                    card_id: target_id,
                    old_controller,
                    new_controller: aura_controller,
                },
                prior_log_size,
            );
        }

        // Step 1c: record turn entered (summoning sickness clock starts now)
        if let Ok(card) = self.cards.get_mut(target_id) {
            let old_value = card.turn_entered_battlefield;
            let prior_log_size = self.logger.log_count();
            let new_value = Some(self.turn.turn_number);
            card.turn_entered_battlefield = new_value;
            self.undo_log.log(
                crate::undo::GameAction::SetTurnEnteredBattlefield {
                    card_id: target_id,
                    old_value,
                    new_value,
                },
                prior_log_size,
            );
        }

        // Step 2: apply the reanimated creature's own ETB (etbCounter + triggers).
        self.apply_etb_counters(target_id)?;
        self.check_triggers(TriggerEvent::EntersBattlefield, target_id)?;

        // Step 3: attach the Aura. Both cards are on the battlefield now, so
        // `attach_aura`'s zone checks will succeed. This runs after the target's
        // ETB triggers — matching the Java SVar order
        // `TrigReanimate (move) → DBAnimate (no-op for us) → DBAttach`.
        if self.battlefield.contains(aura_id) && self.battlefield.contains(target_id) {
            self.attach_aura(aura_id, target_id)?;
        }

        // Step 4 (mtg-400): the DBDelay leave-sacrifice trigger. Animate Dead's
        // Oracle: "When CARDNAME leaves the battlefield, that creature's
        // controller sacrifices it." Register a delayed trigger that WATCHES the
        // Aura (`aura_id`) leaving the battlefield (to ANY zone — destroyed,
        // bounced, exiled) and SACRIFICES the reanimated creature (`target_id`).
        // Empty `to_zones` matches any destination (CR 603.7: the trigger fires
        // on the Aura's leave event regardless of where it goes). The creature's
        // CardId is captured into the delayed-trigger state (serialized), so it
        // reconstructs identically on snapshot/resume and WASM rewind/replay.
        //
        // If the creature dies first, SBA detaches+graveyards the Aura, which
        // fires this trigger — but `SacrificeOther` no-ops when the creature is
        // no longer on the battlefield, so there is no double-sacrifice.
        //
        // REWIND SAFETY (mtg-400): the `add` MUST be undo-logged with
        // `RegisterDelayedTrigger`, exactly like every other delayed-trigger
        // registration site (the Mana Drain / dies-trigger sites below). Without
        // it, rewind-to-turn-start (snapshot/resume, WASM rewind/replay, undo
        // search) does NOT remove the trigger, so the replayed turn-start state
        // carries an extra `delayed_triggers` entry and the turn-start state hash
        // diverges across rewinds ("undo log is no longer a faithful inverse").
        // A reanimator deck (All Hallow's Eve + Animate Dead, rogerbrand seed 3)
        // hit this as a 100%-deterministic turn-6 rewind desync.
        {
            use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerCondition, DelayedTriggerId};
            use smallvec::smallvec;

            let sac_trigger = DelayedTrigger::new(
                DelayedTriggerId::new(0),
                aura_id,
                aura_id,
                aura_controller,
                DelayedTriggerCondition::ZoneChange {
                    from_zones: smallvec![Zone::Battlefield],
                    to_zones: smallvec![],
                },
                DelayedEffect::SacrificeOther { card: target_id },
            );
            let prior_log_size = self.logger.log_count();
            let trigger_id = self.delayed_triggers.add(sac_trigger);
            self.undo_log.log(
                crate::undo::GameAction::RegisterDelayedTrigger { id: trigger_id },
                prior_log_size,
            );
        }

        Ok(())
    }

    /// Apply counters from `K:etbCounter` keyword as a card enters the battlefield.
    ///
    /// Implements MTG CR 614.1c — "X enters the battlefield with N counters" is a
    /// self-replacement effect on a permanent's own ETB. We model it by placing the
    /// counters immediately after the card moves into the Battlefield zone but
    /// **before** any ETB triggers fire, so triggers like "whenever a creature enters
    /// with +1/+1 counters" observe the counters correctly.
    ///
    /// Triggered for every entry into the battlefield (cast, played as land, returned
    /// from any zone via `ChangeZone`, etc.) — this is what makes a reanimated
    /// Triskelion arrive with its three +1/+1 counters.
    ///
    /// Silently no-ops for cards without the keyword. Logs a warning and skips the
    /// counter for unsupported counter types or non-numeric amounts (e.g. `X`/`Y`,
    /// which would require evaluation context — TODO).
    ///
    /// # Errors
    ///
    /// Returns an error only if `add_counters` itself fails (card disappeared between
    /// the lookup and the mutation, which should not happen during a single ETB
    /// resolution).
    fn apply_etb_counters(&mut self, card_id: CardId) -> Result<()> {
        use crate::core::{CounterType, Keyword, KeywordArgs};

        let (counter_type, amount, card_name) = {
            let Some(card) = self.cards.try_get(card_id) else {
                return Ok(());
            };
            let Some(args) = card.keywords.get_args(Keyword::EtbCounter) else {
                return Ok(());
            };
            let KeywordArgs::EtbCounter {
                counter_type,
                amount,
                condition: _,
            } = args
            else {
                return Ok(());
            };
            let Some(ct) = CounterType::parse(counter_type) else {
                log::warn!(
                    "apply_etb_counters: unknown counter type '{}' on {}",
                    counter_type,
                    card.name
                );
                return Ok(());
            };
            let Ok(amt) = amount.parse::<u8>() else {
                // TODO(mtg-400): symbolic amounts like "X" / "Y" require an
                // evaluation context (caster's choice, X paid, etc.).
                log::warn!(
                    "apply_etb_counters: non-numeric amount '{}' on {} not yet supported",
                    amount,
                    card.name
                );
                return Ok(());
            };
            (ct, amt, card.name.clone())
        };

        if amount == 0 {
            return Ok(());
        }

        self.logger.gamelog(&format!(
            "{} enters the battlefield with {} {} counter{}",
            card_name,
            amount,
            counter_type.display_name(),
            if amount == 1 { "" } else { "s" }
        ));
        self.add_counters(card_id, counter_type, amount)?;
        Ok(())
    }

    /// Attach Aura to a target card
    ///
    /// This is called when an Aura spell resolves and enters the battlefield.
    ///
    /// ## Rules Implementation (CR 303.4)
    /// - Auras can attach to any legal target (including opponent's creatures)
    /// - The target is determined by the "enchant" keyword (e.g., "Enchant creature")
    /// - If already attached, detaches from previous target first
    ///
    /// # Errors
    ///
    /// Returns an error if the aura/target is not on battlefield, or the card is not an aura.
    pub fn attach_aura(&mut self, aura_id: CardId, target_id: CardId) -> Result<()> {
        // Validate Aura is on battlefield
        if !self.battlefield.contains(aura_id) {
            return Err(MtgError::InvalidAction(
                "Aura must be on battlefield to attach".to_string(),
            ));
        }

        // Validate target is on battlefield
        if !self.battlefield.contains(target_id) {
            return Err(MtgError::InvalidAction("Target must be on battlefield".to_string()));
        }

        // Get Aura and target
        let aura = self.cards.get(aura_id)?;
        if !aura.is_aura() {
            return Err(MtgError::InvalidAction(
                "Only Auras can be attached via attach_aura".to_string(),
            ));
        }
        let aura_name = aura.name.to_string();

        // Validate target type based on enchant restriction from KeywordArgs::Enchant
        // Parse the Aura's "Enchant X" keyword to determine valid targets
        let enchant_type = aura.keywords.get_args(crate::core::Keyword::Enchant).and_then(|args| {
            if let crate::core::KeywordArgs::Enchant { card_type } = args {
                Some(card_type.as_str().to_string())
            } else {
                None
            }
        });

        let target = self.cards.get(target_id)?;
        // Strip the `.inZone<X>` qualifier (used by reanimation Auras like Animate Dead
        // — `Enchant:Creature.inZoneGraveyard`). This qualifier filters the **casting**
        // target to a graveyard card; once we've reanimated the creature and are
        // attaching the Aura on the battlefield, only the bare card type matters.
        // Without this, Animate Dead would refuse to attach to its own reanimated
        // target because Triskelion has no "Creature.inZoneGraveyard" subtype.
        let strip_inzone = |s: &str| -> String {
            if let Some(idx) = s.to_ascii_lowercase().find(".inzone") {
                s[..idx].to_string()
            } else {
                s.to_string()
            }
        };
        let normalized = enchant_type.as_deref().map(strip_inzone);
        let target_valid = match normalized.as_deref() {
            Some("Creature") | None => target.is_creature(), // Default: Enchant creature
            Some("Land") => target.is_land(),
            Some("Artifact") => target.is_artifact(),
            Some("Enchantment") => target.is_enchantment(),
            Some("Permanent" | "permanent") => true, // Any permanent
            Some("Player" | "player") => false,      // Player auras handled separately
            Some(other) => {
                // Check if it matches a creature subtype (e.g., "Enchant Goblin")
                target.is_creature() && target.subtypes.iter().any(|st| st.as_str().eq_ignore_ascii_case(other))
            }
        };

        if !target_valid {
            let type_desc = enchant_type.as_deref().unwrap_or("creature");
            return Err(MtgError::InvalidAction(format!(
                "This Aura can only enchant {}s",
                type_desc.to_lowercase()
            )));
        }

        // Detach from previous target if needed (unlikely for newly-resolved Aura)
        let aura = self.cards.get_mut(aura_id)?;
        if let Some(old_target) = aura.attached_to {
            let old_target_name = self
                .cards
                .get(old_target)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.logger
                .verbose(&format!("{} detaches from {}", aura_name, old_target_name));
        }

        // Attach to new target
        // Capture old value and log size before mutation
        let old_target = self.cards.get(aura_id)?.attached_to;
        let prior_log_size = self.logger.log_count();

        let aura = self.cards.get_mut(aura_id)?;
        let new_target = Some(target_id);
        aura.attached_to = new_target;

        // Log the mutation for undo
        self.undo_log.log(
            crate::undo::GameAction::SetAttachedTo {
                equipment_id: aura_id,
                old_target,
                new_target,
            },
            prior_log_size,
        );

        // Log attachment
        let target_name = self
            .cards
            .get(target_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        self.logger.gamelog(&format!("{} enchants {}", aura_name, target_name));

        Ok(())
    }

    /// Detach Equipment or Aura from its target
    ///
    /// This is called when:
    /// - The attached creature leaves the battlefield (state-based action)
    /// - An effect explicitly detaches the Equipment
    /// - The Equipment/Aura leaves the battlefield
    ///
    /// ## Rules Implementation
    /// - Equipment remains on battlefield when detached
    /// - Auras that become unattached typically go to graveyard (handled elsewhere)
    ///
    /// # Errors
    ///
    /// Returns an error if the equipment cannot be found.
    pub fn detach_equipment(&mut self, equipment_id: CardId) -> Result<()> {
        // Get names and attached_to before mutable borrow
        let equipment = self.cards.get(equipment_id)?;
        let equipment_name = equipment.name.to_string();
        let target_id_opt = equipment.attached_to;

        if let Some(target_id) = target_id_opt {
            // Log detachment
            let target_name = self
                .cards
                .get(target_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.logger
                .verbose(&format!("{} detaches from {}", equipment_name, target_name));

            // Now do the actual detachment
            // Capture old value and log size before mutation
            let old_target = target_id_opt; // We already have this from above
            let prior_log_size = self.logger.log_count();

            let equipment = self.cards.get_mut(equipment_id)?;
            let new_target = None;
            equipment.attached_to = new_target;

            // Log the mutation for undo
            self.undo_log.log(
                crate::undo::GameAction::SetAttachedTo {
                    equipment_id,
                    old_target,
                    new_target,
                },
                prior_log_size,
            );
        }

        Ok(())
    }

    /// Get all Equipment attached to a creature
    ///
    /// Used for:
    /// - Calculating creature's effective power/toughness with Equipment buffs
    /// - Determining which Equipment to detach when creature leaves battlefield
    /// - AI evaluation of creature strength
    pub fn get_attached_equipment(&self, creature_id: CardId) -> Vec<CardId> {
        self.battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;
                if card.is_equipment() && card.attached_to == Some(creature_id) {
                    Some(card_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all Auras attached to a permanent
    ///
    /// Used for:
    /// - Calculating creature's effective power/toughness with Aura buffs
    /// - Determining which Auras to move to graveyard when enchanted permanent leaves
    /// - AI evaluation of permanent strength
    pub fn get_attached_auras(&self, permanent_id: CardId) -> Vec<CardId> {
        self.battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;
                if card.is_aura() && card.attached_to == Some(permanent_id) {
                    Some(card_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get a creature's effective power using CR 613 layer system.
    ///
    /// ## Comprehensive Rules 613.4 (Layer 7: Power and Toughness)
    ///
    /// This implements the full layer calculation:
    /// 1. Layer 7a (CHARACTERISTIC): Characteristic-defining abilities (e.g., Tarmogoyf)
    /// 2. Layer 7b (SETPT): Effects that SET P/T (e.g., "becomes 0/1")
    /// 3. Layer 7c (MODIFYPT): Effects and counters that MODIFY P/T (Equipment, +1/+1 counters)
    /// 4. Layer 7d (SWITCH): Effects that switch P/T
    ///
    /// See `continuous_effects::PTBreakdown` for detailed implementation.
    ///
    /// ## Current Implementation Status
    ///
    /// - ✅ Layer 7a: Stubbed (no characteristic-defining abilities yet)
    /// - ✅ Layer 7b: Stubbed (no set P/T effects yet)
    /// - ✅ Layer 7c: Equipment bonuses + counter bonuses
    /// - ✅ Layer 7d: Stubbed (no switch effects yet)
    ///
    /// ## Returns
    ///
    /// Final power after applying all layers, or error if creature not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the creature cannot be found.
    pub fn get_effective_power(&self, creature_id: CardId) -> Result<i32> {
        let breakdown = self.get_pt_breakdown(creature_id)?;
        Ok(breakdown.power())
    }

    /// Get a creature's effective toughness using CR 613 layer system.
    ///
    /// See `get_effective_power()` for full documentation of the layer system.
    ///
    /// ## Returns
    ///
    /// Final toughness after applying all layers, or error if creature not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the creature cannot be found.
    pub fn get_effective_toughness(&self, creature_id: CardId) -> Result<i32> {
        let breakdown = self.get_pt_breakdown(creature_id)?;
        Ok(breakdown.toughness())
    }

    // TODO: Implement get_valid_targets function that filters game entities to find valid targets
    // based on effect type (damage, destroy, tap, etc.), targeting restrictions (hexproof,
    // shroud, protection), controller ownership, and zone requirements.

    /// Calculate effective mana cost after applying cost reduction effects like Affinity.
    ///
    /// Affinity for X reduces generic mana cost by 1 for each permanent of type X you control.
    /// Example: "Affinity for Allies" on a 2G spell with 3 Allies in play = G (0 generic + G)
    ///
    /// # Parameters
    /// - `card_id`: The card being cast
    /// - `player_id`: The player casting the spell
    ///
    /// # Returns
    /// The effective mana cost after applying all cost reductions, or the original cost on error
    pub fn calculate_effective_cost(&self, card_id: CardId, player_id: PlayerId) -> crate::core::ManaCost {
        use crate::core::{Keyword, KeywordArgs, StaticAbility};

        let card = match self.cards.get(card_id) {
            Ok(c) => c,
            Err(_) => return crate::core::ManaCost::new(),
        };

        let mut effective_cost = card.mana_cost;

        // Check for Affinity keyword
        // Affinity for X: This spell costs {1} less for each X you control
        if let Some(KeywordArgs::Affinity { card_type }) = card.keywords.get_args(Keyword::Affinity) {
            // Count permanents of the specified type controlled by the player
            let count = self
                .battlefield
                .cards
                .iter()
                .filter(|&&bf_card_id| {
                    self.cards
                        .try_get(bf_card_id)
                        .is_some_and(|c| c.controller == player_id && c.subtypes.contains(card_type))
                })
                .count() as u8;

            // Reduce generic cost (minimum 0)
            effective_cost.generic = effective_cost.generic.saturating_sub(count);

            if count > 0 {
                log::debug!(
                    "Affinity for {:?}: {} permanents controlled, reducing generic cost by {} (was {}, now {})",
                    card_type,
                    count,
                    count,
                    card.mana_cost.generic,
                    effective_cost.generic
                );
            }
        }

        // Check for ReduceCost / RaiseCost static abilities from permanents.
        //
        // Polarity rules (CR 601.2f):
        // - ReduceCost is a "discount" effect — by convention these abilities
        //   only help their own controller (e.g. Affinity-style cards,
        //   Gran-Gran). We filter to source_card.controller == player_id.
        // - RaiseCost is a "hose" effect — the canonical examples (Gloom,
        //   Karma, Chains of Mephistopheles) hose all players because they
        //   raise the cost of any spell that matches the filter, regardless
        //   of whose battlefield the static ability is on. We do NOT filter
        //   by controller here.
        for &bf_card_id in &self.battlefield.cards {
            let Some(source_card) = self.cards.try_get(bf_card_id) else {
                continue;
            };

            for static_ability in &source_card.static_abilities {
                if let StaticAbility::ReduceCost {
                    valid_card,
                    amount,
                    condition,
                    description,
                } = static_ability
                {
                    // ReduceCost only applies to the controlling player's own spells.
                    if source_card.controller != player_id {
                        continue;
                    }

                    if !spell_matches_cost_filter(card, valid_card) {
                        continue;
                    }

                    // Check if the condition is met (if any)
                    let condition_met = if let Some(cond) = condition {
                        // Count cards matching is_present filter in the specified zone
                        self.count_cards_matching_filter(player_id, &cond.is_present, cond.present_zone)
                            >= cond.min_count as usize
                    } else {
                        true // No condition means always active
                    };

                    if condition_met {
                        let old_generic = effective_cost.generic;
                        effective_cost.generic = effective_cost.generic.saturating_sub(*amount);

                        if old_generic != effective_cost.generic {
                            log::debug!(
                                "ReduceCost from {}: {} (reducing generic by {}, was {}, now {})",
                                source_card.name,
                                description,
                                amount,
                                old_generic,
                                effective_cost.generic
                            );
                        }
                    }
                }

                // Also check for RaiseCost (mana-based cost increases).
                // RaiseCost applies regardless of source controller — see polarity
                // note above (Gloom hoses both players).
                if let StaticAbility::RaiseCost {
                    valid_card,
                    raised_cost,
                    description,
                } = static_ability
                {
                    use crate::core::RaisedCost;

                    if !spell_matches_cost_filter(card, valid_card) {
                        continue;
                    }

                    // Handle mana-based cost increase
                    if let RaisedCost::Mana(amount) = raised_cost {
                        let old_generic = effective_cost.generic;
                        effective_cost.generic = effective_cost.generic.saturating_add(*amount);

                        if old_generic != effective_cost.generic {
                            log::debug!(
                                "RaiseCost from {}: {} (increasing generic by {}, was {}, now {})",
                                source_card.name,
                                description,
                                amount,
                                old_generic,
                                effective_cost.generic
                            );
                        }
                    }
                    // Note: Sacrifice-based RaiseCost is handled separately during spell casting
                    // as it requires prompting for sacrifice choices, not just mana adjustment
                }
            }
        }

        // Resolve X in mana cost: each X symbol adds x_paid generic mana
        // x_paid is set by the priority loop before this is called
        if effective_cost.has_x() {
            effective_cost = effective_cost.with_x_value(card.x_paid);
        }

        effective_cost
    }

    /// Count cards matching a filter string in a specified zone
    ///
    /// Used for checking ReduceCost conditions like "IsPresent$ Lesson.YouOwn | PresentZone$ Graveyard"
    /// and conditional static abilities like Sedge Troll's `IsPresent$ Swamp.YouCtrl`.
    pub(crate) fn count_cards_matching_filter(
        &self,
        player_id: PlayerId,
        filter: &str,
        zone: crate::zones::Zone,
    ) -> usize {
        use crate::zones::Zone;

        // Parse filter: "Lesson.YouOwn" -> type="Lesson", quals=["YouOwn"].
        // Qualifiers after the first '.' are '+'-joined, e.g.
        // "Permanent.White+YouCtrl" -> type="Permanent", quals=["White","YouCtrl"].
        let mut sections = filter.splitn(2, '.');
        let type_filter = sections.next().unwrap_or("");
        let quals: Vec<&str> = sections.next().map(|q| q.split('+').collect()).unwrap_or_default();
        // Default ownership/control qualifier is "YouOwn" if none of the
        // ownership-style qualifiers is present.
        let ownership = quals
            .iter()
            .copied()
            .find(|q| matches!(*q, "YouOwn" | "OppOwn" | "YouCtrl" | "OppCtrl"))
            .unwrap_or("YouOwn");

        // Get the appropriate zone's cards
        let zone_cards: &[CardId] = match zone {
            Zone::Graveyard => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    zones.graveyard.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Hand => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    zones.hand.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Battlefield => self.battlefield.cards.as_slice(),
            Zone::Exile => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    zones.exile.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Library => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    zones.library.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Stack => {
                // Stack items are StackEntry, not directly cards
                return 0;
            }
            Zone::Command => {
                // Command zone (for Commander format) not typically checked
                return 0;
            }
        };

        // mtg-728 class-A / mtg-725 R1: on a SHADOW game the opponent's
        // Hand/Library cards are RESERVED (instance-less) ids. `try_get` returns
        // None for them, so the server (real instances) counts them while the
        // shadow drops them — a branch-on-absence count desync. Handle the
        // reserved id SYMMETRICALLY (sig-2c/2d template): a reserved card in
        // `player_id`'s hidden zone is owned + controlled by `player_id`, so it
        // matches a WILDCARD type filter ("" / "Card" / "Permanent") whose
        // ownership qualifier is zone-owner-relative (YouOwn / YouCtrl). Typed or
        // colored filters and opponent-relative qualifiers (OppOwn / OppCtrl) are
        // unevaluable without the instance, so a reserved id does NOT match them
        // (no over-count; a hidden zone cannot be conditioned on by type).
        let reserved_owner_matches = self.is_shadow_game
            && matches!(type_filter, "" | "Card" | "Permanent")
            && matches!(ownership, "YouOwn" | "YouCtrl")
            && !quals
                .iter()
                .any(|q| matches!(*q, "White" | "Blue" | "Black" | "Red" | "Green"));

        zone_cards
            .iter()
            .filter(|&&cid| {
                let Some(c) = self.cards.try_get(cid) else {
                    return reserved_owner_matches;
                };

                // Check ownership filter
                let ownership_ok = match ownership {
                    "YouOwn" => c.owner == player_id,
                    "OppOwn" => c.owner != player_id,
                    "YouCtrl" => c.controller == player_id,
                    "OppCtrl" => c.controller != player_id,
                    _ => true,
                };

                if !ownership_ok {
                    return false;
                }

                // Check the base type filter. "Card"/"Permanent" are wildcards;
                // card types (Land/Artifact/Creature/Enchantment/...) match
                // c.types; otherwise treat as a creature subtype.
                let type_ok = match type_filter {
                    "" | "Card" | "Permanent" => true,
                    "Land" => c.is_land(),
                    "Artifact" => c.is_artifact(),
                    "Creature" => c.is_creature(),
                    "Enchantment" => c.is_enchantment(),
                    "Planeswalker" => c.types.contains(&crate::core::CardType::Planeswalker),
                    other => c.subtypes.contains(&crate::core::Subtype::new(other)),
                };
                if !type_ok {
                    return false;
                }

                // Check any color qualifiers (e.g. "White" in
                // "Permanent.White+YouCtrl"). All present color quals must hold.
                quals.iter().all(|q| match *q {
                    "White" => c.colors.contains(&crate::core::Color::White),
                    "Blue" => c.colors.contains(&crate::core::Color::Blue),
                    "Black" => c.colors.contains(&crate::core::Color::Black),
                    "Red" => c.colors.contains(&crate::core::Color::Red),
                    "Green" => c.colors.contains(&crate::core::Color::Green),
                    // Ownership/control quals already handled above; everything
                    // else is ignored (best-effort) rather than failing the match.
                    _ => true,
                })
            })
            .count()
    }

    /// Pay sacrifice costs for a card being cast
    ///
    /// Checks if the card has any RaiseCost::Sacrifice static abilities and
    /// sacrifices the required permanents. For AI players, this auto-selects
    /// the permanents to sacrifice.
    fn pay_sacrifice_costs(&mut self, card_id: CardId, player_id: PlayerId) -> Result<()> {
        use crate::core::{RaisedCost, RaisedCostAmount, StaticAbility};

        // Get the card's static abilities (need to clone to avoid borrow issues)
        let static_abilities: Vec<StaticAbility> = self
            .cards
            .try_get(card_id)
            .map(|c| c.static_abilities.clone())
            .unwrap_or_default();

        // Get the card's SVars for X calculation
        let svars: std::collections::HashMap<String, String> =
            self.cards.try_get(card_id).map(|c| c.svars.clone()).unwrap_or_default();

        for static_ability in &static_abilities {
            if let StaticAbility::RaiseCost {
                raised_cost: RaisedCost::Sacrifice { amount, valid_type },
                description,
                ..
            } = static_ability
            {
                // Calculate required sacrifice amount
                let required_amount = match amount {
                    RaisedCostAmount::Fixed(n) => *n as usize,
                    RaisedCostAmount::Variable(svar_name) => {
                        self.evaluate_sacrifice_svar_internal(svar_name, &svars, player_id, valid_type)
                    }
                };

                if required_amount == 0 {
                    continue;
                }

                // Find permanents to sacrifice
                let permanents_to_sacrifice: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .filter(|&&pid| {
                        self.cards.try_get(pid).is_some_and(|c| {
                            c.controller == player_id && Self::card_matches_type_filter_static(c, valid_type)
                        })
                    })
                    .copied()
                    .take(required_amount)
                    .collect();

                if permanents_to_sacrifice.len() < required_amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Cannot pay sacrifice cost: need {} {} but only have {}",
                        required_amount,
                        valid_type,
                        permanents_to_sacrifice.len()
                    )));
                }

                // Log the sacrifice
                if !description.is_empty() {
                    log::debug!(
                        "Paying sacrifice cost: {} ({} {})",
                        description,
                        required_amount,
                        valid_type
                    );
                }

                // Sacrifice the permanents
                for sacrifice_id in permanents_to_sacrifice {
                    if let Some(card) = self.cards.try_get(sacrifice_id) {
                        let card_name = card.name.clone();
                        self.logger.gamelog(&format!(
                            "  sacrifices {} ({}) as additional cost",
                            card_name, sacrifice_id
                        ));
                    }
                    self.move_card(
                        sacrifice_id,
                        Zone::Battlefield,
                        self.death_destination_for_card(sacrifice_id),
                        player_id,
                    )?;
                }
            }
        }

        Ok(())
    }

    /// Evaluate an SVar for sacrifice cost amount (internal version for GameState)
    fn evaluate_sacrifice_svar_internal(
        &self,
        svar_name: &str,
        svars: &std::collections::HashMap<String, String>,
        player_id: PlayerId,
        _valid_type: &str,
    ) -> usize {
        let Some(svar_value) = svars.get(svar_name) else {
            log::warn!("RaiseCost SVar '{}' not found", svar_name);
            return 0;
        };

        // Parse "Count$Valid Land.YouCtrl/HalfUp" or similar
        if let Some(count_expr) = svar_value.strip_prefix("Count$Valid ") {
            let parts: Vec<&str> = count_expr.split('/').collect();
            let type_filter = parts.first().copied().unwrap_or("");
            let modifier = parts.get(1).copied().unwrap_or("");

            let filter_type = type_filter.split('.').next().unwrap_or(type_filter);
            let count = self.count_permanents_by_type_internal(player_id, filter_type);

            match modifier {
                "HalfUp" => count.div_ceil(2),
                "Half" => count / 2,
                _ => count,
            }
        } else {
            svar_value.parse().unwrap_or(0)
        }
    }

    /// Count permanents of a specific type controlled by a player (internal version)
    fn count_permanents_by_type_internal(&self, player_id: PlayerId, type_filter: &str) -> usize {
        self.battlefield
            .cards
            .iter()
            .filter(|&&card_id| {
                self.cards
                    .try_get(card_id)
                    .is_some_and(|c| c.controller == player_id && Self::card_matches_type_filter_static(c, type_filter))
            })
            .count()
    }

    /// Check if a card matches a type filter string (static method)
    fn card_matches_type_filter_static(card: &crate::core::Card, type_filter: &str) -> bool {
        match type_filter {
            "Land" => card.is_land(),
            "Creature" => card.is_creature(),
            "Artifact" => card.is_artifact(),
            "Enchantment" => card.is_enchantment(),
            "Permanent" => true,
            _ => {
                let subtype = crate::core::Subtype::new(type_filter);
                card.subtypes.contains(&subtype)
            }
        }
    }

    /// Cast a spell following the full 8-step process (MTG Rules 601.2)
    ///
    /// This method implements the complete spell casting sequence:
    /// 1. Propose the spell (move to stack)
    /// 2. Make choices (modes, X values) - TODO
    /// 3. Choose targets
    /// 4. Divide effects - TODO
    /// 5. Determine total cost
    /// 6. Activate mana abilities (tap sources for mana)
    /// 7. Pay costs
    /// 8. Spell becomes cast (trigger abilities) - TODO
    ///
    /// ## Parameters
    /// - `player_id`: The player casting the spell
    /// - `card_id`: The spell card to cast
    /// - `choose_targets_fn`: Callback to choose targets (step 3)
    /// - `mana_engine`: Pre-computed ManaEngine for mana payment (step 6)
    ///
    /// ## Java Forge Equivalent
    /// This matches `ComputerUtil.handlePlayingSpellAbility()` which:
    /// 1. Moves spell to stack (line 99)
    /// 2. Handles targeting
    /// 3. Pays costs with `CostPayment.payComputerCosts()` (line 125)
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not in hand, cannot move to stack, or mana payment fails.
    pub fn cast_spell_8_step<TargetFn>(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        choose_targets_fn: TargetFn,
        mana_engine: &crate::game::mana_engine::ManaEngine,
    ) -> Result<()>
    where
        TargetFn: FnOnce(&GameState, CardId) -> smallvec::SmallVec<[CardId; 2]>,
    {
        self.cast_spell_8_step_from(player_id, card_id, choose_targets_fn, mana_engine, Zone::Hand, None)
    }

    /// Generalized 8-step spell casting process that works from any source zone.
    ///
    /// - `source_zone`: Where the card is being cast from (Hand, Exile, Command, etc.)
    /// - `override_cost`: If Some, use this cost instead of the card's printed mana cost
    ///   (e.g., alternative cost for Airbend, or base cost + commander tax for commanders)
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not in the expected source zone, the source zone
    /// is not a valid casting zone, sacrifice costs cannot be paid, or mana payment fails.
    pub fn cast_spell_8_step_from<TargetFn>(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        choose_targets_fn: TargetFn,
        mana_engine: &crate::game::mana_engine::ManaEngine,
        source_zone: Zone,
        override_cost: Option<crate::core::ManaCost>,
    ) -> Result<()>
    where
        TargetFn: FnOnce(&GameState, CardId) -> smallvec::SmallVec<[CardId; 2]>,
    {
        // Verify card is in the expected source zone
        match source_zone {
            Zone::Hand => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    if !zones.hand.contains(card_id) {
                        return Err(MtgError::InvalidAction("Card not in hand".to_string()));
                    }
                }
            }
            Zone::Exile => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    if !zones.exile.contains(card_id) {
                        // Also check by owner since exile zone belongs to the card's owner
                        let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(player_id);
                        if owner != player_id {
                            if let Some(owner_zones) = self.get_player_zones(owner) {
                                if !owner_zones.exile.contains(card_id) {
                                    return Err(MtgError::InvalidAction("Card not in exile".to_string()));
                                }
                            }
                        } else {
                            return Err(MtgError::InvalidAction("Card not in exile".to_string()));
                        }
                    }
                }
            }
            Zone::Command => {
                if let Some(zones) = self.get_player_zones(player_id) {
                    if !zones.command.contains(card_id) {
                        return Err(MtgError::InvalidAction("Card not in command zone".to_string()));
                    }
                }
            }
            Zone::Library | Zone::Battlefield | Zone::Graveyard | Zone::Stack => {
                return Err(MtgError::InvalidAction(format!(
                    "Cannot cast spell from {:?}",
                    source_zone
                )));
            }
        }

        // Step 1: Propose the spell - move card to stack (move_card auto-reveals + logs MoveCard)
        // This happens BEFORE paying costs (unlike our old implementation)
        let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(player_id);
        self.move_card(card_id, source_zone, Zone::Stack, owner)?;

        // Step 2: Make choices (modes, X values)
        // Modal choices and X value selection are handled in the priority loop
        // (priority.rs) BEFORE this function is called. The card's x_paid field
        // is set there, and calculate_effective_cost uses it below.

        // Step 3: Choose targets
        let chosen_targets = choose_targets_fn(self, card_id);
        let num_targets = chosen_targets.len();
        // TODO: Store targets on the spell for resolution
        // For now, we'll use them to update effects immediately (simplified)

        // Step 4: Divide effects
        // Damage division among the chosen targets (Fireball) is applied at
        // resolution in resolve_effect_target / resolve_x_paid_effect.

        // Step 5: Determine total cost (after applying Affinity and other reductions)
        // If an override cost is provided (e.g., alternative cost or commander cost),
        // use that instead of calculating from the card's printed cost.
        let mut mana_cost = if let Some(cost) = override_cost {
            cost
        } else {
            self.calculate_effective_cost(card_id, player_id)
        };

        // Step 5a: Relative per-target cost (Fireball, CR 601.2f): "{1} more to
        // cast for each target beyond the first". Applied AFTER target selection
        // because the count is only known now. num_targets is public state, so
        // the adjustment is network-deterministic. Costs nothing extra for 0 or
        // 1 target.
        let relative_target_cost = self
            .cards
            .get(card_id)
            .map(|c| c.definition.cache.spell_relative_target_cost)
            .unwrap_or(false);
        if relative_target_cost && num_targets > 1 {
            let extra = (num_targets - 1).min(u8::MAX as usize) as u8;
            mana_cost.generic = mana_cost.generic.saturating_add(extra);
            log::debug!(
                "Relative per-target cost: {} target(s) -> +{} generic (now {})",
                num_targets,
                extra,
                mana_cost.generic
            );
        }

        // Step 5b: Pay additional costs (sacrifice costs from RaiseCost)
        // This must happen BEFORE mana payment so sacrificed lands aren't used for mana
        if let Err(e) = self.pay_sacrifice_costs(card_id, player_id) {
            // Cannot pay sacrifice cost - unwind the spell cast
            self.move_card(card_id, Zone::Stack, source_zone, owner)?;
            return Err(e);
        }

        // Step 6: Activate mana abilities
        // This is where mana gets tapped - AFTER the spell is on the stack
        //
        // IMPORTANT: Check if the player already has floating mana in their pool
        // (e.g., from Dark Ritual). We should use that first, then tap sources
        // only for the remaining cost.
        use crate::core::ManaCost;
        use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

        // Get total available mana (regular pool + combat mana from Firebending)
        // Combat mana lasts until end of combat and can be used for spells
        let current_pool = self.get_player(player_id)?.total_available_mana();

        // Calculate the remaining cost after using pool mana
        // First satisfy colored requirements from pool, then generic
        let remaining_white = mana_cost.white.saturating_sub(current_pool.white);
        let remaining_blue = mana_cost.blue.saturating_sub(current_pool.blue);
        let remaining_black = mana_cost.black.saturating_sub(current_pool.black);
        let remaining_red = mana_cost.red.saturating_sub(current_pool.red);
        let remaining_green = mana_cost.green.saturating_sub(current_pool.green);
        let remaining_colorless = mana_cost.colorless.saturating_sub(current_pool.colorless);

        // Calculate pool mana used for colored requirements
        let used_white = mana_cost.white.min(current_pool.white);
        let used_blue = mana_cost.blue.min(current_pool.blue);
        let used_black = mana_cost.black.min(current_pool.black);
        let used_red = mana_cost.red.min(current_pool.red);
        let used_green = mana_cost.green.min(current_pool.green);
        let used_colorless = mana_cost.colorless.min(current_pool.colorless);

        // Pool mana remaining after colored requirements can be used for generic
        let pool_for_generic = (current_pool.white.saturating_sub(used_white))
            + (current_pool.blue.saturating_sub(used_blue))
            + (current_pool.black.saturating_sub(used_black))
            + (current_pool.red.saturating_sub(used_red))
            + (current_pool.green.saturating_sub(used_green))
            + (current_pool.colorless.saturating_sub(used_colorless));

        let remaining_generic = mana_cost.generic.saturating_sub(pool_for_generic);

        // Create the remaining cost that must be paid by tapping sources
        let remaining_cost = ManaCost {
            generic: remaining_generic,
            white: remaining_white,
            blue: remaining_blue,
            black: remaining_black,
            red: remaining_red,
            green: remaining_green,
            colorless: remaining_colorless,
            x_count: 0,
        };

        let mana_sources = mana_engine.all_sources();
        let resolver = GreedyManaResolver::new();
        let mut sources_to_tap = Vec::new();

        // Only compute tap order for the remaining cost (after pool mana is used)
        // If remaining cost is zero, we don't need to tap any sources
        if remaining_cost.cmc() > 0 && !resolver.compute_tap_order(&remaining_cost, mana_sources, &mut sources_to_tap) {
            // Cannot pay the cost - unwind the spell cast
            self.move_card(card_id, Zone::Stack, source_zone, owner)?;
            return Err(MtgError::InvalidAction(format!(
                "Failed to pay mana cost {:?}: Insufficient mana",
                mana_cost
            )));
        }

        // Track which sources we've successfully tapped for unwinding if needed
        let mut tapped_sources = smallvec::SmallVec::<[CardId; 4]>::new();

        // Track remaining cost as hint for each land tap
        // This ensures dual lands produce the right color based on what's still needed
        let mut remaining_hint = remaining_cost;

        for &source_id in &sources_to_tap {
            if let Err(e) = self.tap_for_mana_and_update_hint(player_id, source_id, &mut remaining_hint) {
                // Tapping failed - unwind the spell cast
                // Move card back to source zone
                self.move_card(card_id, Zone::Stack, source_zone, owner)?;

                // Untap all sources that were successfully tapped so far
                for &tapped_id in &tapped_sources {
                    // Use helper that handles untap + undo log + mana version
                    let _ = self.untap_permanent(tapped_id);
                }

                // Clear the mana pool (remove any mana that was added)
                let player = self.get_player_mut(player_id)?;
                player.mana_pool.clear();

                return Err(MtgError::InvalidAction(format!("Failed to tap mana source: {e}")));
            }
            tapped_sources.push(source_id);
        }

        // Step 7: Pay costs (from both regular and combat mana pools).
        // Snapshot combat mana for undo when it will be spent (mtg-ba6uq #7).
        if self
            .get_player(player_id)
            .map(|p| p.combat_mana_pool.is_some())
            .unwrap_or(false)
        {
            self.log_combat_mana_pool(player_id);
        }
        // Snapshot the regular mana pool for undo before the payment (mtg-733).
        self.log_mana_pool(player_id);
        let player = self.get_player_mut(player_id)?;
        if let Err(e) = player.pay_from_total_mana(&mana_cost) {
            // If we can't pay, we need to unwind:
            // 1. Move card back to source zone from stack
            // 2. Untap all mana sources that were tapped
            // 3. Clear the mana pool

            // Move card back to source zone
            self.move_card(card_id, Zone::Stack, source_zone, owner)?;

            // Untap all sources that were tapped
            for &source_id in &tapped_sources {
                // Use helper that handles untap + undo log + mana version
                let _ = self.untap_permanent(source_id);
            }

            // Clear the mana pool (remove any mana that was added)
            let player = self.get_player_mut(player_id)?;
            player.mana_pool.clear();

            return Err(MtgError::InvalidAction(format!("Failed to pay mana cost: {e}")));
        }

        // Step 8: Spell becomes cast
        // Trigger "whenever you cast a spell" abilities (like Boar-q-pine, Prowess)
        // MTG Rules 601.2i: The spell becomes cast once all costs are paid
        self.check_spellcast_triggers(card_id, player_id)?;

        Ok(())
    }

    /// Resolve an effect's placeholder targets and player IDs
    ///
    /// This helper function resolves placeholder values (target ID 0, player ID 0) in effects
    /// without requiring a clone of the entire Vec<Effect>. It returns a resolved copy of the
    /// effect with targets filled in from the provided context.
    ///
    /// ## Parameters
    /// - `effect`: The effect to resolve (borrowed)
    /// - `chosen_targets`: Slice of targets chosen during spell casting
    /// - `target_index`: Mutable index tracking which target to consume next
    /// - `card_owner`: The controller of the spell (for "you" player references)
    /// - `opponent_id`: Pre-computed opponent ID for untargeted damage effects
    /// - `last_resolved_target`: Tracks the most recently resolved target for SubAbility chains
    ///   with `Defined$ Targeted` (reuse_previous sentinel)
    ///
    /// Note: Wildcard match is intentional - effects without placeholder targets
    /// are returned unchanged. New Effect variants should be reviewed for target
    /// resolution needs.
    #[inline]
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_effect_target(
        &self,
        effect: &Effect,
        chosen_targets: &[CardId],
        target_index: &mut usize,
        card_owner: PlayerId,
        opponent_id: Option<PlayerId>,
        last_resolved_target: &mut Option<CardId>,
    ) -> Effect {
        match effect {
            // DealDamage with a player placeholder (`Defined$ You`) → the
            // controller of the resolving spell (card_owner here). This is the
            // self-damage rider on cards like Psionic Blast
            // (`SVar:DBDealDamage:DB$ DealDamage | Defined$ You | NumDmg$ 2`).
            //
            // The effect_converter encodes `Defined$ You` as
            // `TargetRef::Player(PlayerId::new(0))`, and `PLACEHOLDER_ID == 0`,
            // so this is `is_placeholder()`. Without this arm the unresolved
            // placeholder `PlayerId(0)` fell through to execute_effect and dealt
            // the damage to the *literal* player 0 (P1) — correct only when the
            // caster happened to be P1, but wrong on a cross-player cast (P2
            // casting Psionic Blast at P1 dealt the 2 self-damage to P1 instead
            // of the caster P2). Mirrors the PreventDamage `Defined$ You` arm
            // below and the trigger-path arm in resolve_effect_placeholder.
            // (mtg-533: Psionic Blast cross-player self-damage bug.)
            Effect::DealDamage {
                target: TargetRef::Player(player_id),
                amount,
            } if player_id.is_placeholder() => Effect::DealDamage {
                target: TargetRef::Player(card_owner),
                amount: *amount,
            },

            // Target resolution for permanent-targeting effects
            Effect::DealDamage {
                target: TargetRef::None,
                amount,
            } => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    // Decode player sentinel CardIds back into TargetRef::Player
                    // so Lightning Bolt-style "any target" spells aimed at a
                    // player route through the player-damage path. See
                    // `player_as_target_sentinel` (mtg-bolt-player-tgt).
                    Effect::DealDamage {
                        target: crate::core::target_ref_from_chosen_target(target),
                        amount: *amount,
                    }
                } else if let Some(opp) = opponent_id {
                    // Default to opponent for untargeted damage
                    Effect::DealDamage {
                        target: TargetRef::Player(opp),
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }

            // DealDamageDynamic: resolve the chosen target AND evaluate the
            // CountExpression (e.g. Combustion Technique's
            // `Count$ValidGraveyard Lesson.YouOwn/Plus.2`) against the caster's
            // controller right now, then produce a concrete DealDamage so
            // execute_effect can run without needing the controller context.
            Effect::DealDamageDynamic {
                target: TargetRef::None,
                count,
            } => {
                let amount = self.evaluate_count_expression(count, card_owner).unwrap_or(0).max(0);
                if *target_index < chosen_targets.len() {
                    let raw = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(raw);
                    Effect::DealDamage {
                        target: crate::core::target_ref_from_chosen_target(raw),
                        amount,
                    }
                } else if let Some(opp) = opponent_id {
                    Effect::DealDamage {
                        target: TargetRef::Player(opp),
                        amount,
                    }
                } else {
                    effect.clone()
                }
            }
            // DealDamageDynamic with a resolved target: just evaluate the count.
            Effect::DealDamageDynamic { target, count } => {
                let amount = self.evaluate_count_expression(count, card_owner).unwrap_or(0).max(0);
                Effect::DealDamage {
                    target: target.clone(),
                    amount,
                }
            }

            // DivideEvenly$ RoundedDown (Fireball, CR 601.2d): resolve_x_paid_effect
            // already produced a target-less DealDamageDivided whose `amount_each`
            // holds the UNDIVIDED total X. Consume ALL remaining chosen targets and
            // rewrite amount_each = floor(total / N) (remainder lost). Mirrors the
            // EachDamage consume-all pattern below. Empty-target case fizzles.
            Effect::DealDamageDivided {
                targets: existing,
                amount_each: total,
            } if existing.is_empty() => {
                let remaining = &chosen_targets[*target_index..];
                if remaining.is_empty() {
                    // No targets chosen (TargetMin$ 0) — effect fizzles.
                    return effect.clone();
                }
                let resolved_targets: smallvec::SmallVec<[TargetRef; 4]> = remaining
                    .iter()
                    .map(|&t| crate::core::target_ref_from_chosen_target(t))
                    .collect();
                *last_resolved_target = remaining.last().copied();
                *target_index = chosen_targets.len();
                let n = resolved_targets.len() as i32;
                Effect::DealDamageDivided {
                    targets: resolved_targets,
                    amount_each: *total / n,
                }
            }

            // DealDamageXPaid with no target: resolve target like DealDamage
            Effect::DealDamageXPaid {
                target: TargetRef::None,
                divide: crate::core::DamageDivision::None,
            } => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    Effect::DealDamageXPaid {
                        target: crate::core::target_ref_from_chosen_target(target),
                        divide: crate::core::DamageDivision::None,
                    }
                } else if let Some(opp) = opponent_id {
                    Effect::DealDamageXPaid {
                        target: TargetRef::Player(opp),
                        divide: crate::core::DamageDivision::None,
                    }
                } else {
                    effect.clone()
                }
            }

            // PreventDamage with no target: resolve from chosen_targets
            Effect::PreventDamage {
                target: TargetRef::None,
                amount,
            } => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    Effect::PreventDamage {
                        target: TargetRef::Permanent(target),
                        amount: *amount,
                    }
                } else {
                    // No target chosen - default to self-prevention via controller
                    Effect::PreventDamage {
                        target: TargetRef::Player(card_owner),
                        amount: *amount,
                    }
                }
            }

            // PreventDamage with Defined$ Self: resolve placeholder to source card
            Effect::PreventDamage {
                target: TargetRef::Permanent(card_id),
                amount,
            } if card_id.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    Effect::PreventDamage {
                        target: TargetRef::Permanent(target),
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }

            // PreventDamage with Defined$ You: resolve placeholder to controller
            Effect::PreventDamage {
                target: TargetRef::Player(player_id),
                amount,
            } if player_id.is_placeholder() => Effect::PreventDamage {
                target: TargetRef::Player(card_owner),
                amount: *amount,
            },

            // EachDamage: multiple creatures deal damage to one target
            // Empty damagers vector means "use parent targets" - all chosen_targets except last
            // Placeholder receiver means "use last chosen_target"
            Effect::EachDamage {
                damagers,
                receiver,
                use_card_power,
                fixed_damage,
            } if damagers.is_empty() && receiver.is_placeholder() => {
                if chosen_targets.len() >= 2 {
                    // Damagers = all targets except the last one
                    // Receiver = the last target
                    let resolved_damagers: smallvec::SmallVec<[CardId; 4]> =
                        chosen_targets[..chosen_targets.len() - 1].iter().copied().collect();
                    let resolved_receiver = chosen_targets[chosen_targets.len() - 1];

                    // Consume all targets
                    *target_index = chosen_targets.len();
                    *last_resolved_target = Some(resolved_receiver);

                    Effect::EachDamage {
                        damagers: resolved_damagers,
                        receiver: resolved_receiver,
                        use_card_power: *use_card_power,
                        fixed_damage: *fixed_damage,
                    }
                } else if chosen_targets.len() == 1 {
                    // Only one target = no damagers, just the receiver
                    // This happens when TargetMin$ 0 and user selected 0 damagers
                    *target_index = 1;
                    *last_resolved_target = Some(chosen_targets[0]);

                    Effect::EachDamage {
                        damagers: smallvec::SmallVec::new(),
                        receiver: chosen_targets[0],
                        use_card_power: *use_card_power,
                        fixed_damage: *fixed_damage,
                    }
                } else {
                    // No targets at all - effect fizzles
                    effect.clone()
                }
            }

            Effect::DestroyPermanent {
                target,
                restriction,
                no_regenerate,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::DestroyPermanent {
                        target: resolved_target,
                        restriction: restriction.clone(),
                        no_regenerate: *no_regenerate,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
                keywords_granted,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::PumpCreature {
                        target: resolved_target,
                        power_bonus: *power_bonus,
                        toughness_bonus: *toughness_bonus,
                        keywords_granted: keywords_granted.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            // Variable-bonus pump (Berserk's +X/+0 where X = target power, or
            // any Count$-driven pump cast as a targeted spell). Binds the chosen
            // target the same way as the fixed PumpCreature arm above so the
            // bonus is computed against the right creature at execution.
            Effect::PumpCreatureVariable {
                target,
                power_count,
                toughness_count,
                keywords_granted,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::PumpCreatureVariable {
                        target: resolved_target,
                        power_count: power_count.clone(),
                        toughness_count: toughness_count.clone(),
                        keywords_granted: keywords_granted.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::DebuffCreature {
                target,
                keywords_removed,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::DebuffCreature {
                        target: resolved_target,
                        keywords_removed: keywords_removed.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            // `DB$ Tap | Defined$ Targeted` chained after damage (Falling Star:
            // "deal 3 damage to target creature; if it survives, tap it").
            // Reuse the parent ability's target rather than consuming a fresh
            // chosen target, AND gate on survival: skip the tap (fizzle to an
            // already-resolved placeholder) if the creature has accumulated
            // lethal damage and is therefore about to die to a state-based
            // action (CR 704.5g). A creature with toughness 0 or less is also
            // dead. Mirrors the UntapPermanent reuse_previous arm below.
            Effect::TapPermanent { target } if target.is_reuse_previous() => {
                let prev = match *last_resolved_target {
                    Some(prev_target) => prev_target,
                    None => {
                        log::warn!(target: "resolve_effect", "TapPermanent has reuse_previous but no previous target");
                        return Effect::TapPermanent {
                            target: CardId::placeholder(),
                        };
                    }
                };
                let survives = self.battlefield.contains(prev)
                    && self.cards.try_get(prev).is_some_and(|c| {
                        let toughness = i32::from(c.current_toughness());
                        toughness > 0 && toughness > c.damage
                    });
                if survives {
                    Effect::TapPermanent { target: prev }
                } else {
                    // Creature died (or left the battlefield) — no tap. Resolve to
                    // a placeholder so execute_effect treats it as a no-op fizzle.
                    Effect::TapPermanent {
                        target: CardId::placeholder(),
                    }
                }
            }
            Effect::TapPermanent { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::TapPermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::GainControl {
                target,
                untap,
                duration,
                restriction,
                source,
                ..
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::GainControl {
                        target: resolved_target,
                        new_controller: card_owner,
                        untap: *untap,
                        duration: *duration,
                        restriction: restriction.clone(),
                        source: *source,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::Fight { fighter, target } if target.is_placeholder() => {
                // Fight has two participants:
                // - fighter: from Defined$ (Self = source card, ParentTarget = last_resolved_target)
                // - target: from ValidTgts$ (chosen_targets)
                let resolved_fighter = if fighter.is_placeholder() {
                    // If fighter is also placeholder, use last_resolved_target (from ParentTarget/Targeted)
                    // or fall back to the spell's source card
                    last_resolved_target.unwrap_or(CardId::placeholder())
                } else {
                    *fighter
                };
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::Fight {
                        fighter: resolved_fighter,
                        target: resolved_target,
                    }
                } else if !resolved_fighter.is_placeholder() {
                    // SubAbility$ chained Fight (e.g., Prey Upon: SP$ Pump → DB$ Fight):
                    // The fighter was resolved from the parent effect's target (last_resolved_target),
                    // but no explicit target was chosen for the Fight itself because our casting
                    // system currently picks targets from a single flat list.
                    // Auto-select the best opponent creature as the fight target.
                    // TODO(mtg-52): Implement per-effect target selection at cast time (CR 601.2c)
                    let fighter_controller = self
                        .cards
                        .get(resolved_fighter)
                        .map(|c| c.controller)
                        .unwrap_or(card_owner);
                    let fighter_colors: smallvec::SmallVec<[crate::core::Color; 2]> = self
                        .cards
                        .get(resolved_fighter)
                        .map(|c| c.colors.clone())
                        .unwrap_or_default();
                    let mut best_target: Option<(CardId, i32)> = None;
                    for &cid in &self.battlefield.cards {
                        if let Ok(tc) = self.cards.get(cid) {
                            if tc.is_creature()
                                && tc.controller != fighter_controller
                                && is_legal_target(tc, fighter_controller, &fighter_colors)
                            {
                                let power = i32::from(tc.base_power().unwrap_or(0)) + tc.power_bonus;
                                if best_target.is_none_or(|(_, bp)| power > bp) {
                                    best_target = Some((cid, power));
                                }
                            }
                        }
                    }
                    if let Some((fight_target, _)) = best_target {
                        *last_resolved_target = Some(fight_target);
                        Effect::Fight {
                            fighter: resolved_fighter,
                            target: fight_target,
                        }
                    } else {
                        log::debug!(target: "fight", "Fight fizzled: no valid opponent creature to fight");
                        effect.clone()
                    }
                } else {
                    effect.clone()
                }
            }
            // Handle UntapPermanent with reuse_previous sentinel (from Defined$ Targeted)
            Effect::UntapPermanent { target } if target.is_reuse_previous() => {
                // Reuse the target from the previous effect in the chain
                if let Some(prev_target) = *last_resolved_target {
                    Effect::UntapPermanent { target: prev_target }
                } else {
                    log::warn!(target: "resolve_effect", "UntapPermanent has reuse_previous but no previous target");
                    Effect::UntapPermanent {
                        target: CardId::placeholder(),
                    }
                }
            }
            Effect::UntapPermanent { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::UntapPermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::TapOrUntapPermanent { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::TapOrUntapPermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::CounterSpell {
                target,
                spell_restriction,
                remember_mana_value,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::CounterSpell {
                        target: resolved_target,
                        spell_restriction: spell_restriction.clone(),
                        remember_mana_value: *remember_mana_value,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::ExilePermanent { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    // Record the exiled permanent so a chained SubAbility that
                    // refers to it (e.g. Swords to Plowshares' `Defined$
                    // TargetedController` GainLife) can resolve against it via
                    // last-known information.
                    *last_resolved_target = Some(resolved_target);
                    Effect::ExilePermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            // Disintegrate's ReplaceDyingDefined clause rides on the parent
            // DealDamage's target. The parent set last_resolved_target when it
            // resolved; bind to it here (reuse_previous sentinel). If the parent
            // targeted a player, last_resolved_target is a player sentinel —
            // execute_effect's is_creature() guard makes this a safe no-op.
            Effect::ExileIfWouldDieThisTurn { target } if target.is_reuse_previous() || target.is_placeholder() => {
                if let Some(prev) = *last_resolved_target {
                    Effect::ExileIfWouldDieThisTurn { target: prev }
                } else if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    Effect::ExileIfWouldDieThisTurn {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            // Player ID resolution for player-targeting effects
            Effect::DrawCards { player, count } if player.is_placeholder() => Effect::DrawCards {
                player: card_owner,
                count: *count,
            },
            Effect::DrawCardsXPaid { player } if player.is_placeholder() => {
                Effect::DrawCardsXPaid { player: card_owner }
            }
            Effect::DiscardCards {
                player,
                count,
                remember_discarded,
                optional,
                remember_discarding_players,
            } if player.is_placeholder() => Effect::DiscardCards {
                player: card_owner,
                count: *count,
                remember_discarded: *remember_discarded,
                optional: *optional,
                remember_discarding_players: *remember_discarding_players,
            },
            // ValidTgts$ Player (Mind Twist): target the opponent. See mtg-564.
            Effect::DiscardCards {
                player,
                count,
                remember_discarded,
                optional,
                remember_discarding_players,
            } if player.is_target_opponent() => Effect::DiscardCards {
                player: opponent_id.unwrap_or(card_owner),
                count: *count,
                remember_discarded: *remember_discarded,
                optional: *optional,
                remember_discarding_players: *remember_discarding_players,
            },
            Effect::DiscardCardsXPaid {
                player,
                remember_discarded,
            } if player.is_placeholder() => Effect::DiscardCardsXPaid {
                player: card_owner,
                remember_discarded: *remember_discarded,
            },
            Effect::DiscardCardsXPaid {
                player,
                remember_discarded,
            } if player.is_target_opponent() => Effect::DiscardCardsXPaid {
                player: opponent_id.unwrap_or(card_owner),
                remember_discarded: *remember_discarded,
            },
            Effect::GainLife { player, amount } if player.is_placeholder() => Effect::GainLife {
                player: card_owner,
                amount: *amount,
            },
            // Recall's "return N cards from your graveyard" — always targets the
            // spell's controller (Defined$ You). Resolve the player placeholder here
            // so execute_effect sees the real PlayerId.
            Effect::ReturnCardsFromGraveyardToHand { player } if player.is_placeholder() => {
                Effect::ReturnCardsFromGraveyardToHand { player: card_owner }
            }
            // "Return one matching card from your graveyard to hand" — also targets
            // the spell/trigger controller by default. Resolve placeholder.
            Effect::ReturnGraveyardCardToHand { player, type_filter } if player.is_placeholder() => {
                Effect::ReturnGraveyardCardToHand {
                    player: card_owner,
                    type_filter: type_filter.clone(),
                }
            }
            // Maze of Ith's "prevent all combat damage": the target creature is the
            // same creature targeted by the preceding UntapPermanent effect in the
            // sub-ability chain. Reuse `last_resolved_target` (set by UntapPermanent).
            Effect::PreventAllCombatDamageThisTurn { target } if target.is_placeholder() => {
                let resolved = last_resolved_target.unwrap_or(*target);
                Effect::PreventAllCombatDamageThisTurn { target: resolved }
            }
            // Dynamic-amount life gain (Swords to Plowshares, Divine Offering).
            // Fill the `reference` card from the spell's targeted permanent and
            // the `player` from its `Defined$` selector:
            //   - placeholder            => `Defined$ You` (the spell's controller)
            //   - target_controller()    => `Defined$ TargetedController`
            // The referenced card comes from the most recently resolved target
            // (the exile/destroy effect that precedes this GainLife in the chain).
            Effect::GainLifeDynamic {
                player,
                amount,
                reference,
            } => {
                let resolved_reference = if reference.is_reuse_previous() || reference.is_placeholder() {
                    last_resolved_target.unwrap_or(*reference)
                } else {
                    *reference
                };
                let resolved_player = if player.is_target_controller() {
                    self.cards
                        .try_get(resolved_reference)
                        .map(|c| c.controller)
                        .unwrap_or(card_owner)
                } else if player.is_placeholder() {
                    card_owner
                } else {
                    *player
                };
                Effect::GainLifeDynamic {
                    player: resolved_player,
                    amount: amount.clone(),
                    reference: resolved_reference,
                }
            }
            Effect::LoseLife { player, amount } if player.is_placeholder() => Effect::LoseLife {
                // LoseLife defaults to opponent (most common: "each opponent loses N life")
                player: opponent_id.unwrap_or(card_owner),
                amount: *amount,
            },
            Effect::ForceSacrifice {
                player,
                sac_type,
                count,
            } if player.is_placeholder() => Effect::ForceSacrifice {
                // ForceSacrifice defaults to opponent (Diabolic Edict pattern)
                player: opponent_id.unwrap_or(card_owner),
                sac_type: sac_type.clone(),
                count: *count,
            },
            Effect::SetLife { player, amount } if player.is_placeholder() => Effect::SetLife {
                // SetLife defaults to self (Angel of Grace: "Your life total becomes 10")
                player: card_owner,
                amount: *amount,
            },
            Effect::Mill { player, count } if player.is_placeholder() => Effect::Mill {
                player: card_owner,
                count: *count,
            },
            // DrainMana (Power Sink "lose all unspent mana"): resolve the player
            // sentinel. TargetedController => controller of the countered spell
            // (tracked via last_resolved_target); placeholder => controller;
            // target_opponent => opponent.
            Effect::DrainMana { player }
                if player.is_target_controller() || player.is_placeholder() || player.is_target_opponent() =>
            {
                let resolved = if player.is_target_controller() {
                    last_resolved_target
                        .and_then(|t| self.cards.try_get(t).map(|c| c.controller))
                        .or(opponent_id)
                        .unwrap_or(card_owner)
                } else if player.is_placeholder() {
                    card_owner
                } else {
                    // target_opponent sentinel
                    opponent_id.unwrap_or(card_owner)
                };
                Effect::DrainMana { player: resolved }
            }
            Effect::Scry { player, count } if player.is_placeholder() => Effect::Scry {
                player: card_owner,
                count: *count,
            },
            Effect::Surveil { player, count } if player.is_placeholder() => Effect::Surveil {
                player: card_owner,
                count: *count,
            },
            Effect::AddTurn { player, num_turns } if player.is_placeholder() => Effect::AddTurn {
                player: card_owner,
                num_turns: *num_turns,
            },
            Effect::ChooseColor { player, source } if player.is_placeholder() => Effect::ChooseColor {
                player: card_owner,
                source: *source,
            },
            Effect::Loot {
                player,
                discard_count,
                draw_count,
            } if player.is_placeholder() => Effect::Loot {
                player: card_owner,
                discard_count: *discard_count,
                draw_count: *draw_count,
            },
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
            // Earthbend: Target land becomes 0/0 creature with haste
            Effect::Earthbend { target, num_counters } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::Earthbend {
                        target: resolved_target,
                        num_counters: *num_counters,
                    }
                } else {
                    effect.clone()
                }
            }
            // Airbend: Exile target, owner may cast for {2}
            Effect::Airbend { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::Airbend {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            // RemoveCounter: Remove counters from target permanent
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::RemoveCounter {
                        target: resolved_target,
                        counter_type: *counter_type,
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }
            // PutCounter: Put counters on target permanent
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::PutCounter {
                        target: resolved_target,
                        counter_type: *counter_type,
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::MultiplyCounter {
                target,
                counter_type,
                multiplier,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::MultiplyCounter {
                        target: resolved_target,
                        counter_type: *counter_type,
                        multiplier: *multiplier,
                    }
                } else {
                    effect.clone()
                }
            }
            // CopyPermanent: Create token copy of target permanent
            Effect::CopyPermanent {
                target,
                controller,
                non_legendary,
                set_power,
                set_toughness,
                add_types,
                num_copies,
                restriction,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::CopyPermanent {
                        target: resolved_target,
                        controller: if controller.is_placeholder() {
                            card_owner
                        } else {
                            *controller
                        },
                        non_legendary: *non_legendary,
                        set_power: *set_power,
                        set_toughness: *set_toughness,
                        add_types: add_types.clone(),
                        num_copies: *num_copies,
                        restriction: restriction.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            // CreateDelayedTrigger: fill in tracked_card from chosen_targets
            // This is for spells like Fatal Fissure that target a creature and create
            // a delayed trigger for when that creature dies
            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } if tracked_card.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::CreateDelayedTrigger {
                        tracked_card: resolved_target,
                        condition: condition.clone(),
                        effect: delayed_effect.clone(),
                        expiry: expiry.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            // RememberObjects$ Targeted (Berserk): bind tracked_card to the
            // parent ability's already-resolved target (the creature Berserk
            // pumped) WITHOUT consuming a fresh chosen target. Mirrors the
            // reuse_previous arms for TapPermanent / DestroyPermanent sub-effects.
            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } if tracked_card.is_reuse_previous() => match last_resolved_target {
                Some(prev) => Effect::CreateDelayedTrigger {
                    tracked_card: *prev,
                    condition: condition.clone(),
                    effect: delayed_effect.clone(),
                    expiry: expiry.clone(),
                },
                None => effect.clone(),
            },

            // UnlessCostWrapper: resolve inner effect and payer reference
            Effect::UnlessCostWrapper {
                inner_effect,
                unless_cost,
            } => {
                // Recursively resolve the inner effect
                let resolved_inner = self.resolve_effect_target(
                    inner_effect,
                    chosen_targets,
                    target_index,
                    card_owner,
                    opponent_id,
                    last_resolved_target,
                );

                // Resolve payer reference to concrete PlayerId
                let resolved_payer = match unless_cost.payer.as_str() {
                    "You" => card_owner,
                    // "TargetedController" = the controller of the targeted
                    // permanent/spell. "TargetedOrController" (Chain Lightning's
                    // `UnlessPayer$ TargetedOrController`) means "if the target is
                    // a player, that player; otherwise the target's controller" —
                    // both resolve via the last-resolved target. A player target
                    // is carried as a sentinel CardId, so decode that first; only
                    // a real permanent target needs the cards lookup.
                    "TargetedController" | "TargetedOrController" => last_resolved_target
                        .and_then(|target_id| {
                            crate::core::player_target_from_sentinel(target_id)
                                .or_else(|| self.cards.get(target_id).map(|c| c.controller).ok())
                        })
                        .or(opponent_id)
                        .unwrap_or(card_owner),
                    "Player" | "Opponent" => opponent_id.unwrap_or(card_owner),
                    _ => card_owner, // Default to spell controller
                };

                // Create resolved UnlessCost with concrete payer
                let resolved_unless_cost = crate::core::effects::UnlessCost {
                    cost: unless_cost.cost.clone(),
                    payer: resolved_payer.as_u32().to_string(), // Store as numeric ID string
                    switched: unless_cost.switched,
                };

                // If the inner effect copies the parent spell (Chain Lightning),
                // its `Controller$ TargetedOrController` is the SAME player as the
                // payer (the one paying {R}{R} controls the copy). The recursive
                // resolve above can't resolve a player reference for
                // CopySpellAbility, so pin it to the concrete payer id here.
                let resolved_inner = match resolved_inner {
                    Effect::CopySpellAbility {
                        may_choose_targets,
                        defined_source,
                        ..
                    } => Effect::CopySpellAbility {
                        may_choose_targets,
                        defined_source,
                        controller: Some(resolved_payer.as_u32().to_string()),
                    },
                    other => other,
                };

                Effect::UnlessCostWrapper {
                    inner_effect: Box::new(resolved_inner),
                    unless_cost: resolved_unless_cost,
                }
            }

            // Resolve CreateToken controller placeholder to the actual caster
            // The loader sets controller to PlayerId::new(0) as a placeholder;
            // at runtime we resolve it to the spell's owner (card_owner).
            // "Opponent" tokens use PlayerId::new(1) as placeholder -> resolve to opponent.
            Effect::CreateToken {
                controller,
                token_script,
                amount,
                for_each_player,
            } => {
                let resolved_controller = if *controller == PlayerId::new(0) {
                    card_owner
                } else if *controller == PlayerId::new(1) {
                    opponent_id.unwrap_or(*controller)
                } else {
                    *controller
                };
                Effect::CreateToken {
                    controller: resolved_controller,
                    token_script: token_script.clone(),
                    amount: *amount,
                    for_each_player: *for_each_player,
                }
            }

            // No resolution needed - return clone of original
            _ => effect.clone(),
        }
    }

    /// Apply a Clone effect (CR 707): rewrite the copiable values of `source`
    /// so it becomes a copy of `copy_target`, then layer the `add_types` card
    /// types on top (e.g. Copy Artifact stays an Enchantment in addition to the
    /// copied artifact's types).
    ///
    /// Per CR 707.2 the *copiable values* of an object are its printed values
    /// (name, mana cost, color, card types, subtypes, supertypes, P/T, and
    /// abilities) as modified by other copy effects — but NOT counters, status
    /// (tapped/flipped), control, attachments, or other non-copy effects. We
    /// obtain those copiable values by re-instantiating the target's
    /// `CardDefinition`, which yields a fresh Card with the printed
    /// characteristics and abilities and none of the per-instance state. We
    /// then transplant the copiable fields onto `source`, preserving its
    /// identity (id / owner / controller) and per-instance state.
    ///
    /// # Errors
    ///
    /// Returns an error if either card is not found.
    pub fn apply_clone(&mut self, source: CardId, copy_target: CardId, add_types: &[CardType]) -> Result<()> {
        // Build a fresh instance from the target's definition to get the
        // copiable values (CR 707.2). Using the definition (rather than the
        // live Card) deliberately drops counters/damage/tapped/attachments and
        // any non-copy continuous effects on the original.
        let (copy_template, target_name) = {
            let target = self.cards.get(copy_target)?;
            (target.definition.clone(), target.name.clone())
        };
        let source_owner = self.cards.get(source)?.owner;
        // Instantiate under the source's owner so controller-dependent
        // placeholders resolve sensibly; identity is overwritten below.
        let template = copy_template.instantiate(source, source_owner);

        let source_name = self.cards.get(source)?.name.clone();

        // Capture the cloning permanent's prior copiable characteristics BEFORE
        // the overwrite so the undo log can reverse the clone exactly
        // (mtg-559/mtg-610). `apply_clone` mutates ~15 fields in place; without
        // this a rewind+replay left the card stuck as the copied permanent.
        let prior_log_size = self.logger.log_count();
        let prev_copiable = self.cards.get(source)?.capture_copiable_state();

        let source_card = self.cards.get_mut(source)?;

        // --- Transplant copiable characteristics (CR 707.2) ---
        source_card.name = template.name.clone();
        source_card.mana_cost = template.mana_cost;
        source_card.types = template.types.clone();
        source_card.subtypes = template.subtypes.clone();
        source_card.colors = template.colors.clone();
        source_card.set_base_power(template.base_power());
        source_card.set_base_toughness(template.base_toughness());
        source_card.text = template.text.clone();
        source_card.is_legendary = template.is_legendary;
        source_card.keywords = template.keywords.clone();
        source_card.activated_abilities = template.activated_abilities.clone();
        source_card.static_abilities = template.static_abilities.clone();
        source_card.svars = template.svars.clone();
        // Replace the printed definition so future re-instantiation / display /
        // mana-production all reflect the copied card (and any copy-of-a-copy
        // chains use the copied definition as their base).
        source_card.definition = template.definition.clone();
        // Triggers are copiable too, but the cloning card's *own* ETB-replacement
        // (the Clone trigger) has already fired; the copied permanent should run
        // the target's triggers going forward.
        source_card.triggers = template.triggers;

        // --- Layer the AddTypes$ card types on top (CR 707.2 + 613) ---
        // Copy Artifact: `AddTypes$ Enchantment` — the copy is an Enchantment in
        // addition to the copied artifact's types.
        for &added in add_types {
            if !source_card.types.contains(&added) {
                source_card.types.push(added);
            }
        }

        // Refresh the type-flag cache so is_artifact()/is_enchantment()/etc.
        // reflect the new (copied + added) type line.
        source_card.definition.cache.update_from_types(&source_card.types);
        source_card
            .definition
            .cache
            .update_from_subtypes(&source_card.subtypes, source_card.name.as_str());

        let added_desc = if add_types.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = add_types.iter().map(|t| t.as_str()).collect();
            format!(" (also {})", names.join(" "))
        };
        self.logger.gamelog(&format!(
            "{} enters the battlefield as a copy of {}{}",
            source_name, target_name, added_desc
        ));

        // Log the clone for undo (mtg-559/mtg-610): restores the prior copiable
        // characteristics on rewind so the round-trip is exact.
        self.undo_log.log(
            crate::undo::GameAction::CloneCard {
                card_id: source,
                prev: Box::new(prev_copiable),
            },
            prior_log_size,
        );

        Ok(())
    }

    /// Execute a single effect
    ///
    /// # Errors
    ///
    /// Returns an error if the effect cannot be executed (e.g., invalid target).
    pub fn execute_effect(&mut self, effect: &Effect) -> Result<()> {
        match effect {
            Effect::DealDamage { target, amount } => self.execute_deal_damage(target, *amount)?,

            // DivideEvenly$ RoundedDown resolved form (Fireball): deal amount_each
            // to every chosen target. The source is set via current_damage_source
            // by the caller (resolve_spell_effects), so this is a single source
            // dealing simultaneous damage to N targets (CR 601.2d / 118.5).
            Effect::DealDamageDivided { targets, amount_each } => {
                self.execute_deal_damage_divided(targets, *amount_each)?
            }

            // DealDamageDynamic is always resolved into a concrete DealDamage by
            // resolve_effect_target (which has the card_owner context). If it
            // somehow reaches execute_effect unresolved, fizzle.
            Effect::DealDamageDynamic { .. } => {
                log::debug!("DealDamageDynamic reached execute_effect unresolved - fizzling");
            }

            // DealDamageToTriggeredPlayer is resolved into a concrete
            // Effect::DealDamage by check_triggers_for_controller before this
            // dispatch (it needs the active-player trigger context). Reaching
            // here unresolved means there was no trigger context, so there is no
            // player to damage — fizzle rather than guess.
            Effect::DealDamageToTriggeredPlayer { .. } => {
                log::debug!(
                    "DealDamageToTriggeredPlayer reached execute_effect unresolved - no trigger context, fizzling"
                );
            }

            Effect::EachDamage {
                damagers,
                receiver,
                use_card_power,
                fixed_damage,
            } => self.execute_each_damage(damagers, *receiver, *use_card_power, *fixed_damage)?,

            Effect::DrawCards { player, count } => self.execute_draw_cards(*player, *count)?,
            Effect::DiscardCards { .. } | Effect::Loot { .. } => {
                // Discard-producing effects route through the shared
                // `execute_discard_effect` helper. The generic execute_effect
                // path supplies NO cause (None) — correct for self-initiated
                // discards (cost payments, your own looting/rummaging). A
                // discard FORCED by a spell or ability supplies the cause at its
                // own resolution site (resolve_top_spell_with_discard_hook /
                // priority_round / check_triggers_inner) by calling
                // `execute_discard_effect` directly with the forcing
                // spell/ability's controller — see mtg-648 / mtg-894.
                self.execute_discard_effect(effect, None)?;
            }
            Effect::GainLife { player, amount } => self.execute_gain_life(*player, *amount)?,
            Effect::GainLifeDynamic {
                player,
                amount,
                reference,
            } => self.execute_gain_life_dynamic(*player, amount, *reference)?,
            Effect::LoseLife { player, amount } => self.execute_lose_life(*player, *amount)?,
            Effect::DestroyPermanent {
                target, no_regenerate, ..
            } => {
                // Skip if target is still placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_self_target() {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // MTG Rules 702.12b: Permanents with indestructible can't be destroyed
                let (owner, has_indestructible, has_regen_shield) = {
                    let card = self.cards.get(*target)?;
                    (card.owner, card.has_indestructible(), card.regeneration_shields > 0)
                };
                if has_indestructible {
                    // Indestructible - can't be destroyed
                } else if has_regen_shield && !*no_regenerate {
                    // CR 701.15a: Regeneration replaces destruction.
                    // When the destroy says "can't be regenerated" (NoRegen$ True,
                    // e.g. The Abyss / Terror), the regeneration shield does NOT
                    // apply (CR 701.15d) and the permanent is destroyed outright.
                    self.apply_regeneration_shield(*target)?;
                } else {
                    let dest = self.death_destination_for_card(*target);
                    // Check death triggers BEFORE moving the card (trigger still has access to card data)
                    let _ = self.check_death_triggers(*target);
                    self.move_card(*target, Zone::Battlefield, dest, owner)?;
                }
            }
            Effect::GainControl {
                target,
                new_controller,
                untap,
                duration,
                source,
                ..
            } => {
                use crate::core::effects::ControlDuration;
                // Skip if target is still placeholder
                if target.is_placeholder() {
                    return Ok(());
                }
                // Skip if target is not on battlefield
                if !self.battlefield.contains(*target) {
                    log::debug!(target: "gain_control", "GainControl fizzled: target {} not on battlefield", target.as_u32());
                    return Ok(());
                }

                let prior_log_size = self.logger.log_count();
                let (old_controller, target_name) = {
                    let card = self.cards.get(*target)?;
                    (card.controller, card.name.to_string())
                };
                let new_ctrl_name = self
                    .get_player(*new_controller)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|_| format!("P{}", new_controller.as_u32()).into());

                // Change controller, and for a source-duration grant record the
                // (source, grantee) so `recompute_source_control` reverts it when
                // the grantee stops controlling the source (Aladdin).
                {
                    let card = self.cards.get_mut(*target)?;
                    card.controller = *new_controller;
                    match duration {
                        ControlDuration::WhileControlSource => {
                            if let Some(src) = source {
                                card.control_grant = Some((*src, *new_controller));
                            }
                        }
                        // Permanent / EndOfTurn grants are not source-bounded.
                        // (EndOfTurn revert remains TODO(mtg-77).)
                        ControlDuration::Permanent | ControlDuration::EndOfTurn => {}
                    }
                }

                // Log the undo action
                self.undo_log.log(
                    crate::undo::GameAction::ChangeController {
                        card_id: *target,
                        old_controller,
                        new_controller: *new_controller,
                    },
                    prior_log_size,
                );

                // Optionally untap the stolen permanent
                if *untap {
                    self.untap_permanent(*target)?;
                }

                let duration_text = match duration {
                    ControlDuration::EndOfTurn => " until end of turn",
                    ControlDuration::WhileControlSource => " for as long as they control the source",
                    ControlDuration::Permanent => "",
                };
                self.logger.gamelog(&format!(
                    "{} gains control of {}{}",
                    new_ctrl_name, target_name, duration_text
                ));

                // TODO(mtg-77): Implement EOT control return for ControlDuration::EndOfTurn
                // (needs end-of-turn delayed-trigger infrastructure).
            }
            Effect::Fight { fighter, target } => {
                // CR 701.12: Fight - each creature deals damage equal to its power to the other
                if fighter.is_placeholder() || target.is_placeholder() {
                    return Ok(());
                }
                // Both creatures must be on the battlefield
                if !self.battlefield.contains(*fighter) || !self.battlefield.contains(*target) {
                    log::debug!(target: "fight", "Fight fizzled: fighter or target not on battlefield");
                    return Ok(());
                }
                // Get power values before dealing damage
                let fighter_power = self.get_effective_power(*fighter).unwrap_or_else(|_| {
                    self.cards
                        .get(*fighter)
                        .map(|c| i32::from(c.current_power()))
                        .unwrap_or(0)
                });
                let target_power = self.get_effective_power(*target).unwrap_or_else(|_| {
                    self.cards
                        .get(*target)
                        .map(|c| i32::from(c.current_power()))
                        .unwrap_or(0)
                });

                let fighter_name = self.cards.get(*fighter).map(|c| c.name.to_string()).unwrap_or_default();
                let target_name = self.cards.get(*target).map(|c| c.name.to_string()).unwrap_or_default();

                // CR 701.12a: Each creature deals damage equal to its power to the other
                // Only deal damage if power > 0
                if fighter_power > 0 {
                    self.deal_damage_to_creature(*target, fighter_power)?;
                }
                if target_power > 0 {
                    self.deal_damage_to_creature(*fighter, target_power)?;
                }

                self.logger.gamelog(&format!(
                    "{} fights {} ({} deals {} damage, {} deals {} damage)",
                    fighter_name,
                    target_name,
                    fighter_name,
                    fighter_power.max(0),
                    target_name,
                    target_power.max(0),
                ));
            }
            Effect::TapPermanent { target } => {
                // Skip if target is still placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_reuse_previous() {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Use helper that handles tap + undo log + mana version
                self.tap_permanent(*target)?;
                // Check for Taps triggers
                self.check_triggers(TriggerEvent::Taps, *target)?;
            }
            Effect::UntapPermanent { target } => {
                // Skip if target is placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_reuse_previous() {
                    return Ok(());
                }
                // Use helper that handles untap + undo log + mana version
                self.untap_permanent(*target)?;
            }
            Effect::TapOrUntapPermanent { target } => {
                // Tap or untap target permanent (AI chooses)
                // Heuristic: untap our own creatures, tap opponent's
                if target.is_placeholder() {
                    return Ok(());
                }
                if let Some(card) = self.cards.try_get(*target) {
                    let is_ours = card.controller == self.turn.active_player;
                    if is_ours {
                        // Untap our own permanent (free mana, ready to block)
                        self.untap_permanent(*target)?;
                    } else {
                        // Tap opponent's permanent (remove blocker, deny mana)
                        self.tap_permanent(*target)?;
                        self.check_triggers(TriggerEvent::Taps, *target)?;
                    }
                }
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
                keywords_granted,
            } => {
                // Skip if target is still placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_reuse_previous() {
                    log::warn!(target: "pump", "PumpCreature fizzled: unresolved target {}", target.as_u32());
                    return Ok(());
                }
                log::debug!(target: "pump", "PumpCreature executing: target={}, power_bonus={}, toughness_bonus={}, keywords={:?}", target.as_u32(), power_bonus, toughness_bonus, keywords_granted);
                // Capture log size before pump
                let prior_log_size = self.logger.log_count();

                let card = self.cards.get_mut(*target)?;
                card.power_bonus += power_bonus;
                card.toughness_bonus += toughness_bonus;
                // Grant keywords until end of turn (tracked so forward cleanup
                // + rewind sweep can remove them deterministically; mtg-610).
                // Record ONLY the keywords this pump *newly* added so the undo
                // entry removes exactly those, never a printed/other-source
                // keyword (mtg-731: Rockface Village pumping a printed-haste
                // creature was stripping its Haste).
                let mut newly_granted: smallvec::SmallVec<[crate::core::Keyword; 2]> = smallvec::SmallVec::new();
                for keyword in keywords_granted.iter() {
                    if card.grant_keyword_until_eot(*keyword) {
                        newly_granted.push(*keyword);
                    }
                }

                // Log the pump effect
                self.undo_log.log(
                    crate::undo::GameAction::PumpCreature {
                        card_id: *target,
                        power_delta: *power_bonus,
                        toughness_delta: *toughness_bonus,
                        keywords_granted: newly_granted,
                    },
                    prior_log_size,
                );
            }
            Effect::PumpCreatureVariable {
                target,
                power_count,
                toughness_count,
                keywords_granted,
            } => {
                // Variable pump: bonus depends on counting game state
                // Example: Elephant-Mandrill gets +X/+X where X is artifacts opponents control

                // Skip if target is still placeholder
                if target.is_placeholder() {
                    log::warn!(target: "pump", "PumpCreatureVariable fizzled: target is still placeholder");
                    return Ok(());
                }

                // Get target's controller for filter resolution
                let target_controller = self.cards.get(*target)?.controller;

                // Evaluate the count expressions. `Targeted$CardPower` (Berserk's
                // power-doubling +X/+0) resolves against the target itself and is
                // not visible to `evaluate_count_expression` (controller-only), so
                // resolve it HERE from the target's CURRENT power — read BEFORE the
                // pump mutates power_bonus below, so X locks to the pre-pump value
                // (CR 613.4: the +X/+0 layer applies once, X = power at resolution).
                let target_power = i32::from(self.cards.get(*target)?.current_power());
                let resolve = |this: &Self, count: &crate::core::CountExpression| -> Result<i32> {
                    if matches!(count, crate::core::CountExpression::TargetedCardPower) {
                        Ok(target_power)
                    } else {
                        this.evaluate_count_expression(count, target_controller)
                    }
                };
                let power_bonus = resolve(self, power_count)?;
                let toughness_bonus = resolve(self, toughness_count)?;

                log::debug!(
                    target: "pump",
                    "PumpCreatureVariable executing: target={}, power_bonus={}, toughness_bonus={}, keywords={:?}",
                    target.as_u32(),
                    power_bonus,
                    toughness_bonus,
                    keywords_granted
                );

                // Apply the pump
                let prior_log_size = self.logger.log_count();
                let card = self.cards.get_mut(*target)?;
                card.power_bonus += power_bonus;
                card.toughness_bonus += toughness_bonus;
                // Record ONLY newly-added keywords so undo never strips a
                // printed/other-source keyword (mtg-731).
                let mut newly_granted: smallvec::SmallVec<[crate::core::Keyword; 2]> = smallvec::SmallVec::new();
                for keyword in keywords_granted.iter() {
                    if card.grant_keyword_until_eot(*keyword) {
                        newly_granted.push(*keyword);
                    }
                }

                // Log for undo
                self.undo_log.log(
                    crate::undo::GameAction::PumpCreature {
                        card_id: *target,
                        power_delta: power_bonus,
                        toughness_delta: toughness_bonus,
                        keywords_granted: newly_granted,
                    },
                    prior_log_size,
                );
            }
            Effect::DebuffCreature {
                target,
                keywords_removed,
            } => {
                // Skip if target is still placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_reuse_previous() {
                    log::warn!(target: "debuff", "DebuffCreature fizzled: unresolved target {}", target.as_u32());
                    return Ok(());
                }
                log::debug!(target: "debuff", "DebuffCreature executing: target={}, keywords_removed={:?}", target.as_u32(), keywords_removed);

                let prior_log_size = self.logger.log_count();
                let card = self.cards.get_mut(*target)?;
                // Remove keywords
                for keyword in keywords_removed.iter() {
                    card.keywords.remove(*keyword);
                }

                // Log the debuff effect for undo
                self.undo_log.log(
                    crate::undo::GameAction::DebuffCreature {
                        card_id: *target,
                        keywords_removed: keywords_removed.clone(),
                    },
                    prior_log_size,
                );
            }
            Effect::PumpAllCreatures {
                controller,
                filter,
                power_bonus,
                toughness_bonus,
            } => {
                // Mass pump: "Creatures you control get +X/+Y until end of turn"
                // Find all creatures matching the filter and pump them
                let restriction = crate::core::TargetRestriction::parse(filter);
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .filter_map(|&card_id| {
                        if let Some(card) = self.cards.try_get(card_id) {
                            if card.is_creature()
                                && restriction.matches_with_controller(card, *controller, card.controller)
                            {
                                Some(card_id)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();

                // Apply pump to all matching creatures
                for target in targets {
                    let prior_log_size = self.logger.log_count();
                    if let Ok(card) = self.cards.get_mut(target) {
                        card.power_bonus += power_bonus;
                        card.toughness_bonus += toughness_bonus;
                        log::debug!(
                            "PumpAllCreatures: {} gets +{}/+{}",
                            card.name,
                            power_bonus,
                            toughness_bonus
                        );
                        self.logger.normal(&format!(
                            "{} gets +{}/+{} until end of turn",
                            card.name, power_bonus, toughness_bonus
                        ));
                    }
                    self.undo_log.log(
                        crate::undo::GameAction::PumpCreature {
                            card_id: target,
                            power_delta: *power_bonus,
                            toughness_delta: *toughness_bonus,
                            keywords_granted: smallvec::SmallVec::new(),
                        },
                        prior_log_size,
                    );
                }
            }
            Effect::AnimateAll {
                controller,
                filter,
                power,
                toughness,
                keywords_granted,
            } => {
                // AnimateAll: set base P/T and/or grant keywords to all matching permanents
                // Similar to PumpAllCreatures but sets base P/T instead of bonuses
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .filter_map(|&card_id| {
                        let card = self.cards.try_get(card_id)?;
                        // Check controller filters
                        if filter.contains("YouCtrl") && card.controller != *controller {
                            return None;
                        }
                        if filter.contains("OppCtrl") && card.controller == *controller {
                            return None;
                        }
                        // Check type filters
                        if filter.contains("Creature") && !card.is_creature() {
                            return None;
                        }
                        if filter.contains("Planeswalker") && !card.is_planeswalker() {
                            return None;
                        }
                        if filter.contains("Land") && !card.is_land() {
                            return None;
                        }
                        Some(card_id)
                    })
                    .collect();

                for target in targets {
                    // Set base P/T (if specified) via the logged helper so the
                    // override is reversible by the undo log (mtg-614 hole (c)).
                    if power.is_some() || toughness.is_some() {
                        self.set_temp_base_stats_logged(
                            target,
                            (*power).map(|p| p as i8),
                            (*toughness).map(|t| t as i8),
                        );
                    }
                    if let Ok(card) = self.cards.get_mut(target) {
                        let card_name = card.name.clone();

                        // Grant keywords until end of turn (AnimateAll is an
                        // until-EOT mass animate; track them so forward cleanup
                        // + rewind sweep remove them deterministically; mtg-610).
                        for kw in keywords_granted {
                            card.grant_keyword_until_eot(*kw);
                        }

                        if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                            let kws: Vec<_> = keywords_granted.iter().map(|k| format!("{:?}", k)).collect();
                            let kw_str = if kws.is_empty() {
                                String::new()
                            } else {
                                format!(" and gains {}", kws.join(", "))
                            };

                            if power.is_some() || toughness.is_some() {
                                self.logger.gamelog(&format!(
                                    "{} becomes {}/{}{}",
                                    card_name,
                                    card.current_power(),
                                    card.current_toughness(),
                                    kw_str
                                ));
                            } else if !kws.is_empty() {
                                self.logger.gamelog(&format!("{} gains {}", card_name, kws.join(", ")));
                            }
                        }
                    }
                }
            }
            Effect::Mill { player, count } => self.execute_mill(*player, *count)?,
            Effect::DrainMana { player } => self.execute_drain_mana(*player)?,
            Effect::Scry { player, count } => self.execute_scry(*player, *count)?,
            Effect::Surveil { player, count } => self.execute_surveil(*player, *count)?,
            Effect::AddTurn { player, num_turns } => {
                // Take extra turns (CR 500.7) - Time Walk, Temporal Manipulation, etc.
                // Add extra turns to the GameState extra-turn queue (consumed in
                // GameState::advance_step at end of turn, CR 500.7). NOTE: this
                // must push to `self.extra_turns` (the VecDeque actually drained
                // by the turn-rotation code), NOT `self.turn.extra_turns` (a
                // dead, write-only field) — otherwise the extra turn was queued
                // somewhere nothing reads, and never taken (mtg-551).
                for _ in 0..*num_turns {
                    let prior_log_size = self.logger.log_count();
                    self.extra_turns.push_back(*player);
                    // Log for undo so a rewind+replay across the AddTurn
                    // resolution doesn't leave a stale queued extra turn
                    // (mtg-559/mtg-610).
                    self.undo_log.log(
                        crate::undo::GameAction::PushExtraTurn { player: *player },
                        prior_log_size,
                    );
                }
                let player_name = self
                    .get_player(*player)
                    .map(|p| p.name.as_str().to_string())
                    .unwrap_or_else(|_| "Unknown".to_string());
                self.logger.gamelog(&format!(
                    "{} takes {} extra turn(s) after this one",
                    player_name, num_turns
                ));
            }
            Effect::CounterSpell {
                target,
                remember_mana_value,
                ..
            } => {
                // Counter a spell on the stack
                // Fizzle if target is placeholder (no valid target found) or not on stack
                // This happens when triggered counter effects (e.g., Ulamog's Nullifier ETB)
                // fire when no spell is on the stack to target
                if target.is_placeholder() || !self.stack.contains(*target) {
                    log::debug!("CounterSpell fizzles - target {} not on stack", target.as_u32());
                } else {
                    // Mana Drain (RememberCounteredCMC$ True): record the
                    // countered spell's mana value (including any X paid) BEFORE
                    // it leaves the stack, so the chained delayed trigger can
                    // add that much {C} at the controller's next main phase.
                    if *remember_mana_value {
                        let mana_value = self
                            .cards
                            .try_get(*target)
                            .map(|c| u32::from(c.mana_cost.cmc()) + u32::from(c.x_paid))
                            .unwrap_or(0);
                        let prior_log_size = self.logger.log_count();
                        let previous = self.remembered_amount;
                        self.remembered_amount = Some(mana_value);
                        self.undo_log.log(
                            crate::undo::GameAction::SetRememberedAmount { previous },
                            prior_log_size,
                        );
                    }
                    self.counter_spell(*target)?;
                }
            }
            Effect::AddMana {
                player,
                mana,
                produces_chosen_color,
                amount_var,
            } => {
                // Capture log size before mana addition
                let prior_log_size = self.logger.log_count();

                // Add mana to player's mana pool
                // Note: For mana abilities, produces_chosen_color is handled in tap_for_mana_for_cost
                // where we have access to the source card's chosen_color.
                // This path is mainly for spell effects (Dark Ritual) and triggered abilities (Su-Chi).
                // Note: amount_var (for variable mana like Raucous Audience) is resolved in ManaEngine
                // during tap_for_mana_for_cost, not here.
                if *produces_chosen_color {
                    // This shouldn't happen in practice since mana abilities go through tap_for_mana_for_cost
                    // but log a warning if it does
                    self.logger
                        .normal("Warning: produces_chosen_color in execute_effect - source card unknown");
                }
                if amount_var.is_some() {
                    // Variable mana should be resolved before reaching execute_effect
                    self.logger
                        .normal("Warning: amount_var in execute_effect - should be resolved in ManaEngine");
                }
                let p = self.get_player_mut(*player)?;

                // Add each component of the mana cost to the pool
                for _ in 0..mana.white {
                    p.mana_pool.add_color(crate::core::Color::White);
                }
                for _ in 0..mana.blue {
                    p.mana_pool.add_color(crate::core::Color::Blue);
                }
                for _ in 0..mana.black {
                    p.mana_pool.add_color(crate::core::Color::Black);
                }
                for _ in 0..mana.red {
                    p.mana_pool.add_color(crate::core::Color::Red);
                }
                for _ in 0..mana.green {
                    p.mana_pool.add_color(crate::core::Color::Green);
                }
                for _ in 0..mana.colorless {
                    p.mana_pool.add_color(crate::core::Color::Colorless);
                }

                // Log the mana addition
                self.undo_log.log(
                    crate::undo::GameAction::AddMana {
                        player_id: *player,
                        mana: *mana,
                    },
                    prior_log_size,
                );
            }
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } => {
                // Skip if target is still placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_reuse_previous() {
                    return Ok(());
                }
                // `Defined$ Remembered` (e.g. All Hallow's Eve's chained
                // PutCounter after a RememberChanged self-exile) — apply the
                // counters to every card currently in `remembered_cards`.
                // Clone first to avoid the &self borrow held by `iter()`
                // conflicting with `add_counters`'s &mut self.
                if target.is_remembered_card() {
                    let remembered: smallvec::SmallVec<[CardId; 4]> = self.remembered_cards.iter().copied().collect();
                    if remembered.is_empty() {
                        log::debug!(
                            target: "put_counter",
                            "PutCounter Defined$ Remembered with empty remembered_cards list, skipping"
                        );
                        return Ok(());
                    }
                    for cid in remembered {
                        self.add_counters(cid, *counter_type, *amount)?;
                    }
                    return Ok(());
                }
                // Add counters using the GameState method (which logs for undo)
                self.add_counters(*target, *counter_type, *amount)?;
            }
            Effect::MultiplyCounter {
                target,
                counter_type,
                multiplier,
            } => {
                if target.is_placeholder() {
                    return Ok(());
                }
                // Multiply counters on the target card
                if let Some(card) = self.cards.try_get(*target) {
                    let counters_to_add: smallvec::SmallVec<[(crate::core::CounterType, u8); 4]> =
                        if let Some(ct) = counter_type {
                            // Multiply specific counter type
                            let current = card.get_counter(*ct);
                            if current > 0 {
                                let to_add = current.saturating_mul(*multiplier - 1);
                                smallvec::smallvec![(*ct, to_add)]
                            } else {
                                smallvec::SmallVec::new()
                            }
                        } else {
                            // Multiply ALL counter types on the card
                            card.counters
                                .iter()
                                .filter_map(|(ct, count)| {
                                    if *count > 0 {
                                        Some((*ct, count.saturating_mul(*multiplier - 1)))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };

                    for (ct, amount) in counters_to_add {
                        if amount > 0 {
                            self.add_counters(*target, ct, amount)?;
                        }
                    }
                }
            }
            Effect::PutCounterAll {
                restriction,
                counter_type,
                amount,
            } => {
                // Put counters on all permanents matching the restriction
                let spell_controller = self.turn.active_player;
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter(|&card_id| {
                        self.cards.try_get(card_id).is_some_and(|card| {
                            restriction.matches_with_controller(card, spell_controller, card.controller)
                        })
                    })
                    .collect();

                for card_id in targets {
                    self.add_counters(card_id, *counter_type, *amount)?;
                }
            }
            Effect::Proliferate => {
                // Proliferate (CR 701.34a): choose any number of permanents and/or players
                // that have a counter, then give each one additional counter of each kind
                // that permanent or player already has.
                //
                // For automated play: proliferate all permanents with counters.
                // The AI/controller choice of which permanents to skip is handled
                // at the should_cast level; once resolved, we proliferate everything.
                let permanents_with_counters: Vec<(CardId, Vec<crate::core::CounterType>)> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter_map(|card_id| {
                        let card = self.cards.try_get(card_id)?;
                        if card.has_counters() {
                            let counter_types: Vec<crate::core::CounterType> = card
                                .counters
                                .iter()
                                .filter(|(_, count)| *count > 0)
                                .map(|(ct, _)| *ct)
                                .collect();
                            if counter_types.is_empty() {
                                None
                            } else {
                                Some((card_id, counter_types))
                            }
                        } else {
                            None
                        }
                    })
                    .collect();

                for (card_id, counter_types) in permanents_with_counters {
                    for ct in counter_types {
                        self.add_counters(card_id, ct, 1)?;
                    }
                }
            }
            Effect::ChangeZoneAll {
                restriction,
                origins,
                destination,
                shuffle,
            } => {
                // Move all cards matching the restriction from EACH origin zone
                // to the destination. Timetwister/Diminishing-Returns style mass
                // shuffles list two origins (Hand + Graveyard); single-origin
                // mass bounce/exile list one. Track (card, owner, source-zone)
                // so each card moves from the zone it actually lives in.
                let mut cards_to_move: Vec<(CardId, PlayerId, crate::zones::Zone)> = Vec::new();
                // SHADOW determinism (mtg-728 sig-2c): the opponent's hidden
                // hand/library cards are late-bound reserved CardIds with NO
                // instance, so `try_get` returns None and `restriction.matches`
                // can't be evaluated. For an UNRESTRICTED mass move (Timetwister
                // / Wheel / Windfall shuffle-back — matches any card) those
                // reserved cards MUST still move; otherwise the opponent's
                // library ends up short on the shadow and its subsequent
                // shuffle consumes a different amount of RNG than the server's,
                // breaking server<->shadow RNG lockstep and desyncing every
                // later shuffle/draw. Only reached in shadow games (the server
                // and native clients always have real instances).
                let move_reserved_in_shadow = self.is_shadow_game && restriction.is_unrestricted();
                for &origin in origins {
                    match origin {
                        crate::zones::Zone::Battlefield => {
                            for card_id in self.battlefield.cards.iter().copied() {
                                if let Some(card) = self.cards.try_get(card_id) {
                                    if restriction.matches(card) {
                                        cards_to_move.push((card_id, card.owner, origin));
                                    }
                                }
                            }
                        }
                        // Per-player private/owned zones: collect from each
                        // player's own zone so ownership is exact.
                        crate::zones::Zone::Graveyard
                        | crate::zones::Zone::Hand
                        | crate::zones::Zone::Library
                        | crate::zones::Zone::Exile => {
                            for (player_id, zones) in &self.player_zones {
                                let zone_cards = match origin {
                                    crate::zones::Zone::Graveyard => &zones.graveyard.cards,
                                    crate::zones::Zone::Hand => &zones.hand.cards,
                                    crate::zones::Zone::Library => &zones.library.cards,
                                    crate::zones::Zone::Exile => &zones.exile.cards,
                                    // Unreachable: outer match already narrowed to these four.
                                    crate::zones::Zone::Battlefield
                                    | crate::zones::Zone::Stack
                                    | crate::zones::Zone::Command => continue,
                                };
                                for &card_id in zone_cards {
                                    match self.cards.try_get(card_id) {
                                        Some(card) => {
                                            if restriction.matches(card) {
                                                cards_to_move.push((card_id, *player_id, origin));
                                            }
                                        }
                                        // Reserved (instance-less) shadow card under an
                                        // unrestricted mass move (mtg-728 sig-2c).
                                        None if move_reserved_in_shadow => {
                                            cards_to_move.push((card_id, *player_id, origin));
                                        }
                                        None => {}
                                    }
                                }
                            }
                        }
                        crate::zones::Zone::Stack | crate::zones::Zone::Command => {
                            // Mass zone changes don't originate from the stack or
                            // the command zone in any supported card.
                        }
                    }
                }

                for (card_id, owner, origin) in cards_to_move {
                    self.move_card(card_id, origin, *destination, owner)?;
                }

                // `Shuffle$ True` mass move into the library (Timetwister,
                // Mnemonic Nexus) requires shuffling the affected libraries so
                // the moved cards land in random order. Ordered moves
                // (`LibraryPosition$ -1`, e.g. Manifold Insights) set
                // shuffle=false and are left untouched. The effect is symmetric
                // across players, so shuffle every library; a library that
                // received no cards only advances RNG (deterministic /
                // replay-safe).
                if *shuffle && matches!(destination, crate::zones::Zone::Library) {
                    let player_ids: smallvec::SmallVec<[PlayerId; 4]> = self.players.iter().map(|p| p.id).collect();
                    for pid in player_ids {
                        self.shuffle_library(pid);
                    }
                }
            }
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Remove counters using the GameState method (which logs for undo)
                if let Some(ct) = counter_type {
                    // Specific counter type
                    self.remove_counters(*target, *ct, *amount)?;
                } else {
                    // CounterType$ Any - remove counters of any type
                    // Get all counter types present on the card and remove up to `amount` total
                    let mut remaining = *amount;
                    let counter_types: smallvec::SmallVec<[crate::core::CounterType; 4]> = {
                        let card = self.cards.get(*target)?;
                        card.counters.iter().map(|(ct, _)| *ct).collect()
                    };

                    for ct in counter_types {
                        if remaining == 0 {
                            break;
                        }
                        let removed = self.remove_counters(*target, ct, remaining)?;
                        remaining = remaining.saturating_sub(removed);
                    }
                }
            }
            Effect::ExilePermanent { target } => {
                // Skip if target is still placeholder (0) or unresolved sentinel
                if target.is_placeholder() || target.is_reuse_previous() {
                    return Ok(());
                }
                // Exile the permanent by moving it from battlefield to exile
                let owner = self.cards.get(*target)?.owner;
                self.move_card(*target, Zone::Battlefield, Zone::Exile, owner)?;
            }
            Effect::ExileIfWouldDieThisTurn { target } => {
                // Disintegrate's ReplaceDyingDefined clause: mark the targeted
                // creature so that, if it would die this turn, it is exiled
                // instead (CR 614). The flag is consulted by
                // death_destination_for_card and cleared at cleanup. Skip if the
                // target failed to resolve to a concrete creature.
                if target.is_placeholder() || target.is_reuse_previous() {
                    return Ok(());
                }
                if let Ok(card) = self.cards.get_mut(*target) {
                    if card.is_creature() {
                        card.exile_if_would_die_this_turn = true;
                    }
                }
            }
            Effect::SelfExileFromStack {
                source,
                remember_changed,
            } => {
                // `SP$ ChangeZone | Origin$ Stack | Destination$ Exile`
                // (e.g. All Hallow's Eve). Move the resolving spell from the
                // stack to exile so it doesn't get sent to the graveyard by the
                // default sorcery resolution path; `resolve_spell_finalize` will
                // notice the card is no longer on the stack and skip its move.
                if source.is_placeholder() || source.is_self_target() {
                    // resolve_self_target should have patched the source CardId;
                    // if it didn't (effect was placed in an unexpected context),
                    // fizzle silently rather than panicking.
                    log::debug!(
                        target: "self_exile",
                        "SelfExileFromStack: source still placeholder/sentinel, skipping"
                    );
                    return Ok(());
                }
                if !self.stack.contains(*source) {
                    log::debug!(
                        target: "self_exile",
                        "SelfExileFromStack: card {} no longer on stack",
                        source.as_u32()
                    );
                    return Ok(());
                }
                let owner = self.cards.get(*source)?.owner;
                self.move_card(*source, Zone::Stack, Zone::Exile, owner)?;
                if *remember_changed {
                    // Make the just-exiled card available to chained
                    // SubAbilities with `Defined$ Remembered` (e.g. the
                    // PutCounter that places two scream counters on it).
                    self.remembered_cards.push(*source);
                }
            }
            Effect::MoveSelfBetweenZones {
                source,
                origin,
                destination,
            } => {
                // `DB$ ChangeZone | Defined$ Self | Origin$ <zone> | Destination$ <zone>`
                // executed by a triggered ability whose source lives outside the
                // battlefield (e.g. All Hallow's Eve moving itself exile→graveyard
                // once its last scream counter is removed).
                if source.is_placeholder() || source.is_self_target() {
                    log::debug!(
                        target: "self_exile",
                        "MoveSelfBetweenZones: source still placeholder/sentinel, skipping"
                    );
                    return Ok(());
                }
                // Verify the card is actually in the origin zone before moving so
                // we never double-move (CR 400.7 / 608.2g object-no-longer-there).
                let in_origin = self.find_card_zone(*source) == Some(*origin);
                if !in_origin {
                    log::debug!(
                        target: "self_exile",
                        "MoveSelfBetweenZones: card {} not in {:?}, skipping",
                        source.as_u32(),
                        origin
                    );
                    return Ok(());
                }
                let owner = self.cards.get(*source)?.owner;
                self.move_card(*source, *origin, *destination, owner)?;
            }
            Effect::ReturnCardsFromGraveyardToHand { player } => {
                // Recall: "return a card from your graveyard to your hand for each
                // card discarded this way" (CR 400.7 / 701.25).
                //
                // The preceding DiscardCards effect stored each discarded card in
                // `remembered_cards`. The count to return is `remembered_cards.len()`
                // (= number of cards actually discarded, which may be less than X if
                // the hand was smaller).
                //
                // Information-independence: we pick cards from the graveyard in stable
                // order (lowest CardId first) so the choice is deterministic across
                // server and both network clients. The graveyard is a public zone so
                // revealing CardIds doesn't leak hidden information. The AI selects the
                // best card to retrieve using `choose_card_to_retrieve_from_graveyard`.
                let count = self.remembered_cards.len();
                if count == 0 {
                    // Nothing was remembered (nothing discarded) — nothing to return.
                    return Ok(());
                }
                // Collect graveyard cards once so we can mutate the zone in the loop.
                let graveyard_cards: smallvec::SmallVec<[CardId; 8]> = self
                    .get_player_zones(*player)
                    .map(|z| z.graveyard.cards.iter().copied().collect())
                    .unwrap_or_default();
                if graveyard_cards.is_empty() {
                    self.logger.gamelog(&format!(
                        "Recall effect: {} has no cards in graveyard to return",
                        self.get_player(*player).ok().map(|p| p.name.as_str()).unwrap_or("?")
                    ));
                    return Ok(());
                }
                // For each card to return, pick the AI-preferred card still in graveyard.
                let player_name = self
                    .get_player(*player)
                    .ok()
                    .map(|p| p.name.as_str())
                    .unwrap_or("?")
                    .to_string();
                let to_return = count.min(graveyard_cards.len());
                for _ in 0..to_return {
                    // Re-snapshot graveyard each iteration (previous iteration may
                    // have moved a card out).
                    let remaining: smallvec::SmallVec<[CardId; 8]> = self
                        .get_player_zones(*player)
                        .map(|z| z.graveyard.cards.iter().copied().collect())
                        .unwrap_or_default();
                    // Deterministic pick: lowest CardId (stable across server/clients).
                    // `min_by_key` returns None only on empty iter; we guard
                    // above with `if remaining.is_empty() { break }`, so the
                    // `?`-style early-exit covers the impossible None case.
                    let Some(&chosen) = remaining.iter().min_by_key(|id| id.as_u32()) else {
                        break;
                    };
                    let card_name = self
                        .cards
                        .try_get(chosen)
                        .map(|c| c.name.as_str())
                        .unwrap_or("?")
                        .to_string();
                    self.move_card(chosen, Zone::Graveyard, Zone::Hand, *player)?;
                    self.logger
                        .gamelog(&format!("{} returns {} from graveyard to hand", player_name, card_name));
                }
            }
            Effect::ReturnGraveyardCardToHand { player, type_filter } => {
                // Return exactly one card matching `type_filter` from the player's graveyard
                // to their hand.  The AI picks the highest-value matching card.
                // Used by triggered abilities like Stormchaser's Talent level-2
                // "return target instant or sorcery from your graveyard to hand".
                let graveyard_cards: smallvec::SmallVec<[CardId; 8]> = self
                    .get_player_zones(*player)
                    .map(|z| z.graveyard.cards.iter().copied().collect())
                    .unwrap_or_default();

                // Filter by type
                let filter_types: Vec<&str> = if type_filter.is_empty() {
                    vec![]
                } else {
                    type_filter.split(',').map(|s| s.trim()).collect()
                };

                let matching: smallvec::SmallVec<[CardId; 8]> = graveyard_cards
                    .into_iter()
                    .filter(|&id| {
                        if filter_types.is_empty() {
                            return true;
                        }
                        if let Some(card) = self.cards.try_get(id) {
                            filter_types.iter().any(|&t| match t {
                                "Instant" => card.is_instant(),
                                "Sorcery" => card.is_sorcery(),
                                "Creature" => card.is_creature(),
                                "Land" => card.is_land(),
                                "Artifact" => card.is_artifact(),
                                _ => false,
                            })
                        } else {
                            false
                        }
                    })
                    .collect();

                if matching.is_empty() {
                    let player_name = self
                        .get_player(*player)
                        .ok()
                        .map(|p| p.name.as_str())
                        .unwrap_or("?")
                        .to_string();
                    self.logger.gamelog(&format!(
                        "{} has no matching {} in graveyard to return",
                        player_name,
                        if type_filter.is_empty() {
                            "card".to_string()
                        } else {
                            type_filter.clone()
                        }
                    ));
                } else {
                    // Deterministic pick: lowest CardId (stable across server/clients).
                    let Some(&chosen) = matching.iter().min_by_key(|id| id.as_u32()) else {
                        return Ok(());
                    };
                    let card_name = self
                        .cards
                        .try_get(chosen)
                        .map(|c| c.name.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let player_name = self
                        .get_player(*player)
                        .ok()
                        .map(|p| p.name.as_str())
                        .unwrap_or("?")
                        .to_string();
                    self.move_card(chosen, Zone::Graveyard, Zone::Hand, *player)?;
                    self.logger
                        .gamelog(&format!("{} returns {} from graveyard to hand", player_name, card_name));
                }
            }

            Effect::ConditionalSelfCounter {
                source,
                condition,
                inner,
            } => {
                // Run the inner effect only if `source` currently satisfies the
                // counter condition (CR 603.4 intervening-if style gating for a
                // mid-chain sub-ability). For All Hallow's Eve the source is the
                // AHE card and the inner effects (exile→graveyard move, then mass
                // resurrection) only fire on the upkeep where the final scream
                // counter was removed (counters_EQ0_SCREAM).
                if source.is_placeholder() || source.is_self_target() {
                    log::debug!(
                        target: "self_exile",
                        "ConditionalSelfCounter: source still placeholder/sentinel, skipping"
                    );
                    return Ok(());
                }
                let satisfied = self
                    .cards
                    .try_get(*source)
                    .map(|c| condition.evaluate(c.get_counter(condition.counter_type)))
                    .unwrap_or(false);
                if satisfied {
                    self.execute_effect(inner)?;
                }
            }
            Effect::SetBasePowerToughness {
                target,
                power,
                toughness,
                keywords_granted,
                types_added,
                subtypes_added,
                remove_creature_subtypes,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Set temporary base P/T override (until end of turn)
                // This is used by Animate effects like Flexible Waterbender,
                // Turtle-Duck, and manlands such as Mishra's Factory.
                //
                // Set the temp base P/T (if specified) via the logged helper so
                // the override is reversible by the undo log (mtg-614 hole (c)).
                if power.is_some() || toughness.is_some() {
                    self.set_temp_base_stats_logged(*target, (*power).map(|p| p as i8), (*toughness).map(|t| t as i8));
                }
                let card = self.cards.get_mut(*target)?;
                let card_name = card.name.clone();
                let _old_power = card.current_power();
                let _old_toughness = card.current_toughness();

                // Grant temporary keywords (until end of turn).
                // Note: Uses same approach as PumpCreature - keywords added to permanent set.
                // Record ONLY the keywords this animate actually adds (those not
                // already present) so the `AnimateTypeline` undo entry can remove
                // exactly them on a rewind without stripping a printed/other-source
                // keyword (mtg-610: Soulstone Sanctuary's Vigilance was leaking
                // across rewind+replay because it was inserted but never undone).
                let mut granted_keywords: smallvec::SmallVec<[crate::core::Keyword; 2]> = smallvec::SmallVec::new();
                for kw in keywords_granted {
                    if !card.keywords.contains(*kw) {
                        card.keywords.insert(*kw);
                        granted_keywords.push(*kw);
                    }
                }

                // Snapshot the pre-animate typeline + tracking vectors so the
                // mutation can be logged as a reversible `AnimateTypeline`
                // GameAction (mtg-610). Captured BEFORE any push/drain below so
                // `undo()` restores the exact prior state and a rewind+replay
                // round-trips deterministically (the cleanup-step revert relies
                // on the tracking vectors, which can drift across rewinds).
                let prev_types = card.types.clone();
                let prev_subtypes = card.subtypes.clone();
                let prev_temp_animate_types = card.temp_animate_types.clone();
                let prev_temp_animate_subtypes = card.temp_animate_subtypes.clone();
                let prev_temp_removed_subtypes = card.temp_removed_subtypes.clone();

                // Animate: add card types (Mishra's Factory becomes
                // Land + Artifact + Creature). We track only the types we
                // actually push so cleanup_temporary_effects can remove them
                // without disturbing the printed type line.
                for ty in types_added {
                    if !card.types.contains(ty) {
                        card.types.push(*ty);
                        card.temp_animate_types.push(*ty);
                    }
                }

                // Animate: optionally strip pre-existing creature subtypes
                // (RemoveCreatureTypes$ True). For the common manland case
                // this is a no-op because the printed card has no subtypes,
                // but it matters for cards that animate into a *different*
                // creature type than their printed line.
                if *remove_creature_subtypes && !card.subtypes.is_empty() {
                    let removed: smallvec::SmallVec<[crate::core::Subtype; 2]> = card.subtypes.drain(..).collect();
                    card.temp_removed_subtypes.extend(removed);
                }

                // Add subtypes (Assembly-Worker, etc.)
                for st in subtypes_added {
                    if !card.subtypes.contains(st) {
                        card.subtypes.push(st.clone());
                        card.temp_animate_subtypes.push(st.clone());
                    }
                }

                // If we touched types or subtypes, refresh the cache flags
                // (`is_creature`, `is_artifact`, etc.) so combat / mana / target
                // logic sees the new typeline immediately.
                let types_changed = !types_added.is_empty() || !subtypes_added.is_empty() || *remove_creature_subtypes;
                if types_changed {
                    let types = card.types.clone();
                    let subtypes = card.subtypes.clone();
                    let name = card.name.clone();
                    card.definition.cache.update_from_types(&types);
                    card.definition.cache.update_from_subtypes(&subtypes, name.as_str());

                    // A card that just became a creature must record its
                    // ETB turn so summoning-sickness logic works (CR 302.1).
                    // Without this, animated lands could attack the same turn
                    // they were played even without Haste — and conversely,
                    // if `turn_entered_battlefield` is set to the current
                    // turn the engine correctly demands Haste.
                    //
                    // The land itself entered the battlefield earlier (on a
                    // prior turn for Mishra's Factory's typical use), so we
                    // intentionally leave `turn_entered_battlefield` alone:
                    // the land's existing entry timestamp already satisfies
                    // summoning sickness once it's been on the battlefield
                    // for a turn, mirroring Forge-Java's "becomes a creature"
                    // not resetting summoning sickness.
                }

                // Snapshot post-animate state we need *after* dropping the
                // mutable card borrow (so we can re-borrow self below).
                let new_power = card.current_power();
                let new_toughness = card.current_toughness();
                let is_mana_source_now = card.definition.cache.is_mana_source;

                // Log the typeline mutation as a reversible GameAction so a
                // rewind+replay restores the exact prior typeline AND removes the
                // keywords this animate granted (mtg-610). Logged when we changed
                // the typeline OR newly granted a keyword (Soulstone Sanctuary
                // changes both, but a hypothetical keyword-only animate must still
                // be reversible).
                if types_changed || !granted_keywords.is_empty() {
                    let prior_log_size = self.logger.log_count();
                    self.undo_log.log(
                        crate::undo::GameAction::AnimateTypeline {
                            card_id: *target,
                            prev_types,
                            prev_subtypes,
                            prev_temp_animate_types,
                            prev_temp_animate_subtypes,
                            prev_temp_removed_subtypes,
                            granted_keywords,
                        },
                        prior_log_size,
                    );
                }

                // If a permanent's typeline changed AND it's a mana source,
                // the per-player ManaSourceCache classification may now be
                // wrong. Mishra's Factory is the canonical case: it was a
                // colorless *simple* source before animate, but post-animate
                // a from-scratch scan re-classifies it as a *complex* source
                // (because creatures with mana abilities go to
                // `complex_sources`). Mark all mana caches dirty so the next
                // ManaEngine update rebuilds, and bump the mana-state version
                // so memoized engine state is invalidated. Mirrors what the
                // undo path already does.
                if types_changed && is_mana_source_now {
                    for (_, cache) in &mut self.mana_caches {
                        cache.mark_dirty();
                    }
                    self.increment_mana_version();
                }

                // Log the effect
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let kw_str = if keywords_granted.is_empty() {
                        String::new()
                    } else {
                        let kws: Vec<_> = keywords_granted.iter().map(|k| format!("{:?}", k)).collect();
                        format!(" and gains {}", kws.join(", "))
                    };

                    if power.is_some() || toughness.is_some() {
                        self.logger.gamelog(&format!(
                            "{} base P/T set to {}/{}{}",
                            card_name, new_power, new_toughness, kw_str
                        ));
                    } else if !keywords_granted.is_empty() {
                        self.logger.gamelog(&format!(
                            "{} gains {}",
                            card_name,
                            keywords_granted
                                .iter()
                                .map(|k| format!("{:?}", k))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }

                    // Surface "becomes a creature" so the gamelog reads
                    // sensibly when a manland animates.
                    if !types_added.is_empty() {
                        let type_names: Vec<_> = types_added.iter().map(|t| format!("{:?}", t)).collect();
                        self.logger
                            .gamelog(&format!("{} becomes {}", card_name, type_names.join(" + ")));
                    }
                }
            }
            Effect::SearchLibrary {
                player,
                card_type_filter,
                destination,
                enters_tapped,
                shuffle,
            } => {
                // Search library for a card matching the filter and move it to destination
                // MTG Rules 701.19a: To search a zone, a player looks at all cards in that zone

                // Get the library zone for the player
                let library_cards = self
                    .player_zones
                    .iter()
                    .find(|(id, _)| *id == *player)
                    .map(|(_, zones)| zones.library.cards.clone())
                    .ok_or_else(|| MtgError::InvalidAction(format!("Player {:?} has no library", player)))?;

                // Search for a card matching the filter
                // Filter format examples:
                // - "Land.Basic" = Land type + Basic subtype
                // - "Creature" = Any Creature
                // - "Plains,Island" = Land with Plains OR Island subtype (fetch lands)
                // - "Artifact.Equipment" = Artifact type + Equipment subtype
                let mut found_card = None;
                for &card_id in &library_cards {
                    if let Some(card) = self.cards.try_get(card_id) {
                        let card_matches = Self::card_matches_search_filter(card, card_type_filter);

                        if card_matches {
                            found_card = Some(card_id);
                            break;
                        }
                    }
                }

                // If we found a matching card, move it to the destination
                if let Some(card_id) = found_card {
                    // Move the card from library to destination
                    self.move_card(card_id, Zone::Library, *destination, *player)?;

                    // If destination is battlefield and enters_tapped is true, tap the card
                    if *destination == Zone::Battlefield && *enters_tapped {
                        // Use helper that handles tap + undo log + mana version
                        let _ = self.tap_permanent(card_id);
                    }
                }

                // Shuffle the library if required (MTG Rules 701.19b)
                if *shuffle {
                    self.shuffle_library(*player);
                }
            }
            Effect::AttachEquipment {
                source_equipment,
                target_creature,
            } => {
                // Attach Equipment to target creature
                // Skip if target is not on battlefield (fizzle)
                if !self.battlefield.contains(*target_creature) {
                    // Ability fizzles - target not on battlefield
                    return Ok(());
                }

                // Call the attach_equipment method
                self.attach_equipment(*source_equipment, *target_creature)?;
            }
            Effect::Balance {
                card_type: _,
                zone: _,
                sub_ability: _,
            } => {
                // Balance effect is handled interactively in the game loop
                // This is a no-op here - the game loop will detect Balance effects
                // and call resolve_balance_effect_interactive with controllers
                //
                // For non-interactive contexts (e.g., unit tests), call
                // execute_balance_effect() directly on GameState.
            }

            Effect::CreateToken {
                controller,
                token_script,
                amount,
                for_each_player,
            } => {
                // Create token(s) on the battlefield
                // MTG Rules 111.2: The player who creates a token is its owner and controller

                // Look up token definition from cache (loaded during game initialization)
                // For native builds, tokens are loaded from tokenscripts/ directory.
                // For WASM builds, tokens are bundled with deck data.
                let token_def = self.token_definitions.get(token_script).cloned();

                if let Some(token_def) = token_def {
                    // Determine which players get tokens
                    let player_ids: Vec<PlayerId> = if *for_each_player {
                        // Each player creates tokens (TokenOwner$ Player)
                        self.players.iter().map(|p| p.id).collect()
                    } else {
                        // Only the specified controller
                        vec![*controller]
                    };

                    for player_id in player_ids {
                        // Use actual token definition
                        for _ in 0..*amount {
                            let token_id = self.next_card_id();

                            // Shadow game dedup: in shadow games, tokens for opponent actions are
                            // pre-added via CardRevealed(TokenCreated) before this effect runs.
                            // CardRevealed uses insert_if_vacant (doesn't advance next_entity_id),
                            // so next_card_id() here returns the SAME id that was pre-added.
                            // We must skip to avoid the EntityStore write-once panic.
                            // For locally-created tokens (own actions in native shadow game),
                            // cards.contains() is false so we proceed normally.
                            if self.is_shadow_game && self.cards.contains(token_id) {
                                // Pre-added by CardRevealed; ensure it's on the battlefield too.
                                if !self.battlefield.contains(token_id) {
                                    self.battlefield.add(token_id);
                                }
                                continue;
                            }

                            // Instantiate token from definition
                            let mut token = token_def.instantiate(token_id, player_id);

                            // Mark as token and set controller
                            token.is_token = true;
                            token.controller = player_id;

                            // Add token to game
                            let token_name = token.name.to_string();
                            self.cards.insert(token_id, token);

                            // Log the entity mint so a rewind can remove the
                            // token AND roll `next_entity_id` back (mtg-ba6uq #3).
                            // next_card_id() already advanced next_entity_id and
                            // cards.insert added the instance — both unlogged
                            // until now, so a rewind leaked the token and replay
                            // minted a duplicate at a higher id. Logged BEFORE the
                            // reveal/battlefield placement so the LIFO undo
                            // reverses those first, then this clears the entity.
                            let mint_log_size = self.logger.log_count();
                            self.undo_log.log(
                                crate::undo::GameAction::CreateEntity { card_id: token_id },
                                mint_log_size,
                            );

                            // NETWORK: Reveal token to all players so server sends
                            // CardRevealed(TokenCreated). Without this, clients don't
                            // know the token's identity (causes desync).
                            let prior_log_size = self.logger.log_count();
                            self.maybe_reveal_to_all(token_id, prior_log_size);

                            // Put token onto the battlefield
                            self.battlefield.add(token_id);

                            // Debug log token creation
                            log::debug!(target: "token", "Created token {} (id={}) under player {}'s control",
                                token_name, token_id.as_u32(), player_id.as_u32());

                            // Log token creation (official game action)
                            self.logger.gamelog(&format!(
                                "Created {} under {}'s control",
                                token_name,
                                self.get_player(player_id)?.name
                            ));
                        }
                    }
                } else {
                    // Token definition not found - log warning and skip
                    // Some token scripts are missing from the forge-java cardsfolder
                    // (e.g., special tokens from newer sets not yet in our token library)
                    log::warn!(
                        "Token definition not found: '{}' - skipping token creation",
                        token_script
                    );
                }
            }

            Effect::Airbend { target } => {
                // Airbend effect: Exile target, grant owner permission to cast for {2}
                // CR 701.65b: Avatar set mechanic
                //
                // Implementation:
                // 1. Skip if target is still placeholder (0)
                // 2. Get the card's owner before exile
                // 3. Exile the target card
                // 4. Create a PersistentEffect (MayPlayFromExile) for the owner
                // 5. The effect is cleaned up when the card leaves exile or is cast

                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Get card info before exile
                let (owner, card_name) = {
                    let card = self.cards.get(*target)?;
                    (card.owner, card.name.clone())
                };

                // Move card from battlefield to exile
                self.move_card(*target, Zone::Battlefield, Zone::Exile, owner)?;

                // Create a PersistentEffect granting MayPlay from exile for {2}
                use crate::core::{CleanupCondition, ManaCost, PersistentEffectKind};

                let cleanup = CleanupCondition::Any(vec![
                    CleanupCondition::TrackedCardLeavesZone {
                        card: *target,
                        zone: Zone::Exile,
                    },
                    CleanupCondition::TrackedCardIsCast { card: *target },
                ]);

                self.persistent_effects.add(
                    PersistentEffectKind::MayPlayFromExile {
                        tracked_card: *target,
                        alternative_cost: ManaCost::from_string("2"), // {2} alternative cost
                        owner,
                    },
                    *target, // source_card - the airbended card itself is the source
                    owner,   // controller - the owner controls this permission
                    cleanup,
                );

                // Log the airbend
                self.logger.gamelog(&format!(
                    "{} is airbended (exiled, owner may cast for {{2}})",
                    card_name
                ));
            }

            Effect::Earthbend { target, num_counters } => {
                // Earthbend effect: Target land becomes 0/0 creature with haste, gets N +1/+1 counters
                // When it dies or is exiled, return it to battlefield tapped
                // CR 701.XX: Avatar set mechanic (custom)
                //
                // Implementation:
                // 1. Skip if target is still placeholder (0)
                // 2. Add Creature type to the land (it stays a land too)
                // 3. Set base power/toughness to 0/0 (temp, for animate effects)
                // 4. Add Haste keyword
                // 5. Put N +1/+1 counters
                // 6. Register delayed trigger for return-to-battlefield on death/exile

                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Validate the target is a land before earthbending.
                let card_name = {
                    let card = self.cards.get_mut(*target)?;

                    // Must be a land to earthbend
                    if !card.is_land() {
                        return Err(crate::MtgError::InvalidAction(
                            "Earthbend target must be a land".to_string(),
                        ));
                    }

                    card.name.clone()
                };

                // Add Creature type (still remains a land) + Haste via the logged
                // helper so the grant is reversible by the undo log (mtg-610: the
                // inline insert leaked Haste/Creature-type across rewind+replay,
                // making the turn-start keywords history-dependent).
                self.earthbend_animate_creature_haste_logged(*target);

                // Set temp base power/toughness to 0/0 (animate effect) via the
                // logged helper so the override is reversible by the undo log
                // (mtg-614 hole (c)).
                self.set_temp_base_stats_logged(*target, Some(0), Some(0));

                // Add +1/+1 counters
                use crate::core::CounterType;
                self.add_counters(*target, CounterType::P1P1, *num_counters)?;

                // Get controller for the delayed trigger
                let controller = self.turn.active_player;

                // Register delayed trigger: when this land dies or is exiled, return it to battlefield tapped
                use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerCondition};
                use smallvec::smallvec;

                let trigger = DelayedTrigger::new(
                    crate::core::DelayedTriggerId::new(0), // ID will be assigned by store
                    *target,                               // tracked_card
                    *target,                               // source_card (the land itself)
                    controller,
                    DelayedTriggerCondition::ZoneChange {
                        from_zones: smallvec![Zone::Battlefield],
                        to_zones: smallvec![Zone::Graveyard, Zone::Exile],
                    },
                    DelayedEffect::ReturnToBattlefield {
                        tapped: true,
                        to_owner: true,
                    },
                );

                let trigger_id = self.delayed_triggers.add(trigger);

                // Log the earthbend
                self.logger.gamelog(&format!(
                    "{} is earthbent! (0/0 creature with haste, {} +1/+1 counters, returns when dies/exiled)",
                    card_name, num_counters
                ));

                // Log trigger creation for debugging
                self.logger.gamelog(&format!(
                    "  -> Delayed trigger {} registered: return {} to battlefield tapped when it leaves",
                    trigger_id.as_u32(),
                    card_name
                ));
            }

            Effect::Firebend { controller, amount } => {
                // Firebend effect: Add N red mana to controller's combat mana pool
                // This mana lasts until end of combat (cleared in end_combat_step)
                // CR 701.XX: Avatar set mechanic (custom)

                // Get player name before mutable borrow for logging
                let player_name = self
                    .get_player(*controller)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|_| "Unknown".into());

                // Snapshot the combat mana pool for undo BEFORE adding (mtg-ba6uq
                // #7).
                self.log_combat_mana_pool(*controller);
                // Add red mana to combat mana pool (lazy initialization)
                let player = self.get_player_mut(*controller)?;
                for _ in 0..*amount {
                    player.add_combat_mana(crate::core::Color::Red);
                }

                // Log the firebend
                self.logger.gamelog(&format!(
                    "{} adds {} {{R}} (combat mana, lasts until end of combat)",
                    player_name, amount
                ));
            }

            Effect::GrantCantBeBlocked { target } => {
                // GrantCantBeBlocked effect: Target creature can't be blocked this turn
                // Created by AB$ Effect abilities with StaticAbilities$ containing "unblock"
                //
                // Implementation:
                // 1. Skip if target is still placeholder (0)
                // 2. Create a PersistentEffect (CantBeBlocked) for the target
                // 3. The effect is cleaned up at end of turn

                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Get card name for logging
                let card_name = self.cards.get(*target).map(|c| c.name.as_str()).unwrap_or("Unknown");

                // Get the effect controller (the player who activated the ability)
                let controller = self.turn.active_player;

                // Create a PersistentEffect granting "can't be blocked"
                use crate::core::{CleanupCondition, PersistentEffectKind};

                self.persistent_effects.add(
                    PersistentEffectKind::CantBeBlocked { creature: *target },
                    *target,    // source_card - the targeted creature
                    controller, // controller - the active player
                    CleanupCondition::EndOfTurn,
                );

                // Log the effect
                self.logger
                    .gamelog(&format!("{} can't be blocked this turn", card_name));
            }

            Effect::Regenerate { target } => {
                // Regenerate: Add a regeneration shield to target permanent (CR 701.15a)
                // "The next time [permanent] would be destroyed this turn, instead
                // remove all damage marked on it, tap it, and remove it from combat."
                if target.is_placeholder() {
                    return Ok(());
                }
                if !self.battlefield.contains(*target) {
                    return Ok(());
                }
                let card = self.cards.get_mut(*target)?;
                card.regeneration_shields = card.regeneration_shields.saturating_add(1);
                let card_name = self.cards.get(*target).map(|c| c.name.as_str()).unwrap_or("Unknown");
                self.logger
                    .gamelog(&format!("{} ({}) gains a regeneration shield", card_name, target));
            }

            Effect::PreventDamage { target, amount } => self.execute_prevent_damage(target, *amount)?,

            Effect::PreventDamageFromSource {
                protected,
                color,
                source,
            } => self.execute_prevent_damage_from_source(*protected, *color, *source)?,

            Effect::PreventAllCombatDamageThisTurn { target } => {
                self.execute_prevent_all_combat_damage_this_turn(*target)?
            }

            Effect::DestroyAll {
                restriction,
                no_regenerate,
            } => {
                // Destroy all permanents matching the restriction (e.g., Wrath of God)
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter(|&card_id| {
                        self.cards
                            .get(card_id)
                            .map(|card| restriction.matches(card))
                            .unwrap_or(false)
                    })
                    .collect();

                for card_id in targets {
                    let (owner, has_indestructible, has_regen_shield) = {
                        let card = self.cards.get(card_id)?;
                        (card.owner, card.has_indestructible(), card.regeneration_shields > 0)
                    };
                    if has_indestructible {
                        // Indestructible - can't be destroyed
                    } else if has_regen_shield && !no_regenerate {
                        // CR 701.15a: Regeneration replaces destruction
                        self.apply_regeneration_shield(card_id)?;
                    } else {
                        let _ = self.check_death_triggers(card_id);
                        let card_name = self
                            .cards
                            .get(card_id)
                            .map(|c| c.name.to_string())
                            .unwrap_or_else(|_| "Unknown".to_string());
                        self.move_card(
                            card_id,
                            Zone::Battlefield,
                            self.death_destination_for_card(card_id),
                            owner,
                        )?;
                        self.logger
                            .gamelog(&format!("{} ({}) is destroyed", card_name, card_id));
                    }
                }
            }

            Effect::SacrificeAll { restriction } => {
                // Each player sacrifices all permanents matching the restriction (e.g., All is Dust)
                // Sacrifice bypasses indestructible and regeneration (CR 701.17)
                let targets: Vec<(CardId, PlayerId)> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter_map(|card_id| {
                        let card = self.cards.try_get(card_id)?;
                        if restriction.matches(card) {
                            Some((card_id, card.owner))
                        } else {
                            None
                        }
                    })
                    .collect();

                for (card_id, owner) in targets {
                    let _ = self.check_death_triggers(card_id);
                    let card_name = self
                        .cards
                        .try_get(card_id)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    self.move_card(
                        card_id,
                        Zone::Battlefield,
                        self.death_destination_for_card(card_id),
                        owner,
                    )?;
                    self.logger
                        .gamelog(&format!("{} ({}) is sacrificed", card_name, card_id));
                }
            }

            Effect::DamageAll {
                amount,
                valid_cards,
                damage_players,
            } => self.execute_damage_all(*amount, valid_cards, *damage_players)?,

            Effect::ForceSacrifice {
                player,
                sac_type,
                count,
            } => {
                // Force a player to sacrifice permanents matching a type
                // CR 701.17: "sacrifice a permanent" means its controller moves it to graveyard
                let player_name = self
                    .get_player(*player)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|_| "Unknown".to_string().into());

                // Find matching permanents controlled by the target player
                let mut candidates: Vec<(CardId, i32)> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter_map(|card_id| {
                        let card = self.cards.get(card_id).ok()?;
                        if card.controller != *player {
                            return None;
                        }
                        // Match sac_type against card types
                        let type_matches = match sac_type.as_str() {
                            "Creature" => card.is_creature(),
                            "Land" => card.is_land(),
                            "Artifact" => card.is_artifact(),
                            "Enchantment" => card.is_enchantment(),
                            "Permanent" | "" => true, // Any permanent
                            _ => {
                                // Try matching as creature subtype or more complex filter
                                card.is_creature() // Default to creature
                            }
                        };
                        if type_matches {
                            // Score: lower value = sacrifice first
                            // Use P/T sum for creatures, CMC for non-creatures
                            let value = if card.is_creature() {
                                i32::from(card.current_power()) + i32::from(card.current_toughness())
                            } else {
                                i32::from(card.mana_cost.cmc())
                            };
                            Some((card_id, value))
                        } else {
                            None
                        }
                    })
                    .collect();

                // Sort by value ascending (sacrifice least valuable first)
                candidates.sort_by_key(|&(_, v)| v);

                let to_sac = (*count as usize).min(candidates.len());
                for &(card_id, _) in candidates.iter().take(to_sac) {
                    let card_name = self.cards.get(card_id).map(|c| c.name.to_string()).unwrap_or_default();
                    let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(*player);
                    let dest = self.death_destination_for_card(card_id);
                    self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                    self.logger
                        .gamelog(&format!("{} sacrifices {} ({})", player_name, card_name, card_id));
                }

                if to_sac == 0 {
                    self.logger
                        .gamelog(&format!("{} has no {} to sacrifice", player_name, sac_type));
                }
            }

            Effect::TapAll { restriction } => {
                // Tap all permanents matching the restriction
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter(|&card_id| {
                        self.cards
                            .get(card_id)
                            .map(|card| !card.tapped && restriction.matches(card))
                            .unwrap_or(false)
                    })
                    .collect();

                for card_id in targets {
                    // Route through tap_permanent so the undo log, ManaSourceCache
                    // untapped counts, and mana_state_version all stay consistent.
                    // Setting `card.tapped` directly would leave the mana cache
                    // reporting these sources as still untapped, which can offer an
                    // unaffordable spell as a legal play and diverge server vs client
                    // shadow state (network desync). See docs/NETWORK_ARCHITECTURE.md.
                    let card_name = self.cards.get(card_id)?.name.clone();
                    self.tap_permanent(card_id)?;
                    self.logger.gamelog(&format!("{} ({}) is tapped", card_name, card_id));
                }
            }

            Effect::UntapAll { restriction } => {
                // Untap all permanents matching the restriction
                let spell_controller = self.turn.active_player;
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter(|&card_id| {
                        self.cards
                            .get(card_id)
                            .map(|card| {
                                card.tapped
                                    && restriction.matches_with_controller(card, spell_controller, card.controller)
                            })
                            .unwrap_or(false)
                    })
                    .collect();

                for card_id in targets {
                    // Route through untap_permanent so the undo log, ManaSourceCache
                    // untapped counts, and mana_state_version stay consistent (see
                    // the matching note in Effect::TapAll above).
                    let card_name = self.cards.get(card_id)?.name.clone();
                    self.untap_permanent(card_id)?;
                    self.logger.gamelog(&format!("{} ({}) is untapped", card_name, card_id));
                }
            }

            Effect::SetLife { player, amount } => self.execute_set_life(*player, *amount)?,

            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } => {
                // CreateDelayedTrigger effect: Register a delayed trigger that fires on a condition
                // Created by SP$/DB$ DelayedTrigger spells.
                //
                // Two shapes:
                // - ZoneChange (Fatal Fissure): tracks a battlefield card; the
                //   trigger fires when that card changes zones. Requires a valid
                //   tracked card on the battlefield or the spell fizzles.
                // - Phase / SpellCast (Mana Drain, Jeong Jeong): no tracked
                //   battlefield card; the trigger fires at a future phase / on a
                //   future spell cast. The controller is the resolving spell's
                //   controller, regardless of whose turn it is.
                let is_zone_change = matches!(condition, crate::core::DelayedTriggerCondition::ZoneChange { .. });

                if is_zone_change {
                    // Skip if tracked_card is still placeholder (0) - no valid targets found
                    if tracked_card.is_placeholder() {
                        log::debug!(target: "actions", "CreateDelayedTrigger: tracked_card is placeholder, spell fizzles");
                        return Ok(());
                    }
                    // Verify the target is still on battlefield
                    if !self.battlefield.contains(*tracked_card) {
                        log::debug!(target: "actions", "CreateDelayedTrigger: target no longer on battlefield, spell fizzles");
                        return Ok(());
                    }
                }

                // Get card name for logging (tracked card for zone triggers, else
                // the resolving spell's name).
                let card_name = self
                    .cards
                    .try_get(*tracked_card)
                    .or_else(|| self.current_damage_source.and_then(|src| self.cards.try_get(src)))
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|| "Unknown".to_string());

                // The trigger controller is the resolving spell's controller
                // (current_damage_source is set to the resolving spell during
                // resolve_spell_execute_effects). Fall back to the active player
                // for the legacy zone-change path / direct execute_effect calls.
                let controller = self
                    .current_damage_source
                    .and_then(|src| self.cards.try_get(src))
                    .map(|c| c.controller)
                    .unwrap_or(self.turn.active_player);

                // Create the delayed trigger
                use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerId};

                // Check if the inner effect is CopySpellAbility - needs special handling
                // Wildcard is appropriate: all non-CopySpellAbility effects wrap in ExecuteEffect
                #[allow(clippy::wildcard_enum_match_arm)]
                let delayed_effect_type = match **delayed_effect {
                    Effect::CopySpellAbility { may_choose_targets, .. } => {
                        // For CopySpellAbility, use the specialized DelayedEffect variant
                        // tracked_card will be repurposed to hold the spell being copied
                        // (set at trigger fire time, not creation time)
                        DelayedEffect::CopySpellAbility { may_choose_targets }
                    }
                    Effect::DestroyPermanent { .. } => {
                        // Berserk: "At the beginning of the next end step, destroy
                        // that creature if it attacked this turn." The delayed
                        // Destroy targets the TRACKED card (the creature Berserk
                        // targeted via RememberObjects$ Targeted), gated on its
                        // attacked-this-turn flag (CR 603.4). Route to the
                        // dedicated DestroyTracked variant so fire_delayed_trigger
                        // can both bind the tracked card and check the gate.
                        DelayedEffect::DestroyTracked {
                            require_attacked_this_turn: true,
                        }
                    }
                    _ => {
                        // For all other effects, wrap in ExecuteEffect
                        DelayedEffect::ExecuteEffect {
                            effect: delayed_effect.clone(),
                        }
                    }
                };

                let trigger = DelayedTrigger::new(
                    DelayedTriggerId::new(0), // ID will be assigned by store
                    *tracked_card, // tracked_card - for zone triggers: the creature to watch; for spell triggers: will be set at fire time
                    *tracked_card, // source_card - same as tracked for spell-created triggers
                    controller,
                    condition.clone(),
                    delayed_effect_type,
                );

                // Apply expiry if specified
                let trigger = match expiry {
                    Some(exp) => trigger.with_expiry(exp.clone()),
                    None => trigger,
                };

                // Capture any remembered numeric value (Mana Drain's countered
                // mana value) onto the trigger so it survives the chained
                // ClearRemembered and is part of the trigger's serialized state.
                let trigger = trigger.with_remembered_amount(self.remembered_amount);

                let prior_log_size = self.logger.log_count();
                let trigger_id = self.delayed_triggers.add(trigger);
                // Undo-log the registration so rewind-to-turn-start (snapshot/
                // resume, undo search) removes it; otherwise the replay would
                // double-register the trigger (mtg-519).
                self.undo_log.log(
                    crate::undo::GameAction::RegisterDelayedTrigger { id: trigger_id },
                    prior_log_size,
                );

                // Log the delayed trigger creation
                let what = if is_zone_change {
                    format!("watching {} for death", card_name)
                } else {
                    format!("from {}", card_name)
                };
                self.logger
                    .gamelog(&format!("Delayed trigger {} created: {}", trigger_id.as_u32(), what));

                log::debug!(
                    target: "actions",
                    "CreateDelayedTrigger: trigger {} for {} with effect {:?}",
                    trigger_id.as_u32(), card_name, delayed_effect
                );
            }

            Effect::ModalChoice { modes, .. } => {
                // Modal spells are handled during casting, not execution.
                // When the spell resolves, only the selected mode's effect is executed.
                // This variant should not be encountered during execute_effect.
                //
                // If we get here, it means the modal choice wasn't processed during casting.
                // Log a warning and skip execution.
                log::warn!(
                    target: "actions",
                    "ModalChoice effect reached execute_effect - should have been resolved during casting. {} modes available.",
                    modes.len()
                );
            }

            Effect::CopyPermanent {
                target,
                controller,
                non_legendary: _, // TODO(mtg-210): Implement legendary rule removal when legendary is tracked
                set_power,
                set_toughness,
                ref add_types,
                num_copies,
                restriction: _, // Used at targeting time, not execution time
            } => {
                // Create token copies of the target permanent
                // MTG Rules 707.2: A copy of a permanent has the same characteristics
                // as the original, except for any modifications specified

                // Verify target is still on battlefield
                if !self.battlefield.contains(*target) {
                    // Target was removed - spell fizzles
                    log::debug!(target: "actions", "CopyPermanent target no longer on battlefield");
                    return Ok(());
                }

                let original = self.cards.get(*target)?;
                let original_name = original.name.clone();
                let original_base_power = original.base_power();
                let original_base_toughness = original.base_toughness();

                for _ in 0..*num_copies {
                    let token_id = self.next_card_id();

                    // Clone the original card to get all characteristics
                    let original = self.cards.get(*target)?;
                    let mut token = original.clone();

                    // Update identity for the new token
                    token.id = token_id;
                    token.owner = *controller;
                    token.controller = *controller;

                    // Reset state for new permanent
                    token.tapped = false;
                    token.turn_entered_battlefield = None; // Will be set when it enters battlefield
                    token.counters.clear();
                    token.damage = 0;
                    token.attached_to = None;

                    // Apply modifications

                    // SetPower$ N - override power
                    if let Some(power) = set_power {
                        // Power is i8 in Card but i32 in Effect, clamp to i8 range
                        token.set_base_power(Some(*power as i8));
                    }

                    // SetToughness$ N - override toughness
                    if let Some(toughness) = set_toughness {
                        token.set_base_toughness(Some(*toughness as i8));
                    }

                    // AddTypes$ Type1 & Type2 - add creature types (subtypes)
                    for type_str in add_types {
                        let subtype = crate::core::Subtype::from(type_str.as_str());
                        if !token.subtypes.contains(&subtype) {
                            token.subtypes.push(subtype);
                        }
                    }

                    // Add token to game
                    let token_name = token.name.to_string();
                    self.cards.insert(token_id, token);

                    // NETWORK: Reveal token copy to all players so server sends
                    // CardRevealed(TokenCreated). Without this, clients don't
                    // know the token's identity (causes desync).
                    let prior_log_size = self.logger.log_count();
                    self.maybe_reveal_to_all(token_id, prior_log_size);

                    // Put token onto battlefield
                    self.battlefield.add(token_id);

                    // Log token creation
                    let modification_desc = if set_power.is_some() || set_toughness.is_some() || !add_types.is_empty() {
                        let p = set_power.map(|x| x as i8).or(original_base_power).unwrap_or(0);
                        let t = set_toughness.map(|x| x as i8).or(original_base_toughness).unwrap_or(0);
                        let types_str = if add_types.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", add_types.join(" "))
                        };
                        format!(" (as {}/{}{} copy)", p, t, types_str)
                    } else {
                        String::new()
                    };

                    log::debug!(
                        target: "token",
                        "Created token copy of {} (id={}) under player {}'s control{}",
                        original_name, token_id.as_u32(), controller.as_u32(), modification_desc
                    );

                    self.logger.gamelog(&format!(
                        "Created a token copy of {}{} under {}'s control",
                        token_name,
                        modification_desc,
                        self.get_player(*controller)?.name
                    ));
                }
            }

            Effect::Dig {
                dig_count,
                change_count,
                change_all,
                destination,
                rest_destination,
                may_play,
                may_play_without_mana_cost,
                target_self,
                optional,
                rest_random,
                reveal,
                change_valid,
            } => {
                // Dig effect: Look at top N cards of a library and move some to destination
                //
                // Two patterns:
                // 1. target_self=true (Impulse, Wrenn and Seven): Look at top N of your library,
                //    select up to change_count matching ChangeValid$ to destination, rest elsewhere
                // 2. target_self=false (Fire Lord Ozai): Exile top N from each opponent's library
                //
                // AI heuristic for card selection: pick highest-value cards that match the filter,
                // preferring creatures by P/T+CMC, then non-creatures by CMC.

                let digger = self.turn.active_player;
                let mut moved_cards: Vec<CardId> = Vec::with_capacity(*dig_count as usize);

                if *target_self {
                    // Self-dig: look at top N cards of YOUR library
                    let library = self
                        .player_zones
                        .iter()
                        .find(|(id, _)| *id == digger)
                        .map(|(_, zones)| &zones.library);

                    if let Some(library) = library {
                        let take_count = *dig_count as usize;
                        // Library top is at the end of the Vec, so use .rev()
                        let card_ids: smallvec::SmallVec<[CardId; 8]> =
                            library.cards.iter().rev().take(take_count).copied().collect();

                        let digger_name = self.get_player(digger)?.name.to_string();

                        if !card_ids.is_empty() {
                            let verb = if *reveal { "reveals" } else { "looks at" };
                            self.logger.gamelog(&format!(
                                "{} {} the top {} card{} of their library",
                                digger_name,
                                verb,
                                card_ids.len(),
                                if card_ids.len() == 1 { "" } else { "s" }
                            ));
                        }

                        // Separate cards into valid (matchable) and invalid (rest)
                        // If change_valid is empty, all cards are valid
                        let has_filter = !change_valid.is_empty();
                        let mut valid_ids: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();
                        let mut invalid_ids: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();

                        for &card_id in &card_ids {
                            if has_filter {
                                let matches = self
                                    .cards
                                    .try_get(card_id)
                                    .is_some_and(|card| change_valid.iter().any(|f| f.matches(card)));
                                if matches {
                                    valid_ids.push(card_id);
                                } else {
                                    invalid_ids.push(card_id);
                                }
                            } else {
                                valid_ids.push(card_id);
                            }
                        }

                        // Determine how many cards to select
                        let max_select = if *change_all {
                            valid_ids.len()
                        } else {
                            (*change_count as usize).min(valid_ids.len())
                        };

                        // AI heuristic: rank valid cards by value, pick best ones
                        // Score: creatures by (power+toughness)*10 + cmc*5 + 80,
                        //        lands by 100, others by 50 + cmc*30
                        if valid_ids.len() > 1 && max_select < valid_ids.len() {
                            valid_ids.sort_by(|&a, &b| {
                                let score_a = self.dig_card_score(a);
                                let score_b = self.dig_card_score(b);
                                score_b.cmp(&score_a) // Descending: best first
                            });
                        }

                        // If optional and no good cards, AI may choose to skip
                        let select_count = if *optional && max_select > 0 {
                            // Simple heuristic: skip only if best card scores very low
                            let best_score = valid_ids.first().map(|&id| self.dig_card_score(id)).unwrap_or(0);
                            if best_score < 30 {
                                0
                            } else {
                                max_select
                            }
                        } else {
                            max_select
                        };

                        // Move selected cards to destination
                        let selected: smallvec::SmallVec<[CardId; 8]> =
                            valid_ids.iter().take(select_count).copied().collect();
                        let rest_from_valid: smallvec::SmallVec<[CardId; 8]> =
                            valid_ids.iter().skip(select_count).copied().collect();

                        for &card_id in &selected {
                            let card_name = self
                                .cards
                                .get(card_id)
                                .map(|c| c.name.to_string())
                                .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));

                            self.move_card(card_id, Zone::Library, *destination, digger)?;

                            // mtg-212: the DISPLAYED name depends on async reveal
                            // timing on a network shadow (the dug card's public
                            // `RevealCard` may not have arrived on the shadow's
                            // first forward pass — `card_name` falls back to
                            // `card#<id>` — but is present on a rewind replay).
                            // Supply the rewind/replay verifier a reveal-timing-
                            // INDEPENDENT id form so the presentation asymmetry is
                            // not flagged as a fatal desync (the card is in the
                            // destination zone either way — the turn-start hash
                            // proves the STATE). Same mechanism as the
                            // discard-into-graveyard line (mtg-677).
                            let action = if *reveal { "reveals and puts" } else { "puts" };
                            let stable = format!(
                                "{} {} card#{} into {:?}",
                                digger_name,
                                action,
                                card_id.as_u32(),
                                destination
                            );
                            self.logger.gamelog_reveal_stable(
                                &format!("{} {} {} into {:?}", digger_name, action, card_name, destination),
                                &stable,
                            );
                            moved_cards.push(card_id);
                        }

                        // Handle rest: non-selected valid cards + invalid cards
                        let mut rest_cards: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();
                        rest_cards.extend(rest_from_valid.iter().copied());
                        rest_cards.extend(invalid_ids.iter().copied());

                        if !rest_cards.is_empty() {
                            // Shuffle rest if RestRandomOrder$ True
                            if *rest_random {
                                // Use a simple deterministic shuffle based on game state
                                // (card IDs provide enough entropy for reasonable shuffling)
                                let len = rest_cards.len();
                                for i in (1..len).rev() {
                                    let j = (rest_cards[i].as_u32() as usize + i) % (i + 1);
                                    rest_cards.swap(i, j);
                                }
                            }

                            // Move rest to rest_destination
                            if *rest_destination == Zone::Library {
                                // Capture pre-reorder library order so a rewind
                                // can restore it (mtg-ba6uq #2): the raw
                                // remove/add_to_bottom below is not otherwise
                                // undo-logged.
                                self.log_library_reorder(digger, false);
                                // Put on bottom of library: remove from current position,
                                // then insert at index 0 (bottom)
                                if let Some(zones) = self.get_player_zones_mut(digger) {
                                    for &card_id in &rest_cards {
                                        zones.library.remove(card_id);
                                        zones.library.add_to_bottom(card_id);
                                    }
                                }
                                let rest_count = rest_cards.len();
                                self.logger.gamelog(&format!(
                                    "{} puts {} card{} on the bottom of their library",
                                    digger_name,
                                    rest_count,
                                    if rest_count == 1 { "" } else { "s" }
                                ));
                            } else {
                                for &card_id in &rest_cards {
                                    let card_name = self
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.to_string())
                                        .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));

                                    self.move_card(card_id, Zone::Library, *rest_destination, digger)?;

                                    let dest_name = match rest_destination {
                                        Zone::Graveyard => "their graveyard",
                                        Zone::Exile => "exile",
                                        Zone::Hand => "their hand",
                                        Zone::Library | Zone::Battlefield | Zone::Stack | Zone::Command => {
                                            "another zone"
                                        }
                                    };
                                    // mtg-212: reveal-timing-independent verifier
                                    // key (see the selected-cards branch above).
                                    let stable =
                                        format!("{} puts card#{} into {}", digger_name, card_id.as_u32(), dest_name);
                                    self.logger.gamelog_reveal_stable(
                                        &format!("{} puts {} into {}", digger_name, card_name, dest_name),
                                        &stable,
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Opponent-dig pattern (Fire Lord Ozai, Xander's Pact)
                    let opponent_ids: smallvec::SmallVec<[PlayerId; 4]> =
                        self.players.iter().filter(|p| p.id != digger).map(|p| p.id).collect();

                    for opponent_id in opponent_ids {
                        let library = self
                            .player_zones
                            .iter()
                            .find(|(id, _)| *id == opponent_id)
                            .map(|(_, zones)| &zones.library);

                        if let Some(library) = library {
                            let take_count = *dig_count as usize;
                            // Library top is at end of Vec, so use .rev()
                            let card_ids: smallvec::SmallVec<[CardId; 4]> =
                                library.cards.iter().rev().take(take_count).copied().collect();

                            for card_id in card_ids {
                                let opponent_name = self.get_player(opponent_id)?.name.to_string();
                                let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("a card");

                                self.logger
                                    .gamelog(&format!("{} exiled from {}'s library", card_name, opponent_name));

                                self.move_card(card_id, Zone::Library, *destination, opponent_id)?;
                                moved_cards.push(card_id);
                            }
                        }
                    }
                }

                // If may_play is true, create persistent effect to allow playing exiled cards
                if *may_play && !moved_cards.is_empty() {
                    let mana_cost_text = if *may_play_without_mana_cost {
                        " without paying its mana cost"
                    } else {
                        ""
                    };

                    self.logger.gamelog(&format!(
                        "Until end of turn, you may play one of those cards{}",
                        mana_cost_text
                    ));

                    use crate::core::{CleanupCondition, PersistentEffectKind};

                    if *may_play_without_mana_cost {
                        let source_card = moved_cards[0];
                        let num_moved = moved_cards.len();

                        self.persistent_effects.add(
                            PersistentEffectKind::MayPlayOneWithoutManaCost {
                                tracked_cards: std::mem::take(&mut moved_cards),
                                beneficiary: digger,
                            },
                            source_card,
                            digger,
                            CleanupCondition::EndOfTurn,
                        );

                        log::debug!(
                            target: "dig",
                            "Created MayPlayOneWithoutManaCost effect for {} cards, beneficiary: player {}",
                            num_moved,
                            digger.as_u32()
                        );
                    }
                }
            }

            Effect::CopySpellAbility {
                may_choose_targets,
                defined_source,
                controller,
            } => {
                // CopySpellAbility is used in two contexts:
                // 1. Inside a delayed trigger (Jeong Jeong) — handled by
                //    DelayedEffect::CopySpellAbility, NOT here.
                // 2. As a SubAbility of a resolving spell (Chain Lightning:
                //    "...may pay {R}{R}. If the player does, they may copy this
                //    spell and may choose a new target for that copy").
                //
                // The Parent path copies the currently-resolving spell. By the
                // time this SubAbility runs, the source spell is still on the
                // stack (it leaves in resolve_spell_finalize, AFTER all its
                // effects) and is recorded in `current_damage_source` (set for
                // every resolving spell in resolve_spell_execute_effects). We
                // reuse the shared `copy_spell_onto_stack` helper (CR 707.10).
                use crate::core::effects::CopySpellSource;
                match defined_source {
                    CopySpellSource::TargetedSpell => {
                        // "Copy a separately-TARGETED spell/ability" (Twincast,
                        // Reverberate, Fork, Return the Favor, ...): cloning an
                        // arbitrary targeted stack object is NOT yet implemented,
                        // so this is a SAFE NO-OP. Critically it must NOT fall
                        // through to the Parent self-copy below — that would copy
                        // the card itself forever (the commander-format infinite
                        // loop). env_logger only; no player-facing gamelog.
                        log::info!(
                            target: "copy_spell",
                            "CopySpellAbility(TargetedSpell): copy-target-spell not implemented — no-op \
                             (may_choose_targets={})",
                            may_choose_targets
                        );
                    }
                    CopySpellSource::Parent => {
                        // The copy's controller — resolved to a concrete numeric
                        // PlayerId in resolve_effect_target (UnlessCostWrapper arm)
                        // before we get here. Falls back to the resolving spell's
                        // owner if unresolved.
                        let original_id = match self.current_damage_source {
                            Some(id) => id,
                            None => {
                                log::warn!(
                                    target: "copy_spell",
                                    "CopySpellAbility(Parent): no resolving spell tracked; cannot copy"
                                );
                                return Ok(());
                            }
                        };
                        let original_owner = self
                            .cards
                            .get(original_id)
                            .map(|c| c.owner)
                            .unwrap_or_else(|_| PlayerId::new(0));
                        let copy_controller = controller
                            .as_deref()
                            .and_then(|s| s.parse::<u32>().ok())
                            .map(PlayerId::new)
                            .unwrap_or(original_owner);

                        // MayChooseTarget$ True ("may choose a new target for that
                        // copy"): the controller of the copy may retarget. The
                        // canonical Chain Lightning play — and the only meaningful
                        // retarget for a single-target burn copied by the OTHER
                        // player — is to aim the copy back at the original caster
                        // (a player target). This is a deterministic, information-
                        // independent choice (uses only public stack/controller
                        // facts), so it is identical on server and client. When
                        // retargeting is not allowed the copy keeps the original
                        // targets (CR 707.10a).
                        let new_targets = if *may_choose_targets && copy_controller != original_owner {
                            Some(smallvec::smallvec![crate::core::player_as_target_sentinel(
                                original_owner
                            )])
                        } else {
                            None
                        };
                        self.copy_spell_onto_stack(original_id, copy_controller, new_targets)?;
                    }
                    CopySpellSource::TriggeredSpellAbility => {
                        // This case should go through DelayedEffect, but log if we get here
                        log::debug!(
                            target: "actions",
                            "CopySpellAbility: TriggeredSpellAbility reached execute_effect \
                             (should use delayed trigger path). may_choose_targets={}",
                            may_choose_targets
                        );
                    }
                }
            }
            Effect::ImmediateTrigger { condition, sub_effects } => {
                // Check if remembered cards match the condition
                let condition_met = match condition {
                    crate::core::ImmediateTriggerCondition::RememberedNonLand => {
                        // Check if any remembered card is a nonland
                        self.remembered_cards.iter().any(|&card_id| {
                            if let Some(card) = self.cards.try_get(card_id) {
                                !card.is_land()
                            } else {
                                false
                            }
                        })
                    }
                    crate::core::ImmediateTriggerCondition::AnyRemembered => !self.remembered_cards.is_empty(),
                };

                if condition_met {
                    // Execute sub-effects
                    for sub_effect in sub_effects {
                        self.execute_effect(sub_effect)?;
                    }
                }
            }
            Effect::ClearRemembered => {
                // Clear remembered cards, players, and numeric value storage.
                // The numeric value (Mana Drain) is already captured onto the
                // delayed trigger by the preceding CreateDelayedTrigger, so
                // clearing the scratch here is safe.
                self.remembered_cards.clear();
                self.remembered_players.clear();
                if self.remembered_amount.is_some() {
                    let prior_log_size = self.logger.log_count();
                    let previous = self.remembered_amount;
                    self.remembered_amount = None;
                    self.undo_log.log(
                        crate::undo::GameAction::SetRememberedAmount { previous },
                        prior_log_size,
                    );
                }
            }
            Effect::UnlessCostWrapper {
                inner_effect,
                unless_cost,
            } => {
                use crate::core::effects::UnlessCostType;

                // Parse the resolved payer ID (stored as numeric string after resolve_effect_target)
                let payer_id = unless_cost
                    .payer
                    .parse::<u32>()
                    .map(PlayerId::new)
                    .unwrap_or_else(|_| PlayerId::new(0));

                // Check if the cost can be paid
                let can_pay = match &unless_cost.cost {
                    UnlessCostType::Discard { count, card_type: _ } => {
                        // Check if player has enough cards in hand
                        self.player_zones
                            .iter()
                            .find(|(pid, _)| *pid == payer_id)
                            .map(|(_, zones)| zones.hand.cards.len() >= *count as usize)
                            .unwrap_or(false)
                    }
                    UnlessCostType::Sacrifice { count, valid_type } => {
                        // Check if player controls enough permanents of the type
                        self.can_pay_sacrifice_pattern(valid_type, *count, CardId::new(0), payer_id)
                    }
                    UnlessCostType::PayLife(amount) => {
                        // Check if player has enough life
                        self.get_player(payer_id)
                            .map(|p| p.life > i32::from(*amount))
                            .unwrap_or(false)
                    }
                    UnlessCostType::Mana(mana_cost) => {
                        // Check the payer can actually produce this cost, honouring
                        // COLORED requirements (Chain Lightning's `UnlessCost$ R R`
                        // is {R}{R}, generic=0 — the old "generic <= untapped lands"
                        // check ignored color and treated 0 generic as trivially
                        // payable). Reuse the ManaEngine affordability resolver,
                        // built read-only from the current state (it reads the
                        // per-player mana cache, falling back to a battlefield scan
                        // when the cache is unbuilt). Pool mana (floating rituals)
                        // is included via can_pay_with_pool. This is a pure read —
                        // no borrow conflict with `&mut self`.
                        let pool = self.try_get_player(payer_id).map(|p| p.mana_pool).unwrap_or_default();
                        let mut mana_engine = crate::game::mana_engine::ManaEngine::new();
                        mana_engine.update(self, payer_id);
                        mana_engine.can_pay_with_pool(mana_cost, &pool)
                    }
                    UnlessCostType::Reveal { count, card_type: _ } => {
                        // Check if player has enough cards in hand to reveal
                        self.player_zones
                            .iter()
                            .find(|(pid, _)| *pid == payer_id)
                            .map(|(_, zones)| zones.hand.cards.len() >= *count as usize)
                            .unwrap_or(false)
                    }
                };

                // AI heuristic: decide whether to pay
                // - For switched costs (pay → effect): AI pays if the effect benefits them
                // - For non-switched costs (effect if NOT paid): AI pays to prevent opponent's effect
                let should_pay = if can_pay {
                    // Simple heuristic based on who benefits
                    if unless_cost.switched {
                        // "You may discard to draw" - controller benefits from effect
                        // AI always takes beneficial effects
                        true
                    } else {
                        // "Counter unless you pay" - opponent pays to prevent our spell
                        // AI always tries to prevent the effect
                        true
                    }
                } else {
                    false
                };

                // Execute payment if decided to pay
                let paid = if should_pay {
                    match &unless_cost.cost {
                        UnlessCostType::Discard { count, card_type: _ } => {
                            // Discard cards from hand (simple: discard from back)
                            let mut discarded = 0u8;
                            for _ in 0..*count {
                                // Get a card from hand to discard
                                let card_to_discard = self
                                    .player_zones
                                    .iter()
                                    .find(|(pid, _)| *pid == payer_id)
                                    .and_then(|(_, zones)| zones.hand.cards.last().copied());

                                if let Some(card_id) = card_to_discard {
                                    // Move card to graveyard
                                    let _ = self.move_card(card_id, Zone::Hand, Zone::Graveyard, payer_id);
                                    discarded += 1;
                                }
                            }
                            discarded == *count
                        }
                        UnlessCostType::PayLife(amount) => {
                            // Pay life
                            if let Some(player) = self.players.iter_mut().find(|p| p.id == payer_id) {
                                player.life -= i32::from(*amount);
                                true
                            } else {
                                false
                            }
                        }
                        UnlessCostType::Sacrifice {
                            count: _,
                            valid_type: _,
                        } => {
                            // TODO: Implement sacrifice payment
                            // For now, return false (can't pay)
                            log::debug!("UnlessCost: Sacrifice payment not yet implemented");
                            false
                        }
                        UnlessCostType::Mana(mana_cost) => {
                            // Actually tap mana sources and deduct the cost. This
                            // replaces the old auto-success (which neither tapped
                            // nor deducted) — required for correctness AND for a
                            // recursive copy chain (Chain Lightning, mtg-152) to
                            // TERMINATE: each copy costs {R}{R} again, so the
                            // chain stops once a player runs out of red sources.
                            self.pay_mana_cost_by_tapping(payer_id, mana_cost)
                        }
                        UnlessCostType::Reveal { count: _, card_type: _ } => {
                            // Reveal doesn't consume cards, just show them
                            // For now, assume success
                            true
                        }
                    }
                } else {
                    false
                };

                log::debug!(
                    "UnlessCost: payer={}, can_pay={}, should_pay={}, paid={}, switched={}",
                    payer_id.as_u32(),
                    can_pay,
                    should_pay,
                    paid,
                    unless_cost.switched
                );

                // Execute inner effect based on payment result and switched flag
                // - switched=true: execute if paid (e.g., "you may discard, if you do, draw")
                // - switched=false: execute if NOT paid (e.g., "counter unless you pay")
                let should_execute = if unless_cost.switched {
                    paid // Execute effect only if cost was paid
                } else {
                    !paid // Execute effect only if cost was NOT paid
                };

                if should_execute {
                    self.execute_effect(inner_effect)?;
                } else {
                    log::debug!(
                        "UnlessCost: inner effect skipped (paid={}, switched={})",
                        paid,
                        unless_cost.switched
                    );
                }
            }

            Effect::ChooseColor { player, source } => {
                // Choose a color using AI heuristic (pick most prominent color in deck)
                let chosen = self.pick_prominent_color(*player, &[]);

                // Store the chosen color on the source card
                if let Ok(card) = self.cards.get_mut(*source) {
                    let card_name = card.name.clone();
                    card.chosen_color = Some(chosen);
                    let player_name = self
                        .get_player(*player)
                        .map(|p| p.name.to_string())
                        .unwrap_or_else(|_| format!("Player {}", player.as_u32()));
                    self.logger
                        .normal(&format!("{} chooses color: {:?} ({})", player_name, chosen, card_name));
                } else {
                    log::warn!("ChooseColor: source card {} not found", source.as_u32());
                }
            }

            Effect::AddPhase { count } => {
                // Add extra combat phase(s) after the current step
                for _ in 0..*count {
                    self.extra_combat_phases += 1;
                }
                self.logger
                    .gamelog(&format!("AddPhase: {} additional combat phase(s) this turn", count));
            }
            Effect::Clone { .. } => {
                // Clone (Copy Artifact, etc.) requires a controller decision
                // ("you may" + which permanent to copy), so it is resolved by
                // the interactive path in priority.rs::resolve_clone_effect.
                // Reaching execute_effect means that interception was bypassed
                // (e.g. a non-interactive resolution path) — log rather than
                // silently entering as a vanilla permanent so the gap is visible.
                log::warn!(
                    target: "actions",
                    "Effect::Clone reached execute_effect without controller interception; \
                     permanent will not copy. This is a routing bug — Clone must go through \
                     the interactive spell-resolution hook."
                );
            }
            Effect::Unimplemented { api_type } => {
                // Log a warning instead of silently doing nothing
                log::warn!(
                    target: "actions",
                    "Unimplemented effect '{}' resolved as no-op",
                    api_type
                );
                self.logger.gamelog(&format!(
                    "WARNING: Effect '{}' is not yet implemented - resolving as no-op",
                    api_type
                ));
            }
            Effect::NoOp { api_type } => {
                // Intentional no-op (e.g. StoreSVar — the value it would stash is
                // modeled directly elsewhere). Silent: no warning, no gamelog.
                log::debug!(target: "actions", "NoOp effect '{}' (intentional)", api_type);
            }

            // XPaid variants should be resolved to concrete variants before execution
            // by resolve_x_paid_effect() in resolve_spell_execute_effects().
            // If we reach here, treat as amount=0 (shouldn't happen in normal flow).
            Effect::DealDamageXPaid { target, .. } => {
                log::warn!("DealDamageXPaid reached execute_effect without resolution, treating as 0 damage");
                self.execute_effect(&Effect::DealDamage {
                    target: target.clone(),
                    amount: 0,
                })?;
            }
            Effect::DrawCardsXPaid { player } => {
                log::warn!("DrawCardsXPaid reached execute_effect without resolution, treating as 0 cards");
                self.execute_effect(&Effect::DrawCards {
                    player: *player,
                    count: 0,
                })?;
            }
            Effect::DiscardCardsXPaid {
                player,
                remember_discarded,
            } => {
                log::warn!("DiscardCardsXPaid reached execute_effect without resolution, treating as 0 cards");
                self.execute_effect(&Effect::DiscardCards {
                    player: *player,
                    count: 0,
                    remember_discarded: *remember_discarded,
                    optional: false,
                    remember_discarding_players: false,
                })?;
            }

            Effect::ClassLevelUp {
                class_card_id,
                target_level,
            } => {
                self.execute_class_level_up(*class_card_id, *target_level)?;
            }
        }
        Ok(())
    }

    /// Execute the Class level-up mechanic (CR 716, Class supertype rules).
    ///
    /// 1. Add a `Level` counter to the Class permanent (tracks current level).
    /// 2. Fire any one-time `ClassLevelGained` triggers registered on the card.
    /// 3. Attach any permanent ongoing triggers / static abilities defined at
    ///    this level via `AddTrigger$` / `AddStaticAbility$`.
    fn execute_class_level_up(&mut self, class_card_id: crate::core::CardId, target_level: u8) -> Result<()> {
        use crate::core::{CounterType, KeywordArgs};

        // Verify the card is still on the battlefield.
        if self.cards.try_get(class_card_id).is_none() || !self.battlefield.cards.contains(&class_card_id) {
            log::debug!(
                "ClassLevelUp: card {:?} not found on battlefield — fizzling",
                class_card_id
            );
            return Ok(());
        }

        // Guard: only advance if the current level is exactly target_level - 1.
        // Prevents paying the same level-up cost multiple times (CR 716.2).
        let current_level = self
            .cards
            .try_get(class_card_id)
            .map(|c| c.get_counter(CounterType::Level))
            .unwrap_or(0);
        if current_level + 1 != target_level {
            log::debug!(
                "ClassLevelUp: {:?} is at level {} but target is {} — wrong level, fizzling",
                class_card_id,
                current_level,
                target_level
            );
            return Ok(());
        }

        // 1. Increment the Level counter on the Class permanent.
        self.add_counters(class_card_id, CounterType::Level, 1)?;

        let actual_level = self
            .cards
            .try_get(class_card_id)
            .map(|c| c.get_counter(CounterType::Level))
            .unwrap_or(0);

        let card_name = self
            .cards
            .try_get(class_card_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| "Class".to_string());

        self.logger
            .gamelog(&format!("{} advances to level {}", card_name, actual_level));

        // 2. Fire one-time ClassLevelGained triggers (uses the standard
        //    check_triggers infrastructure which scans all battlefield cards).
        self.check_triggers(TriggerEvent::ClassLevelGained { level: actual_level }, class_card_id)?;

        // 3. Collect Class keyword entries at this level for ongoing ability attachment.
        let class_abilities_at_level: Vec<String> = {
            let card = match self.cards.try_get(class_card_id) {
                Some(c) => c,
                None => return Ok(()),
            };
            card.keywords
                .iter_args()
                .filter_map(|kw| {
                    if let KeywordArgs::Class { level, abilities, .. } = kw {
                        if *level == target_level {
                            Some(abilities.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect()
        };

        for abilities_str in &class_abilities_at_level {
            self.apply_class_level_ongoing_abilities(class_card_id, abilities_str)?;
        }

        Ok(())
    }

    /// Parse and attach ongoing triggers / static abilities defined in a Class
    /// level's `AddTrigger$` or `AddStaticAbility$` clause.  Called after the
    /// level counter is advanced; one-time `ClassLevelGained` triggers are fired
    /// separately via `check_triggers`.
    ///
    /// `abilities_str` is the `<abilities>` portion of `K:Class:N:cost:<abilities>`,
    /// e.g. `"AddTrigger$ TriggerCast"` or `"AddStaticAbility$ SReduceCost"`.
    fn apply_class_level_ongoing_abilities(
        &mut self,
        class_card_id: crate::core::CardId,
        abilities_str: &str,
    ) -> Result<()> {
        use crate::loader::card::tokenize_pipe_dollar;

        let params = tokenize_pipe_dollar(abilities_str);

        // --- AddTrigger$ ---
        if let Some(svar_name) = params.get("AddTrigger") {
            let svar_body = self
                .cards
                .try_get(class_card_id)
                .and_then(|c| c.definition.svars.get(svar_name.as_str()).cloned());

            if let Some(body) = svar_body {
                // Parse the SVar trigger mode to determine if it's ongoing.
                // One-time ClassLevelGained triggers were already fired via check_triggers;
                // skip them here to avoid double-firing.
                let svar_params = tokenize_pipe_dollar(&body);
                let mode = svar_params.get("Mode").map(|s| s.as_str()).unwrap_or("");
                if mode == "ClassLevelGained" {
                    // Already handled by check_triggers above.
                    return Ok(());
                }

                // Ongoing trigger: parse using the card's own SVar context
                // (so Execute$ references like TrigToken resolve correctly).
                let card_name = self
                    .cards
                    .try_get(class_card_id)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|| "Class".to_string());

                let maybe_trigger = self
                    .cards
                    .try_get(class_card_id)
                    .and_then(|c| c.definition.parse_ongoing_trigger_from_svar_body(&body));

                if let Some(new_trigger) = maybe_trigger {
                    let desc = new_trigger.description.clone();
                    if let Ok(card) = self.cards.get_mut(class_card_id) {
                        log::info!("{} gains ongoing trigger: {}", card.name.as_str(), &desc);
                        card.triggers.push(new_trigger);
                    }
                    self.logger.gamelog(&format!("{} gains ability: {}", card_name, desc));
                } else {
                    log::debug!(
                        "apply_class_level_ongoing_abilities: AddTrigger$ {} body '{}' produced no trigger",
                        svar_name,
                        body
                    );
                }
            } else {
                log::debug!(
                    "apply_class_level_ongoing_abilities: SVar '{}' not found on class card",
                    svar_name
                );
            }
        }

        // --- AddStaticAbility$ ---
        if let Some(svar_name) = params.get("AddStaticAbility") {
            let svar_body = self
                .cards
                .try_get(class_card_id)
                .and_then(|c| c.definition.svars.get(svar_name.as_str()).cloned());

            if let Some(body) = svar_body {
                let card_name = self
                    .cards
                    .try_get(class_card_id)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|| "Class".to_string());

                let raw = format!("S:{}", body);
                let temp_script = format!("Name:{}\nManaCost:no cost\nTypes:Enchantment\n{}\n", card_name, raw);
                let Ok(card_def) = crate::loader::CardLoader::parse(&temp_script) else {
                    log::debug!(
                        "apply_class_level_ongoing_abilities: failed to parse temp script for AddStaticAbility$ {}",
                        svar_name
                    );
                    return Ok(());
                };
                let new_statics = card_def.parse_static_abilities();

                if new_statics.is_empty() {
                    log::debug!(
                        "apply_class_level_ongoing_abilities: AddStaticAbility$ {} produced no statics",
                        svar_name
                    );
                } else {
                    let count = new_statics.len();
                    if let Ok(card) = self.cards.get_mut(class_card_id) {
                        for s in new_statics {
                            card.static_abilities.push(s);
                        }
                    }
                    let card_name2 = self
                        .cards
                        .try_get(class_card_id)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| "Class".to_string());
                    for _ in 0..count {
                        self.logger.gamelog(&format!("{} gains static ability", card_name2));
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if a card matches a library search filter
    ///
    /// Filter formats supported:
    /// - "Land.Basic" = Land type + Basic subtype (matches any basic land)
    /// - "Creature" = Any Creature type
    /// - "Plains,Island" = Land with Plains OR Island subtype (fetch lands)
    /// - "Artifact.Equipment" = Artifact type + Equipment subtype
    /// - "Forest" = Land with Forest subtype (single subtype)
    pub fn card_matches_search_filter(card: &crate::core::Card, filter: &str) -> bool {
        // Check if filter is comma-separated subtypes (e.g., "Plains,Island")
        // This is the format used by fetch lands
        if filter.contains(',') {
            // Parse as comma-separated subtypes
            // These are land subtypes, so check if card is a land and has any of the subtypes
            if !card.is_land() {
                return false;
            }

            let subtypes: Vec<&str> = filter.split(',').collect();
            return subtypes
                .iter()
                .any(|subtype| card.subtypes.iter().any(|st| st.as_str() == *subtype));
        }

        // Check if filter has type.subtype format (e.g., "Land.Basic")
        if filter.contains('.') {
            let parts: Vec<&str> = filter.split('.').collect();
            let main_type = parts.first().unwrap_or(&"Card");
            let subtype = parts.get(1);

            // Check if card matches the main type
            let type_matches = Self::card_matches_type(card, main_type);

            // Check if card matches the subtype (if specified)
            let subtype_matches = if let Some(sub) = subtype {
                Self::card_matches_subtype(card, sub)
            } else {
                true
            };

            return type_matches && subtype_matches;
        }

        // Single word filter - could be a type OR a subtype
        // First check if it's a known card type
        if matches!(
            filter,
            "Card" | "Land" | "Creature" | "Artifact" | "Enchantment" | "Instant" | "Sorcery" | "Planeswalker"
        ) {
            return Self::card_matches_type(card, filter);
        }

        // Otherwise treat as a subtype (e.g., "Forest", "Plains", "Island")
        // For land subtypes, also verify card is a land
        if matches!(filter, "Plains" | "Island" | "Swamp" | "Mountain" | "Forest") {
            return card.is_land() && Self::card_matches_subtype(card, filter);
        }

        // Generic subtype check
        Self::card_matches_subtype(card, filter)
    }

    /// Score a card for Dig selection AI heuristic.
    /// Higher score = AI prefers to select this card.
    /// Creatures scored by P/T + CMC, lands at fixed 100, spells by CMC.
    fn dig_card_score(&self, card_id: CardId) -> i32 {
        let Some(card) = self.cards.try_get(card_id) else {
            return 0;
        };
        let cmc = i32::from(card.definition.mana_cost.cmc());
        if card.is_creature() {
            let power = i32::from(card.current_power());
            let toughness = i32::from(card.current_toughness());
            80 + (power + toughness) * 10 + cmc * 5
        } else if card.is_land() {
            100
        } else {
            50 + 30 * cmc
        }
    }

    /// Check if a card matches a card type
    fn card_matches_type(card: &crate::core::Card, type_name: &str) -> bool {
        match type_name {
            "Card" => true, // Any card
            "Land" => card.is_land(),
            "Creature" => card.is_creature(),
            "Artifact" => card.is_artifact(),
            "Enchantment" => card.is_enchantment(),
            "Instant" => card.is_instant(),
            "Sorcery" => card.types.contains(&CardType::Sorcery),
            "Planeswalker" => card.types.contains(&CardType::Planeswalker),
            _ => false,
        }
    }

    /// Check if a card matches a subtype
    fn card_matches_subtype(card: &crate::core::Card, subtype: &str) -> bool {
        if subtype == "Basic" {
            // "Basic" means any basic land subtype
            card.subtypes.iter().any(|st| {
                let st_str = st.as_str();
                st_str == "Plains"
                    || st_str == "Island"
                    || st_str == "Swamp"
                    || st_str == "Mountain"
                    || st_str == "Forest"
            })
        } else {
            // Check for specific subtype
            card.subtypes.iter().any(|st| st.as_str() == subtype)
        }
    }

    /// Choose the best permanent to sacrifice for an optional trigger cost.
    /// Returns None if no valid target exists.
    ///
    /// AI heuristic: pick the "lowest value" permanent matching the pattern.
    /// For creatures, this is based on P/T sum. For non-creatures, we prefer tokens.
    pub fn choose_sacrifice_target(
        &self,
        pattern: &str,
        source_card_id: CardId,
        player_id: PlayerId,
    ) -> Option<CardId> {
        // Parse the pattern - multiple options separated by semicolons
        let patterns: Vec<&str> = pattern.split(';').collect();

        // Collect all valid sacrifice targets with their "value" for AI comparison
        let mut candidates: Vec<(CardId, i32)> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;

                // Must be controlled by the player
                if card.controller != player_id {
                    return None;
                }

                // Check each pattern option (OR logic)
                for p in &patterns {
                    let mut matches = false;

                    // Check card type
                    if p.contains("Artifact") && card.is_artifact() {
                        matches = true;
                    }
                    if p.contains("Creature") && card.is_creature() {
                        matches = true;
                    }
                    if p.contains("Land") && card.is_land() {
                        matches = true;
                    }

                    // Check "Other" modifier - can't sacrifice the source
                    if p.contains(".Other") && card_id == source_card_id {
                        matches = false;
                    }

                    if matches {
                        // Calculate a "value" for this permanent (lower = better to sacrifice)
                        // Creatures: P/T sum (prefer low P/T creatures)
                        // Non-creatures: CMC (prefer low CMC)
                        let value = if card.is_creature() {
                            i32::from(card.current_power()) + i32::from(card.current_toughness())
                        } else {
                            i32::from(card.mana_cost.cmc())
                        };

                        return Some((card_id, value));
                    }
                }
                None
            })
            .collect();

        // Sort by value (ascending - lowest value first)
        candidates.sort_by_key(|(_, value)| *value);

        // Return the lowest-value target
        candidates.first().map(|(id, _)| *id)
    }

    /// Check if a sacrifice pattern cost can be paid by the given player.
    /// Returns true if the player has enough valid permanents to sacrifice.
    ///
    /// Sacrifice patterns are strings like:
    /// - "CARDNAME" - the source card itself (self-sacrifice; e.g. Clue token's
    ///   `{2}, Sac<1/CARDNAME>: Draw a card`, Strip Mine's
    ///   `T, Sac<1/CARDNAME>: Destroy target land`)
    /// - "Artifact.Other" - an artifact other than the source
    /// - "Creature.Other" - a creature other than the source
    /// - "Artifact.Other;Creature.Other" - an artifact or creature other than the source
    pub fn can_pay_sacrifice_pattern(
        &self,
        pattern: &str,
        count: u8,
        source_card_id: CardId,
        player_id: PlayerId,
    ) -> bool {
        // Parse the pattern - it can be multiple options separated by semicolons
        // e.g., "Artifact.Other;Creature.Other" means artifact OR creature
        let patterns: Vec<&str> = pattern.split(';').collect();

        // Special case: "CARDNAME" means sacrifice the source itself.
        // We pay this iff the source is on the battlefield under the activating
        // player's control. Mirrors the resolution path which simply pushes
        // `source_card_id` into `to_sacrifice` (see Cost::SacrificePattern handler
        // ~line 6916). Without this, abilities with cost `Sac<N/CARDNAME>` (Clue
        // tokens, sacrificed-itself mana lands, etc.) are silently filtered out
        // of the available-actions list. See bug-clue-token-activation.
        if patterns.iter().any(|p| p.eq_ignore_ascii_case("CARDNAME")) {
            // CARDNAME contributes exactly one sacrifice target (the source).
            // Combined with other pattern alternatives below, treat "source on
            // battlefield + we control it" as covering one needed sacrifice.
            if let Some(src) = self.cards.try_get(source_card_id) {
                if src.controller == player_id && self.battlefield.contains(source_card_id) && count <= 1 {
                    return true;
                }
            }
            // Note: count > 1 with CARDNAME doesn't really exist in practice; if
            // it ever does, fall through to the per-card scan below (which won't
            // count the source under any of the existing type patterns).
        }

        // Count valid sacrifice targets
        let valid_targets: usize = self
            .battlefield
            .cards
            .iter()
            .filter(|&&card_id| {
                if let Some(card) = self.cards.try_get(card_id) {
                    // Must be controlled by the player
                    if card.controller != player_id {
                        return false;
                    }

                    // Check each pattern option (OR logic)
                    for p in &patterns {
                        let mut matches = false;

                        // Check card type
                        if p.contains("Artifact") && card.is_artifact() {
                            matches = true;
                        }
                        if p.contains("Creature") && card.is_creature() {
                            matches = true;
                        }
                        if p.contains("Land") && card.is_land() {
                            matches = true;
                        }

                        // Check "Other" modifier - can't sacrifice self
                        if p.contains(".Other") && card_id == source_card_id {
                            matches = false;
                        }

                        if matches {
                            return true;
                        }
                    }
                }
                false
            })
            .count();

        valid_targets >= count as usize
    }

    /// Choose a card to discard from the player's hand.
    ///
    /// AI heuristic: pick the "lowest value" card.
    /// - Lands are preferred to discard (since hand is usually full of spells)
    /// - For spells, prefer higher CMC (less likely to cast soon)
    ///
    /// Returns None if the player has no cards in hand.
    ///
    /// # Errors
    ///
    /// Returns an error if the player's zones cannot be found.
    pub fn choose_card_to_discard(&self, player_id: PlayerId) -> Result<Option<CardId>> {
        // Gracefully handle missing zones (can happen if player has lost the game)
        let Some(zones) = self.get_player_zones(player_id) else {
            return Ok(None);
        };

        if zones.hand.is_empty() {
            return Ok(None);
        }

        // Collect cards with their "discard value" (higher = more desirable to discard)
        let mut candidates: Vec<(CardId, i32)> = zones
            .hand
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;

                // Calculate discard value (higher = better to discard)
                // Lands are most desirable to discard when looting (value 1000)
                // Spells: prefer discarding high CMC spells (value = CMC)
                // since we're likely to draw into something better
                let value = if card.is_land() {
                    1000
                } else {
                    i32::from(card.mana_cost.cmc())
                };

                Some((card_id, value))
            })
            .collect();

        // Sort by value (descending - highest value first = best to discard)
        candidates.sort_by_key(|(_, value)| -(*value));

        Ok(candidates.first().map(|(id, _)| *id))
    }

    /// Execute a discard-producing effect (`Effect::DiscardCards` or
    /// `Effect::Loot`) with an explicit discard `cause`: the controller of the
    /// spell/ability forcing the discard, or `None` for a self-initiated one.
    ///
    /// This is the single home for the (engine-chosen, non-interactive) discard
    /// and loot resolution logic. `GameState::execute_effect` delegates here
    /// with a `None` cause (so all of its other callers are unaffected). The
    /// forced-discard resolution sites that DO know the cause call this directly
    /// with `Some(cause)`: `check_triggers_inner` passes the trigger's
    /// controller (a triggered ability like Hypnotic Specter), and the
    /// interactive spell/ability paths in `priority.rs` pass the spell owner /
    /// priority holder. Threading the cause as an explicit argument (never
    /// mutable `GameState` state) keeps it out of serialized / rewound state
    /// entirely (mtg-648 / mtg-894).
    ///
    /// Returns `true` if `effect` was a discard-producing effect this helper
    /// handled, `false` otherwise (so a caller can fall back to the generic
    /// `execute_effect`).
    ///
    /// # Errors
    ///
    /// Propagates any error from the underlying discard / draw operations.
    // Intentional wildcard: this helper handles exactly the discard-producing
    // effects (DiscardCards / Loot) and reports `false` for everything else so
    // the caller falls back to the generic execute_effect.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn execute_discard_effect(&mut self, effect: &Effect, cause: Option<PlayerId>) -> Result<bool> {
        match effect {
            Effect::DiscardCards {
                player,
                count,
                remember_discarded,
                optional,
                remember_discarding_players,
            } => {
                // Optional discard: AI decides whether to discard.
                // For "discard hand, draw 7" patterns, always choose to discard.
                if *optional {
                    let hand_size = self
                        .get_player_zones(*player)
                        .map(|zones| zones.hand.cards.len())
                        .unwrap_or(0);
                    // AI heuristic: always discard when optional (the draw is typically worth it)
                    // A smarter heuristic could compare hand quality vs expected draw value
                    if hand_size == 0 {
                        // Nothing to discard - skip but still remember if discarding players
                        // (per rules: choosing to discard 0 cards is still choosing to discard)
                        return Ok(true);
                    }
                }

                if *count == u8::MAX {
                    // Mode$ Hand: discard ENTIRE hand unconditionally.
                    // We collect all card IDs first (can't borrow zones during mutation).
                    // Unlike the choose_card_to_discard path, this doesn't filter by card
                    // properties, so it works even for face-down/hidden cards on network clients.
                    // Sort by CardId for deterministic graveyard ordering across server/clients
                    // (hand iteration order can differ after WASM rewind+replay).
                    let mut hand_cards: smallvec::SmallVec<[CardId; 16]> = self
                        .get_player_zones(*player)
                        .map(|zones| zones.hand.cards.iter().copied().collect())
                        .unwrap_or_default();
                    hand_cards.sort_by_key(|id| id.as_u32());
                    let did_discard = !hand_cards.is_empty();
                    for card_id in hand_cards {
                        if *remember_discarded {
                            self.remembered_cards.push(card_id);
                        }
                        self.discard_card(*player, card_id, cause)?;
                    }
                    if *remember_discarding_players && did_discard {
                        self.remembered_players.push(*player);
                    }
                } else {
                    // Fixed count: forced discard chosen by the engine (e.g.
                    // Hypnotic Specter's "discards a card at random" trigger, or
                    // any non-interactive "discards a card" effect).
                    //
                    // mtg-589: This MUST be information-independent for network
                    // determinism. The previous heuristic (`choose_card_to_discard`)
                    // scored cards by card properties (lands / CMC), which requires
                    // the card identity to be materialized. On the server every
                    // hand card is materialized, but on a client's shadow state the
                    // OPPONENT's hand cards are reserved-but-unrevealed
                    // (`cards.get` → Err), so the heuristic saw an empty candidate
                    // set and discarded nothing — while the server discarded a
                    // card. Result: hand/graveyard counts diverged → FATAL desync
                    // (same hidden-info class as the library-search bug).
                    //
                    // Fix: select deterministically by CardId, which is synced
                    // across server + both clients (the hand zone's CardId list is
                    // identical even when identities are hidden). Lowest CardId is
                    // an arbitrary-but-stable rule; both views pick the same card,
                    // and move_card's Hand→Graveyard auto-reveal materializes it on
                    // the shadow. This applies to all forced discards (local and
                    // network) so behaviour stays identical across modes.
                    let mut did_discard = false;
                    for _ in 0..*count {
                        let card_to_discard = self
                            .get_player_zones(*player)
                            .and_then(|zones| zones.hand.cards.iter().copied().min_by_key(|id| id.as_u32()));
                        if let Some(card_id) = card_to_discard {
                            did_discard = true;
                            if *remember_discarded {
                                self.remembered_cards.push(card_id);
                            }
                            self.discard_card(*player, card_id, cause)?;
                        } else {
                            // No cards in hand to discard
                            break;
                        }
                    }
                    if *remember_discarding_players && did_discard {
                        self.remembered_players.push(*player);
                    }
                }
                Ok(true)
            }
            Effect::Loot {
                player,
                discard_count,
                draw_count,
            } => {
                // Looting: discard first, then draw
                // Use AI to choose what to discard
                for _ in 0..*discard_count {
                    let card_to_discard = self.choose_card_to_discard(*player)?;
                    if let Some(card_id) = card_to_discard {
                        self.discard_card(*player, card_id, cause)?;
                    } else {
                        // No cards to discard, can't complete the loot
                        break;
                    }
                }
                for _ in 0..*draw_count {
                    let (_, draw_num) = self.draw_card(*player)?;
                    // Check for "second card drawn" triggers
                    self.check_card_drawn_triggers(*player, draw_num)?;
                }
                Ok(true)
            }
            // Not a discard-producing effect — caller falls back to execute_effect.
            _ => Ok(false),
        }
    }

    /// Discard a specific card from the player's hand.
    ///
    /// Moves the card from hand to graveyard and logs the action.
    ///
    /// `cause` is the controller of the spell/ability that is FORCING this
    /// discard (CR 701.8 / CR 603), threaded explicitly from the resolution
    /// context — `None` for a self-initiated discard (cleanup-step
    /// over-the-limit discard, your own looting/rummaging, a discard you pay as
    /// a cost). It is consumed only by the `TriggerEvent::Discarded` self-trigger
    /// (Psychic Purge) to resolve `ValidCause$ SpellAbility.OppCtrl` and
    /// `Defined$ TriggeredCauseController`. Passing it as an explicit parameter
    /// (rather than mutable transient `GameState` state) keeps the cause out of
    /// the serialized/rewound game state entirely — there is nothing to
    /// reconstruct on a WASM rewind or a network shadow, which is the safest
    /// possible shape for a determinism-critical value.
    ///
    /// # Errors
    ///
    /// Returns an error if the card or player cannot be found, or if the
    /// card cannot be moved from hand to graveyard.
    pub fn discard_card(&mut self, player_id: PlayerId, card_id: CardId, cause: Option<PlayerId>) -> Result<()> {
        let player_name = self.get_player(player_id)?.name.clone();

        // Move card from hand to graveyard. The graveyard is a PUBLIC zone
        // (CR 400.2, 404), so this move reveals the card's identity to ALL
        // players: `move_card`'s Hand→Graveyard arm calls `maybe_reveal_to_all`
        // BEFORE the move, deterministically materializing the instance on a
        // network shadow (the late-binding `RevealCard` undo entry). Read the
        // name AFTER the move so the discard log line always reflects the now-
        // public identity — identical on the server, the shadow, the forward
        // pass, and any rewind+replay (mtg-610). Because the card is revealed-
        // to-all by construction at this point, `card_name` resolves to the
        // real name everywhere; there is no reveal-timing-dependent fallback.
        self.move_card(card_id, Zone::Hand, Zone::Graveyard, player_id)?;

        let card_name = self
            .cards
            .get(card_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));

        // Log the discard PUBLICLY: a discard puts the card into the graveyard,
        // a public zone, so every player learns its identity (CR 400.2). This
        // is the OPPOSITE of the draw log (drawing to hand is private); the log
        // visibility follows zone publicness.
        //
        // mtg-677: the DISPLAYED name depends on async reveal timing on a
        // network shadow (the discarded opponent card may not be materialised
        // yet — see `move_card`'s Hand→Graveyard note), so the displayed text
        // can be `card#52` on the first forward pass and `Disenchant` on a
        // rewind replay. The STATE is identical; only presentation differs. We
        // supply the rewind/replay verifier a reveal-timing-INDEPENDENT id form
        // so this is not flagged as a fatal desync, WITHOUT masking the public
        // name from any viewer in the UI.
        let verifier_stable = format!("{} discards card#{}", player_name, card_id.as_u32());
        self.logger
            .gamelog_reveal_stable(&format!("{} discards {}", player_name, card_name), &verifier_stable);

        // Fire battlefield-watcher discard triggers (T:Mode$ Discarded |
        // ValidCard$ Card.YouOwn, e.g. Monument to Endurance). CR 603.1: a
        // triggered ability fires whenever its event occurs. Called AFTER the
        // card is in the graveyard so the trigger can "see" it there.
        self.check_card_discarded_triggers(player_id)?;

        // Fire the DISCARDED CARD's own Discarded self-trigger (T:Mode$
        // Discarded | ValidCard$ Card.Self, e.g. Psychic Purge's opponent
        // punisher), on its LKI in the graveyard (CR 603.6/603.10). The cause
        // controller is threaded in explicitly (None for a self-discard).
        self.check_discarded_self_trigger(card_id, player_id, cause)?;

        Ok(())
    }

    /// Fire a [`TriggerEvent::Discarded`] self-trigger on a card that was just
    /// discarded (now in its owner's graveyard, fired on LKI — CR 603.6/603.10).
    ///
    /// `discarded_card` is the card that left hand; `owner` is the player who
    /// discarded it (its owner). `cause` is the controller of the spell/ability
    /// that forced the discard (threaded explicitly from the resolution
    /// context; `None` for a self-initiated discard). The trigger fires only if
    /// the card carries a `Discarded` trigger AND — when that trigger sets
    /// `requires_opponent_cause` (`ValidCause$ SpellAbility.OppCtrl`) — the
    /// discard was caused by a spell/ability controlled by an OPPONENT of
    /// `owner`. A self-caused discard (cleanup / own looting: `cause == owner`)
    /// or a cause-less discard (`cause == None`) does NOT fire an opponent-gated
    /// trigger.
    ///
    /// The trigger's `LoseLife` effect targets the
    /// `PlayerId::triggered_cause_controller()` sentinel, resolved here to the
    /// concrete cause controller.
    fn check_discarded_self_trigger(
        &mut self,
        discarded_card: CardId,
        owner: PlayerId,
        cause: Option<PlayerId>,
    ) -> Result<()> {
        // Collect the discarded card's Discarded triggers (LKI read from the
        // graveyard). Fast-path: most cards have none.
        let triggers: smallvec::SmallVec<[(bool, Vec<Effect>); 1]> = {
            let Some(card) = self.cards.try_get(discarded_card) else {
                return Ok(());
            };
            card.triggers
                .iter()
                .filter(|t| t.event == TriggerEvent::Discarded)
                .map(|t| (t.requires_opponent_cause, t.effects.clone()))
                .collect()
        };
        if triggers.is_empty() {
            return Ok(());
        }

        let cause_controller = cause;

        for (requires_opponent_cause, effects) in triggers {
            // ValidCause$ SpellAbility.OppCtrl gate: the cause must exist AND be
            // controlled by an OPPONENT of the discarding card's owner.
            if requires_opponent_cause {
                let opponent_caused = cause_controller.is_some_and(|cc| cc != owner);
                if !opponent_caused {
                    continue;
                }
            }

            // Resolve the cause-controller sentinel for this firing. If the gate
            // above passed, cause_controller is Some; for an ungated trigger
            // (no requires_opponent_cause) a missing cause leaves the sentinel
            // unresolved and the effect is skipped (no legal target).
            let Some(cause) = cause_controller else {
                continue;
            };

            for effect in &effects {
                // Resolve the cause-controller sentinel on a LoseLife payload
                // (Psychic Purge's "that player loses 5 life") to the concrete
                // cause. Any other effect shape is executed unchanged — the
                // parser only ever emits the sentinel LoseLife here, so this is
                // the single resolution case (avoiding a forbidden wildcard
                // enum match — see CLAUDE.md strong-types convention).
                let resolved = if let Effect::LoseLife { player, amount } = effect {
                    if player.is_triggered_cause_controller() {
                        Effect::LoseLife {
                            player: cause,
                            amount: *amount,
                        }
                    } else {
                        effect.clone()
                    }
                } else {
                    effect.clone()
                };
                self.execute_effect(&resolved)?;
            }
        }

        Ok(())
    }

    ///
    /// This checks all permanents on the battlefield for triggers matching the given event.
    /// When triggers are found, their effects are executed immediately (for now).
    ///
    /// Optional triggers with costs (e.g., "you may sacrifice...") are skipped if:
    /// - The cost cannot be paid (auto-decline)
    ///
    /// If the cost CAN be paid, the trigger fires (AI auto-accepts for now).
    /// TODO: Add player choice for optional triggers when the cost is payable.
    ///
    /// TODO: In full MTG rules, triggers should go on the stack and wait for priority,
    /// but for simplicity we're executing them immediately.
    ///
    /// Note: Wildcard match is intentional - only specific effects need placeholder
    /// target resolution; others execute as-is.
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    pub fn check_triggers(&mut self, event: TriggerEvent, source_card_id: CardId) -> Result<()> {
        self.check_triggers_inner(event, source_card_id, None)
    }

    /// Like [`check_triggers`](Self::check_triggers) but carries a single fixed
    /// damage amount for non-combat damage-driven triggers.
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    pub fn check_triggers_with_damage(
        &mut self,
        event: TriggerEvent,
        source_card_id: CardId,
        damage_amount: Option<i32>,
    ) -> Result<()> {
        let damage = damage_amount.map(crate::game::actions::triggers::DamageForTrigger::Fixed);
        self.check_triggers_inner(event, source_card_id, damage)
    }

    /// Fire `DealsCombatDamage` triggers for a creature that dealt combat damage
    /// this step, gating each trigger on its recipient class
    /// ([`CombatDamageTarget`](crate::core::CombatDamageTarget)) and threading
    /// the correct per-trigger amount (CR 510.2 -- combat damage is one
    /// simultaneous event; players AND creatures are valid recipients).
    ///
    /// This is the SINGLE shared firing path used by native, WASM, and network
    /// (shadow) execution -- combat damage is recorded once per creature in the
    /// deterministic `damage_dealt_by_creature` BTreeMap order, and every
    /// platform routes through here.
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    pub fn check_combat_damage_triggers(
        &mut self,
        source_card_id: CardId,
        breakdown: crate::game::actions::triggers::CombatDamageBreakdown,
    ) -> Result<()> {
        self.check_triggers_inner(
            TriggerEvent::DealsCombatDamage,
            source_card_id,
            Some(crate::game::actions::triggers::DamageForTrigger::Combat(breakdown)),
        )
    }

    /// Shared trigger-firing implementation for [`check_triggers`],
    /// [`check_triggers_with_damage`], and [`check_combat_damage_triggers`].
    ///
    /// `damage` carries the amount each fired trigger observes
    /// (`TriggerCount$DamageAmount`): `None` for non-damage events,
    /// `Some(Fixed(n))` for a single fixed amount (non-combat damage), or
    /// `Some(Combat(breakdown))` for combat damage, where each
    /// `DealsCombatDamage` trigger is additionally gated on its
    /// [`CombatDamageTarget`](crate::core::CombatDamageTarget) recipient class
    /// and observes the matching slice of the breakdown.
    ///
    /// [`check_triggers`]: Self::check_triggers
    /// [`check_triggers_with_damage`]: Self::check_triggers_with_damage
    /// [`check_combat_damage_triggers`]: Self::check_combat_damage_triggers
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn check_triggers_inner(
        &mut self,
        event: TriggerEvent,
        source_card_id: CardId,
        damage: Option<crate::game::actions::triggers::DamageForTrigger>,
    ) -> Result<()> {
        use crate::core::Trigger;

        // Info needed to check trigger payability and execute costs
        struct TriggerInfo {
            card_id: CardId,
            card_name: crate::core::types::CardName, // Use Arc<str> instead of String to avoid heap allocation
            controller: PlayerId,
            trigger: Trigger,
            /// Per-trigger damage amount observed (after recipient-class gating).
            damage_amount: Option<i32>,
        }

        // Collected trigger with cost info for execution
        struct TriggerToExecute {
            source_card_id: CardId,
            effects: Vec<Effect>,
            sacrifice_target: Option<CardId>, // Card to sacrifice for the cost
            sacrificed_power: u8,             // Power of sacrifice target (for Firebend effects)
            /// Per-trigger damage amount observed (after recipient-class gating).
            damage_amount: Option<i32>,
        }

        // Pre-compute source card info for trigger filtering (landfall check, etc.)
        // We need this before the iterator borrows self
        let source_card_is_land = self.cards.try_get(source_card_id).is_some_and(|c| c.is_land());
        let source_card_controller = self.cards.try_get(source_card_id).map(|c| c.controller);

        // Phase 1: Collect matching triggers with their metadata
        // Use flat_map to avoid inner Vec allocation per card - most cards have no matching triggers
        let candidate_triggers: Vec<TriggerInfo> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| self.cards.try_get(card_id).map(|card| (card_id, card)))
            .flat_map(|(card_id, card)| {
                let controller = card.controller;
                let card_name = &card.name;
                let attached_to = card.attached_to;

                card.triggers.iter().filter_map(move |trigger| {
                    // Check event type matches
                    if trigger.event != event {
                        return None;
                    }

                    // Self-only triggers only fire when the trigger source is the event source
                    if trigger.trigger_self_only && card_id != source_card_id {
                        return None;
                    }

                    // Attached-source triggers (Aura/Equipment watching its host,
                    // e.g. Spirit Link `ValidSource$ Card.AttachedBy`) only fire
                    // when the event source is the permanent this card is attached to.
                    if trigger.requires_attached_source && attached_to != Some(source_card_id) {
                        return None;
                    }

                    // "[other]" triggers only fire when the event source is DIFFERENT from trigger source
                    // (e.g., "whenever you sacrifice another permanent" on Pirate Peddlers)
                    // OPTIMIZATION: Use pre-parsed boolean flag instead of runtime .contains()
                    if trigger.requires_other && card_id == source_card_id {
                        return None;
                    }

                    // "[landfall]" triggers only fire when:
                    // 1. The entering card is a Land
                    // 2. The entering card is controlled by the trigger's controller
                    // OPTIMIZATION: Use pre-parsed boolean flag instead of runtime .contains()
                    if trigger.requires_landfall {
                        if !source_card_is_land {
                            return None;
                        }
                        if source_card_controller != Some(controller) {
                            return None;
                        }
                    }

                    // Damage-amount + combat recipient-class gate (CR 510.2).
                    // For combat damage, a player-only trigger (e.g. Hypnotic
                    // Specter) must NOT fire when the creature only damaged a
                    // blocker, while an Any trigger (Spirit Link lifelink)
                    // fires on damage to players AND creatures. `amount_for`
                    // returns `None` to skip when the recipient class wasn't
                    // hit. Non-damage events (`damage == None`) carry no
                    // amount and are not gated.
                    let damage_amount = match damage {
                        None => None,
                        Some(d) => {
                            // Combat-only triggers (CombatDamage$ True, e.g.
                            // Hypnotic Specter) must NOT fire on non-combat
                            // damage (the Fixed variant). The combat path uses
                            // the Combat variant and fires them normally.
                            if trigger.requires_combat_damage
                                && matches!(d, crate::game::actions::triggers::DamageForTrigger::Fixed(_))
                            {
                                return None;
                            }
                            match d.amount_for(trigger.combat_damage_target) {
                                Some(amount) => Some(amount),
                                None => return None,
                            }
                        }
                    };

                    Some(TriggerInfo {
                        card_id,
                        card_name: card_name.clone(), // Clone Arc<str> only for matching triggers
                        controller,
                        trigger: trigger.clone(),
                        damage_amount,
                    })
                })
            })
            .collect();

        // Phase 2: Filter by cost payability, choose sacrifice targets, and collect effects
        let triggered_effects: Vec<TriggerToExecute> = candidate_triggers
            .into_iter()
            .filter_map(|info| {
                let mut sacrifice_target: Option<CardId> = None;
                let mut sacrificed_power: u8 = 0;

                // For optional triggers with costs, check payability and choose targets
                if info.trigger.optional {
                    if let Some(ref cost) = info.trigger.cost {
                        // Check if sacrifice cost can be paid
                        if let Some((count, pattern)) = cost.get_sacrifice_pattern() {
                            if !self.can_pay_sacrifice_pattern(pattern, count, info.card_id, info.controller) {
                                log::debug!(
                                    "Skipping optional trigger on {} - sacrifice cost not payable (need {} {})",
                                    info.card_name,
                                    count,
                                    pattern
                                );
                                return None; // Auto-decline if can't pay
                            }

                            // Choose which permanent to sacrifice (AI heuristic: pick lowest P/T creature or artifact)
                            sacrifice_target = self.choose_sacrifice_target(pattern, info.card_id, info.controller);

                            // Capture power of sacrifice target for Firebend effects (Fire Lord Ozai)
                            if let Some(sac_id) = sacrifice_target {
                                if let Ok(sac_card) = self.cards.get(sac_id) {
                                    sacrificed_power = sac_card.current_power().max(0) as u8;
                                }
                            }
                        }
                        // TODO: Check other cost types (mana, life, etc.)
                    }
                }

                // Trigger passes all checks - collect effects
                if !info.trigger.effects.is_empty() {
                    log::debug!(
                        "Found {} triggers on card {} ({})",
                        info.trigger.effects.len(),
                        info.card_id.as_u32(),
                        info.card_name
                    );
                    for effect in &info.trigger.effects {
                        log::debug!("  Trigger effect: {:?}", effect);
                    }
                    if let Some(sac_id) = sacrifice_target {
                        if let Ok(sac_card) = self.cards.get(sac_id) {
                            log::debug!(
                                "  Will sacrifice: {} ({}) power={}",
                                sac_card.name,
                                sac_id.as_u32(),
                                sacrificed_power
                            );
                        }
                    }
                    Some(TriggerToExecute {
                        source_card_id: info.card_id,
                        effects: info.trigger.effects,
                        sacrifice_target,
                        sacrificed_power,
                        damage_amount: info.damage_amount,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Phase 3: Execute sacrifices and triggered effects
        for trigger_to_exec in triggered_effects {
            let trigger_source = trigger_to_exec.source_card_id;
            let sacrificed_power = trigger_to_exec.sacrificed_power;

            // Execute sacrifice cost first (if any)
            if let Some(sac_target) = trigger_to_exec.sacrifice_target {
                if let Ok(sac_card) = self.cards.get(sac_target) {
                    let sac_name = sac_card.name.to_string();
                    let sac_owner = sac_card.owner;
                    log::info!("Sacrificing {} ({}) for trigger cost", sac_name, sac_target.as_u32());

                    // Move from battlefield to graveyard (or exile if finality counter)
                    let sac_dest = self.death_destination_for_card(sac_target);
                    self.move_card(sac_target, Zone::Battlefield, sac_dest, sac_owner)?;
                    self.check_triggers(TriggerEvent::Sacrificed, sac_target)?;
                }
            }

            // Build trigger context for placeholder resolution
            let trigger_card = self.cards.get(trigger_source)?;
            let controller = trigger_card.controller;
            let trigger_source_colors: smallvec::SmallVec<[crate::core::Color; 2]> = trigger_card.colors.clone();
            let opponent = self.players.iter().find(|p| p.id != controller).map(|p| p.id);
            let ctx = TriggerContext::new(trigger_source, controller)
                .with_event_source(source_card_id)
                .with_sacrificed_power(sacrificed_power);
            let ctx = if let Some(opp) = opponent {
                ctx.with_opponent(opp)
            } else {
                ctx
            };
            // Thread the per-trigger damage amount for damage-driven triggers
            // (Spirit Link's "gain that much life", Mark of Sakiko's "add that
            // much {G}"). For combat damage this is the recipient-class slice
            // selected during filtering (total / to-player / to-creature).
            let ctx = if let Some(dmg) = trigger_to_exec.damage_amount {
                ctx.with_damage_amount(dmg)
            } else {
                ctx
            };

            // Track the most recent target chosen during this trigger's effect chain.
            // Used to share targets across SubAbility chains like
            //   DB$ Attach | ValidTgts$ Creature.YouCtrl | SubAbility$ DBPump
            //   SVar:DBPump:DB$ Pump | Defined$ Targeted | KW$ Double Strike
            // where the Pump must apply to the same creature the Attach picked.
            let mut last_chosen_target: Option<CardId> = None;

            // Execute all trigger effects with placeholder resolution
            for effect in trigger_to_exec.effects {
                // Step 1: Apply shared placeholder resolution for simple cases
                // (player placeholders, self-targeting, token creation)
                let mut effect = resolve_effect_placeholder(&effect, &ctx);

                // Step 2: Handle complex targeting that requires battlefield search
                // These cases need game state access and can't be done in shared function
                match &effect {
                    // DealDamage with TargetRef::None after shared resolution means we should
                    // try to find a creature target first (for "any target" effects like Mongoose Lizard)
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } => {
                        // Try to find opponent's creature first, sorted by CardId for determinism.
                        // Using .find() on unsorted battlefield iteration order would produce
                        // different results after rewind+replay if the internal ordering changed.
                        let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                            .battlefield
                            .cards
                            .iter()
                            .filter(|&card_id| {
                                if let Some(card) = self.cards.try_get(*card_id) {
                                    card.is_creature()
                                        && card.controller != controller
                                        && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                } else {
                                    false
                                }
                            })
                            .copied()
                            .collect();
                        candidates.sort_by_key(|id| id.as_u32());
                        if let Some(&target_id) = candidates.first() {
                            effect = Effect::DealDamage {
                                target: TargetRef::Permanent(target_id),
                                amount: *amount,
                            };
                        } else if let Some(opp) = opponent {
                            // Fall back to opponent player
                            effect = Effect::DealDamage {
                                target: TargetRef::Player(opp),
                                amount: *amount,
                            };
                        }
                        // else stays as TargetRef::None and will fizzle
                    }
                    Effect::DestroyPermanent {
                        target,
                        restriction,
                        no_regenerate,
                    } if target.is_placeholder() => {
                        // Find a valid target matching the restriction (controller
                        // semantics included: ActivePlayerCtrl/YouCtrl/OppCtrl/Any),
                        // sorted by CardId for determinism after rewind+replay.
                        // The active player is the trigger source controller's turn
                        // by default here; for "each player's upkeep" triggers the
                        // dedicated `check_triggers_for_controller` path supplies the
                        // real active player.
                        if let Some(target_id) = self.choose_triggered_destroy_target(
                            restriction,
                            controller,
                            controller,
                            &trigger_source_colors,
                        ) {
                            effect = Effect::DestroyPermanent {
                                target: target_id,
                                restriction: restriction.clone(),
                                no_regenerate: *no_regenerate,
                            };
                        }
                    }
                    // AttachEquipment: target_creature placeholder → find a creature
                    // controlled by the trigger's controller (Card.YouCtrl restriction).
                    // Used by Equipment ETB triggers like Twin Blades.
                    Effect::AttachEquipment {
                        source_equipment,
                        target_creature,
                    } if target_creature.is_placeholder() => {
                        // Find a valid target creature controlled by the trigger's controller,
                        // sorted by CardId for determinism after rewind+replay.
                        let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                            .battlefield
                            .cards
                            .iter()
                            .filter(|&card_id| {
                                if let Some(card) = self.cards.try_get(*card_id) {
                                    card.is_creature()
                                        && card.controller == controller
                                        && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                } else {
                                    false
                                }
                            })
                            .copied()
                            .collect();
                        candidates.sort_by_key(|id| id.as_u32());
                        if let Some(&target_id) = candidates.first() {
                            effect = Effect::AttachEquipment {
                                source_equipment: *source_equipment,
                                target_creature: target_id,
                            };
                            last_chosen_target = Some(target_id);
                        } else {
                            // No valid creature target — trigger fizzles (CR 603.10)
                            log::debug!(
                                "AttachEquipment trigger from {:?} fizzles: no valid Creature.YouCtrl target",
                                trigger_source
                            );
                            continue;
                        }
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                        keywords_granted,
                    } if target.is_placeholder() => {
                        // If a previous effect in this trigger chain (e.g. Attach) chose a
                        // target, reuse it — this models Defined$ Targeted in SubAbility
                        // chains like Twin Blades' DBPump.
                        if let Some(prior_target) = last_chosen_target {
                            effect = Effect::PumpCreature {
                                target: prior_target,
                                power_bonus: *power_bonus,
                                toughness_bonus: *toughness_bonus,
                                keywords_granted: keywords_granted.clone(),
                            };
                        } else {
                            // Find a valid target (any creature on battlefield),
                            // sorted by CardId for determinism after rewind+replay.
                            let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&card_id| {
                                    if let Some(card) = self.cards.try_get(*card_id) {
                                        card.is_creature()
                                            && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                    } else {
                                        false
                                    }
                                })
                                .copied()
                                .collect();
                            candidates.sort_by_key(|id| id.as_u32());
                            if let Some(&target_id) = candidates.first() {
                                effect = Effect::PumpCreature {
                                    target: target_id,
                                    power_bonus: *power_bonus,
                                    toughness_bonus: *toughness_bonus,
                                    keywords_granted: keywords_granted.clone(),
                                };
                            }
                        }
                    }
                    Effect::DebuffCreature {
                        target,
                        keywords_removed,
                    } if target.is_placeholder() => {
                        // Find a valid target creature for debuff
                        let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                            .battlefield
                            .cards
                            .iter()
                            .filter(|&card_id| {
                                if let Some(card) = self.cards.try_get(*card_id) {
                                    card.is_creature()
                                        && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                } else {
                                    false
                                }
                            })
                            .copied()
                            .collect();
                        candidates.sort_by_key(|id| id.as_u32());
                        if let Some(&target_id) = candidates.first() {
                            effect = Effect::DebuffCreature {
                                target: target_id,
                                keywords_removed: keywords_removed.clone(),
                            };
                        }
                    }
                    // Note: CreateToken is handled by resolve_effect_placeholder
                    Effect::ExilePermanent { target } if target.is_placeholder() => {
                        // Find a valid target (opponent's nonland permanent),
                        // sorted by CardId for determinism after rewind+replay.
                        // Web Up and similar cards: "exile target nonland permanent an opponent controls"
                        let controller = self.cards.get(trigger_source)?.controller;
                        let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                            .battlefield
                            .cards
                            .iter()
                            .filter(|&card_id| {
                                if let Some(card) = self.cards.try_get(*card_id) {
                                    !card.is_land()
                                        && card.controller != controller
                                        && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                } else {
                                    false
                                }
                            })
                            .copied()
                            .collect();
                        candidates.sort_by_key(|id| id.as_u32());
                        if let Some(&target_id) = candidates.first() {
                            effect = Effect::ExilePermanent { target: target_id };
                        }
                    }
                    Effect::Earthbend { target, num_counters } if target.is_placeholder() => {
                        // Placeholder CardId 0 means we need to target a land the controller controls
                        // For now, pick the first land they control (AI could choose better targets)
                        let controller = self.cards.get(trigger_source)?.controller;

                        // Find a land controlled by the trigger's controller,
                        // sorted by CardId for determinism after rewind+replay.
                        let mut land_candidates: smallvec::SmallVec<[CardId; 8]> = self
                            .battlefield
                            .cards
                            .iter()
                            .filter_map(|cid| {
                                let card = self.cards.get(*cid).ok()?;
                                if card.controller == controller && card.is_land() {
                                    Some(*cid)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        land_candidates.sort_by_key(|id| id.as_u32());
                        if let Some(&land_id) = land_candidates.first() {
                            effect = Effect::Earthbend {
                                target: land_id,
                                num_counters: *num_counters,
                            };
                        } else {
                            // No valid land target - skip this trigger
                            continue;
                        }
                    }
                    // Note: PumpAllCreatures is handled by resolve_effect_placeholder
                    // Note: Firebend placeholder resolution handled by resolve_effect_placeholder
                    // Log firebend effect after resolution
                    Effect::Firebend { amount, .. } if *amount > 0 => {
                        if let Some(card) = self.cards.try_get(trigger_source) {
                            self.logger.gamelog(&format!(
                                "{} triggers Firebending {} (adding {} {{R}} to combat mana)",
                                card.name, amount, amount
                            ));
                        }
                    }
                    Effect::UntapPermanent { target } if target.is_placeholder() => {
                        // Placeholder CardId 0 means we need to target an artifact or creature
                        // Cat-Owl trigger: "untap target artifact or creature"
                        // Heuristic: prefer tapped friendly permanents
                        let controller = self.cards.get(trigger_source)?.controller;

                        // Find the best target to untap:
                        // 1. Tapped friendly creatures (highest priority)
                        // 2. Tapped friendly artifacts
                        // 3. Any tapped creature/artifact (even opponent's, if allowed)
                        let target_id = self
                            .battlefield
                            .cards
                            .iter()
                            .filter_map(|cid| {
                                let card = self.cards.get(*cid).ok()?;
                                // Must be artifact or creature
                                if !card.is_artifact() && !card.is_creature() {
                                    return None;
                                }
                                // Must be tapped (untapping untapped permanent is pointless)
                                if !card.tapped {
                                    return None;
                                }
                                // Skip the source card itself (can't untap self while attacking)
                                if *cid == trigger_source {
                                    return None;
                                }
                                // Check for hexproof/shroud (CR 702.18a, CR 702.19a)
                                if !targeting::is_legal_target(card, controller, &trigger_source_colors) {
                                    return None;
                                }
                                // Score: prefer friendly permanents
                                let score = if card.controller == controller { 100 } else { 0 };
                                Some((*cid, score))
                            })
                            .max_by_key(|(_, score)| *score)
                            .map(|(id, _)| id);

                        if let Some(target_id) = target_id {
                            effect = Effect::UntapPermanent { target: target_id };
                        } else {
                            // No valid target - skip this trigger
                            continue;
                        }
                    }
                    _ => {}
                }

                // A discard FORCED by this triggered ability (Hypnotic Specter's
                // "that player discards a card at random", The Rack-family, etc.)
                // carries the trigger's CONTROLLER as the discard cause, so a
                // Discarded self-trigger on the discarded card (Psychic Purge's
                // opponent punisher) fires when an opponent's ability caused it.
                // Route DiscardCards/Loot through the cause-aware helper; every
                // other effect uses the generic path (mtg-648 / mtg-894).
                if !self.execute_discard_effect(&effect, Some(controller))? {
                    self.execute_effect(&effect)?;
                }
            }
        }

        Ok(())
    }

    /// Check triggers when a card taps for mana.
    ///
    /// # Errors
    ///
    /// Returns an error if any trigger effect fails to execute.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_taps_for_mana_triggers(&mut self, source_card_id: CardId, activator_player: PlayerId) -> Result<()> {
        use crate::core::Trigger;

        // Info needed to check trigger payability and execute costs
        struct TriggerInfo {
            card_id: CardId,
            controller: PlayerId,
            trigger: Trigger,
        }

        // Collected trigger with cost info for execution
        struct TriggerToExecute {
            source_card_id: CardId,
            effects: Vec<Effect>,
            sacrifice_target: Option<CardId>,
            sacrificed_power: u8,
        }

        // Phase 1: Collect matching triggers with their metadata
        let mut candidate_triggers = Vec::new();
        let active_player = self.turn.active_player;

        for &card_id in &self.battlefield.cards {
            if let Some(card) = self.cards.try_get(card_id) {
                let controller = card.controller;
                for trigger in &card.triggers {
                    if trigger.event != TriggerEvent::TapsForMana {
                        continue;
                    }

                    // Check activator restriction
                    if let Some(ref activator) = trigger.taps_for_mana_activator {
                        match activator.as_str() {
                            "You" => {
                                if activator_player != controller {
                                    continue;
                                }
                            }
                            "Opponent" => {
                                if activator_player == controller {
                                    continue;
                                }
                            }
                            "Player.NonActive" => {
                                if activator_player == active_player {
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Check ValidCard filter
                    if let Some(ref filter) = trigger.taps_for_mana_valid_card {
                        if let Ok(source_card) = self.cards.get(source_card_id) {
                            if !matches_taps_for_mana_filter(source_card, filter, card, controller) {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    }

                    candidate_triggers.push(TriggerInfo {
                        card_id,
                        controller,
                        trigger: trigger.clone(),
                    });
                }
            }
        }

        // Phase 2: Filter by cost payability, choose sacrifice targets, and collect effects
        let mut triggered_effects = Vec::new();
        for info in candidate_triggers {
            let mut sacrifice_target: Option<CardId> = None;
            let mut sacrificed_power: u8 = 0;

            if info.trigger.optional {
                if let Some(ref cost) = info.trigger.cost {
                    if let Some((count, pattern)) = cost.get_sacrifice_pattern() {
                        if !self.can_pay_sacrifice_pattern(pattern, count, info.card_id, info.controller) {
                            continue;
                        }
                        sacrifice_target = self.choose_sacrifice_target(pattern, info.card_id, info.controller);
                        if let Some(sac_id) = sacrifice_target {
                            if let Ok(sac_card) = self.cards.get(sac_id) {
                                sacrificed_power = sac_card.current_power().max(0) as u8;
                            }
                        }
                    }
                }
            }

            if !info.trigger.effects.is_empty() {
                triggered_effects.push(TriggerToExecute {
                    source_card_id: info.card_id,
                    effects: info.trigger.effects,
                    sacrifice_target,
                    sacrificed_power,
                });
            }
        }

        // Phase 3: Execute sacrifices and triggered effects
        for trigger_to_exec in triggered_effects {
            let trigger_source = trigger_to_exec.source_card_id;
            let sacrificed_power = trigger_to_exec.sacrificed_power;

            if let Some(sac_target) = trigger_to_exec.sacrifice_target {
                if let Ok(sac_card) = self.cards.get(sac_target) {
                    let sac_owner = sac_card.owner;
                    let sac_dest = self.death_destination_for_card(sac_target);
                    self.move_card(sac_target, Zone::Battlefield, sac_dest, sac_owner)?;
                    self.check_triggers(TriggerEvent::Sacrificed, sac_target)?;
                }
            }

            let trigger_card = self.cards.get(trigger_source)?;
            let controller = trigger_card.controller;
            let trigger_source_colors: smallvec::SmallVec<[crate::core::Color; 2]> = trigger_card.colors.clone();
            let opponent = self.players.iter().find(|p| p.id != controller).map(|p| p.id);
            let ctx = TriggerContext::new(trigger_source, controller)
                .with_event_source(source_card_id)
                .with_sacrificed_power(sacrificed_power);
            let ctx = if let Some(opp) = opponent {
                ctx.with_opponent(opp)
            } else {
                ctx
            };

            let mut last_chosen_target: Option<CardId> = None;

            for effect in trigger_to_exec.effects {
                let mut effect = resolve_effect_placeholder(&effect, &ctx);

                match &mut effect {
                    Effect::DealDamage {
                        target: ref mut target_ref,
                        ..
                    } => {
                        if matches!(target_ref, TargetRef::None) {
                            let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&card_id| {
                                    if let Some(card) = self.cards.try_get(*card_id) {
                                        card.is_creature()
                                            && card.controller != controller
                                            && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                    } else {
                                        false
                                    }
                                })
                                .copied()
                                .collect();
                            candidates.sort_by_key(|id| id.as_u32());
                            if let Some(&target_id) = candidates.first() {
                                *target_ref = TargetRef::Permanent(target_id);
                            } else if let Some(opp) = opponent {
                                *target_ref = TargetRef::Player(opp);
                            }
                        }
                    }
                    Effect::DestroyPermanent {
                        target: ref mut target_id,
                        restriction,
                        ..
                    } => {
                        if target_id.is_placeholder() {
                            if let Some(chosen_id) = self.choose_triggered_destroy_target(
                                restriction,
                                controller,
                                controller,
                                &trigger_source_colors,
                            ) {
                                *target_id = chosen_id;
                            }
                        }
                    }
                    Effect::AttachEquipment {
                        target_creature: ref mut target_id,
                        ..
                    } => {
                        if target_id.is_placeholder() {
                            let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&card_id| {
                                    if let Some(card) = self.cards.try_get(*card_id) {
                                        card.is_creature()
                                            && card.controller == controller
                                            && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                    } else {
                                        false
                                    }
                                })
                                .copied()
                                .collect();
                            candidates.sort_by_key(|id| id.as_u32());
                            if let Some(&chosen_id) = candidates.first() {
                                *target_id = chosen_id;
                                last_chosen_target = Some(chosen_id);
                            } else {
                                continue;
                            }
                        }
                    }
                    Effect::PumpCreature {
                        target: ref mut target_id,
                        ..
                    } => {
                        if target_id.is_placeholder() {
                            if let Some(prior_target) = last_chosen_target {
                                *target_id = prior_target;
                            } else {
                                let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                    .battlefield
                                    .cards
                                    .iter()
                                    .filter(|&card_id| {
                                        if let Some(card) = self.cards.try_get(*card_id) {
                                            card.is_creature()
                                                && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                        } else {
                                            false
                                        }
                                    })
                                    .copied()
                                    .collect();
                                candidates.sort_by_key(|id| id.as_u32());
                                if let Some(&chosen_id) = candidates.first() {
                                    *target_id = chosen_id;
                                }
                            }
                        }
                    }
                    Effect::DebuffCreature {
                        target: ref mut target_id,
                        ..
                    } => {
                        if target_id.is_placeholder() {
                            let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&card_id| {
                                    if let Some(card) = self.cards.try_get(*card_id) {
                                        card.is_creature()
                                            && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                    } else {
                                        false
                                    }
                                })
                                .copied()
                                .collect();
                            candidates.sort_by_key(|id| id.as_u32());
                            if let Some(&chosen_id) = candidates.first() {
                                *target_id = chosen_id;
                            }
                        }
                    }
                    Effect::ExilePermanent {
                        target: ref mut target_id,
                    } => {
                        if target_id.is_placeholder() {
                            let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&card_id| {
                                    if let Some(card) = self.cards.try_get(*card_id) {
                                        !card.is_land()
                                            && card.controller != controller
                                            && targeting::is_legal_target(card, controller, &trigger_source_colors)
                                    } else {
                                        false
                                    }
                                })
                                .copied()
                                .collect();
                            candidates.sort_by_key(|id| id.as_u32());
                            if let Some(&chosen_id) = candidates.first() {
                                *target_id = chosen_id;
                            }
                        }
                    }
                    Effect::Earthbend {
                        target: ref mut target_id,
                        ..
                    } => {
                        if target_id.is_placeholder() {
                            let mut land_candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter_map(|cid| {
                                    let card = self.cards.get(*cid).ok()?;
                                    if card.controller == controller && card.is_land() {
                                        Some(*cid)
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            land_candidates.sort_by_key(|id| id.as_u32());
                            if let Some(&land_id) = land_candidates.first() {
                                *target_id = land_id;
                            } else {
                                continue;
                            }
                        }
                    }
                    Effect::UntapPermanent {
                        target: ref mut target_id,
                    } => {
                        if target_id.is_placeholder() {
                            let chosen_id = self
                                .battlefield
                                .cards
                                .iter()
                                .filter_map(|cid| {
                                    let card = self.cards.get(*cid).ok()?;
                                    if !card.is_artifact() && !card.is_creature() {
                                        return None;
                                    }
                                    if !card.tapped {
                                        return None;
                                    }
                                    if *cid == trigger_source {
                                        return None;
                                    }
                                    if !targeting::is_legal_target(card, controller, &trigger_source_colors) {
                                        return None;
                                    }
                                    let score = if card.controller == controller { 100 } else { 0 };
                                    Some((*cid, score))
                                })
                                .max_by_key(|(_, score)| *score)
                                .map(|(id, _)| id);

                            if let Some(chosen_id) = chosen_id {
                                *target_id = chosen_id;
                            } else {
                                continue;
                            }
                        }
                    }
                    _ => {}
                }

                self.execute_effect(&effect)?;
            }
        }

        Ok(())
    }

    /// Check and execute triggered abilities for a specific card only
    ///
    /// This is used by phase triggers where we've already determined which cards
    /// should trigger based on the active player (controller_only filtering).
    /// Accepts the active player for proper trigger filtering.
    ///
    /// Note: Wildcard matches are intentional - only specific effects need placeholder
    /// resolution or formatted logging; others pass through unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found or effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    /// Choose a target for a triggered `DestroyPermanent` whose target is still a
    /// placeholder, honoring the `TargetRestriction`'s type/controller filters.
    ///
    /// Triggered abilities in this engine resolve their targets via deterministic
    /// engine selection (sorted by `CardId`) rather than going on the stack — this
    /// keeps controllers information-independent and replays byte-identical
    /// (see `docs/NETWORK_ARCHITECTURE.md`). This helper is the single source of
    /// truth for that selection, shared by `check_triggers` (event-source path)
    /// and `check_triggers_for_controller` (phase-trigger path).
    ///
    /// Controller semantics:
    /// - `Any`               → any creature/permanent matching the type filter
    /// - `YouCtrl`           → controlled by `trigger_controller`
    /// - `OppCtrl`           → NOT controlled by `trigger_controller`
    /// - `ActivePlayerCtrl`  → controlled by `active_player` (the player whose
    ///   upkeep/turn it is) — used by The Abyss
    ///
    /// Returns the lowest-`CardId` legal target, or `None` if no legal target
    /// exists (the trigger then does nothing, CR 603.10 / 608.2c).
    fn choose_triggered_destroy_target(
        &self,
        restriction: &crate::core::effects::TargetRestriction,
        trigger_controller: PlayerId,
        active_player: PlayerId,
        trigger_source_colors: &[crate::core::Color],
    ) -> Option<CardId> {
        use crate::core::effects::ControllerRestriction;
        let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
            .battlefield
            .cards
            .iter()
            .filter(|&card_id| {
                let Some(card) = self.cards.try_get(*card_id) else {
                    return false;
                };
                // Type / token / power / nonartifact filters.
                if !restriction.matches(card) {
                    return false;
                }
                // Controller filter (resolved with the real active player).
                let controller_ok = match restriction.controller {
                    ControllerRestriction::Any => true,
                    ControllerRestriction::YouCtrl => card.controller == trigger_controller,
                    ControllerRestriction::OppCtrl => card.controller != trigger_controller,
                    ControllerRestriction::ActivePlayerCtrl => card.controller == active_player,
                };
                controller_ok && targeting::is_legal_target(card, trigger_controller, trigger_source_colors)
            })
            .copied()
            .collect();
        candidates.sort_by_key(|id| id.as_u32());
        candidates.first().copied()
    }

    /// Execute the triggered abilities of `card_id` for `event`, resolving
    /// targets among the `active_player`'s permanents where the ability says so
    /// (e.g. The Abyss's "each player's upkeep" destroy targets the active
    /// player's nonartifact creatures).
    ///
    /// # Errors
    ///
    /// Returns an error if the trigger source card lookup fails or an effect
    /// fails to execute.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_triggers_for_controller(
        &mut self,
        event: TriggerEvent,
        card_id: CardId,
        active_player: PlayerId,
    ) -> Result<()> {
        // Get the card's triggers
        let effects_to_execute: Vec<Effect> = {
            let card = self.cards.get(card_id)?;

            // Only process triggers where the controller matches the active player
            // OR the trigger doesn't have the [controller_only] flag
            card.triggers
                .iter()
                .filter(|trigger| {
                    if trigger.event != event {
                        return false;
                    }
                    // Controller-only triggers should only fire on the controller's turn
                    // OPTIMIZATION: Use pre-parsed boolean flag instead of runtime string check
                    if trigger.controller_turn_only {
                        return card.controller == active_player;
                    }
                    // ValidPlayer$ Player.Chosen (Black Vise): fire only on the
                    // chosen player's turn (and only once a player was chosen).
                    if trigger.chosen_player_turn_only {
                        return card.chosen_player == Some(active_player);
                    }
                    // ValidPlayer$ Player.EnchantedController (Paralyze): fire only
                    // on the upkeep of the ENCHANTED permanent's controller. The
                    // host's controller (not the Aura's) must be the active player.
                    if trigger.enchanted_controller_turn_only {
                        return card
                            .attached_to
                            .and_then(|host| self.cards.try_get(host))
                            .is_some_and(|host| host.controller == active_player);
                    }
                    // Intervening-if condition (CR 603.4): the source must satisfy
                    // its self-state condition right now, or the trigger does not
                    // fire. Howling Mine: "if CARDNAME is untapped, that player
                    // draws an additional card" — a tapped Howling Mine grants no
                    // extra draw.
                    if let Some(cond) = &trigger.present_self_condition {
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
                    true
                })
                .flat_map(|trigger| trigger.effects.clone())
                .collect()
        };

        // Build trigger context for placeholder resolution
        let controller = self.cards.get(card_id)?.controller;
        let mut ctx = TriggerContext::new(card_id, controller);
        // For a beginning-of-draw-step phase trigger (Howling Mine:
        // `Phase$ Draw | ValidPlayer$ Player`), the "triggered player" is the
        // player whose draw step fired the trigger — the active player — NOT the
        // trigger source's controller. Populate `drawing_player` so a
        // `Defined$ TriggeredPlayer` DrawCards resolves to that player (CR 504.2:
        // the active player draws during their own draw step).
        if event == TriggerEvent::BeginningOfDraw {
            ctx = ctx.with_drawing_player(active_player);
        }
        // Colors of the trigger source, needed for protection / legal-target checks.
        let trigger_source_colors: smallvec::SmallVec<[crate::core::Color; 2]> =
            self.cards.get(card_id)?.colors.clone();

        // Execute each effect with placeholder resolution
        for effect in effects_to_execute {
            // Step 1: Apply shared placeholder resolution for simple cases
            let mut effect = resolve_effect_placeholder(&effect, &ctx);

            // Step 2: Handle complex targeting that requires battlefield search
            match &effect {
                // Targeted destroy fired by an "each player's upkeep" style trigger
                // (e.g. The Abyss: `T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player`
                // with `DB$ Destroy | ValidTgts$ Creature.nonArtifact+ActivePlayerCtrl`).
                // The trigger fires for whichever player's upkeep it is, so the target
                // must be chosen among the ACTIVE player's permanents. Without this arm
                // the placeholder target was never resolved and the destroy silently
                // fizzled (CR 603 / CR 701.7).
                Effect::DestroyPermanent {
                    target,
                    restriction,
                    no_regenerate,
                } if target.is_placeholder() => {
                    if let Some(target_id) = self.choose_triggered_destroy_target(
                        restriction,
                        controller,
                        active_player,
                        &trigger_source_colors,
                    ) {
                        effect = Effect::DestroyPermanent {
                            target: target_id,
                            restriction: restriction.clone(),
                            no_regenerate: *no_regenerate,
                        };
                    } else {
                        // No legal target — the triggered ability does nothing
                        // (CR 603.10 / 608.2c). Skip to avoid a fizzling log line.
                        continue;
                    }
                }
                // "Each player's upkeep, deal damage to that player equal to a
                // count of their own permanents/cards" (Karma, Black Vise). The
                // target player and the count are both resolved against the same
                // player: the active player whose upkeep fired (target_self =
                // false), or the trigger source's controller (target_self = true,
                // for Defined$ You variable punishers). Reuses the shared
                // evaluate_count_expression so ActivePlayerCtrl / YouCtrl filters
                // count the right player's permanents. Resolves to a concrete
                // DealDamage so the logging + execute_effect path below is shared.
                Effect::DealDamageToTriggeredPlayer { count, target_self } => {
                    let target_player = if *target_self { controller } else { active_player };
                    let amount = self.evaluate_count_expression(count, target_player)?.max(0);
                    if amount == 0 {
                        // No damage to deal — skip to avoid a "deals 0 damage" line
                        // (CR 120.8: a source dealing 0 damage isn't dealing damage).
                        continue;
                    }
                    effect = Effect::DealDamage {
                        target: TargetRef::Player(target_player),
                        amount,
                    };
                }
                Effect::Earthbend { target, num_counters } if target.is_placeholder() => {
                    // Placeholder CardId 0 means we need to target a land the controller controls
                    let land_target = self
                        .battlefield
                        .cards
                        .iter()
                        .filter_map(|cid| {
                            let card = self.cards.get(*cid).ok()?;
                            if card.controller == controller && card.is_land() {
                                Some(*cid)
                            } else {
                                None
                            }
                        })
                        .next();

                    if let Some(land_id) = land_target {
                        effect = Effect::Earthbend {
                            target: land_id,
                            num_counters: *num_counters,
                        };
                    } else {
                        // No valid land target - skip this trigger
                        continue;
                    }
                }
                // Paralyze's optional pay-{4}-to-untap upkeep trigger:
                // UnlessCostWrapper { UntapPermanent { placeholder }, .. }. The
                // untap target is the ENCHANTED permanent (Defined$ Enchanted) —
                // resolve it from the Aura's `attached_to`. The payer
                // (UnlessPayer$ EnchantedController) is that permanent's
                // controller. The trigger path does NOT go through
                // resolve_effect_target (the spell-resolution payer resolver),
                // so resolve both the target and the payer here.
                Effect::UnlessCostWrapper {
                    inner_effect,
                    unless_cost,
                } if matches!(
                    inner_effect.as_ref(),
                    Effect::UntapPermanent { target } if target.is_placeholder()
                ) =>
                {
                    let Some(host_id) = self.cards.get(card_id)?.attached_to else {
                        // Aura no longer attached — nothing to untap.
                        continue;
                    };
                    let Some(host_controller) = self.cards.try_get(host_id).map(|h| h.controller) else {
                        continue;
                    };
                    effect = Effect::UnlessCostWrapper {
                        inner_effect: Box::new(Effect::UntapPermanent { target: host_id }),
                        unless_cost: crate::core::effects::UnlessCost {
                            cost: unless_cost.cost.clone(),
                            // Store the resolved payer as a numeric id string, the
                            // form the UnlessCostWrapper executor parses.
                            payer: host_controller.as_u32().to_string(),
                            switched: unless_cost.switched,
                        },
                    };
                }
                _ => {}
            }

            // Log the trigger effect
            if let Some(card) = self.cards.try_get(card_id) {
                let card_name = card.name.clone();
                let message = match &effect {
                    Effect::DealDamage {
                        target: TargetRef::Player(player_id),
                        amount,
                    } => {
                        let player_name = self
                            .get_player(*player_id)
                            .map(|p| p.name.as_str().to_string())
                            .unwrap_or_else(|_| "player".to_string());
                        format!("{} deals {} damage to {}", card_name, amount, player_name)
                    }
                    Effect::GainLife { player, amount } => {
                        let player_name = self
                            .get_player(*player)
                            .map(|p| p.name.as_str().to_string())
                            .unwrap_or_else(|_| "player".to_string());
                        format!("{} causes {} to gain {} life", card_name, player_name, amount)
                    }
                    _ => format!("{} trigger effect", card_name),
                };
                self.logger.normal(&message);
            }

            self.execute_effect(&effect)?;
        }

        Ok(())
    }

    /// Check and execute SpellCast triggers when a spell is cast
    ///
    /// This handles "Whenever you cast a [noncreature] spell" triggers like:
    /// - Boar-q-pine: Whenever you cast a noncreature spell, put a +1/+1 counter on this creature
    /// - Prowess: Whenever you cast a noncreature spell, this creature gets +1/+1 until end of turn
    ///
    /// MTG Rules 601.2i: The spell becomes cast after costs are paid, triggering these abilities.
    ///
    /// # Parameters
    /// - `cast_spell_id`: The spell that was just cast (used to check if it's noncreature)
    /// - `caster_id`: The player who cast the spell (triggers only fire for spells cast by the controller)
    ///
    /// # Errors
    ///
    /// Returns an error if trigger effects fail to resolve.
    pub fn check_spellcast_triggers(&mut self, cast_spell_id: CardId, caster_id: PlayerId) -> Result<()> {
        use crate::core::Trigger;

        // Increment spell cast counter for the caster.
        //
        // Logged via `SetSpellsCastThisTurn` so that undo / rewind can
        // restore the previous count. Without the undo entry, the WASM
        // rewind/replay verifier sees a `players[].spells_cast_this_turn`
        // drift across rewinds (the value monotonically grows on every
        // forward pass but is never decremented on rollback).
        let prior_log_size = self.logger.log_count();
        let mut new_count = None;
        if let Ok(player) = self.get_player_mut(caster_id) {
            let old_value = player.spells_cast_this_turn;
            player.spells_cast_this_turn = old_value.saturating_add(1);
            new_count = Some((old_value, player.spells_cast_this_turn));
        }
        if let Some((old_value, new_value)) = new_count {
            self.undo_log.log(
                crate::undo::GameAction::SetSpellsCastThisTurn {
                    player_id: caster_id,
                    old_value,
                    new_value,
                },
                prior_log_size,
            );
        }

        // Check cast-spell type flags (for SpellCast trigger filtering)
        let is_creature_spell = self.cards.get(cast_spell_id).map(|c| c.is_creature()).unwrap_or(false);
        let is_instant_or_sorcery = self
            .cards
            .get(cast_spell_id)
            .map(|c| c.is_instant() || c.is_sorcery())
            .unwrap_or(false);

        // Collect SpellCast triggers from permanents on the battlefield
        // These triggers fire when their controller casts a spell
        struct TriggerToExecute {
            source_card_id: CardId,
            controller: PlayerId,
            source_name: String,
            effects: Vec<Effect>,
            description: String,
        }

        let triggers_to_execute: Vec<TriggerToExecute> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Some(card) = self.cards.try_get(card_id) {
                    // Only trigger for permanents controlled by the caster
                    if card.controller != caster_id {
                        return None;
                    }

                    // Find SpellCast triggers on this permanent
                    let matching_triggers: Vec<&Trigger> = card
                        .triggers
                        .iter()
                        .filter(|trigger| {
                            if trigger.event != TriggerEvent::SpellCast {
                                return false;
                            }

                            // Check noncreature-only triggers using pre-parsed flag
                            // OPTIMIZATION: Use boolean flag instead of runtime .contains()
                            if trigger.requires_noncreature && is_creature_spell {
                                return false;
                            }

                            // Check instant-or-sorcery-only triggers
                            if trigger.requires_instant_or_sorcery && !is_instant_or_sorcery {
                                return false;
                            }

                            true
                        })
                        .collect();

                    if matching_triggers.is_empty() {
                        None
                    } else {
                        Some(
                            matching_triggers
                                .into_iter()
                                .map(|trigger| TriggerToExecute {
                                    source_card_id: card_id,
                                    controller: card.controller,
                                    source_name: card.name.to_string(),
                                    effects: trigger.effects.clone(),
                                    description: trigger.description.clone(),
                                })
                                .collect::<Vec<_>>(),
                        )
                    }
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        // Execute each trigger's effects
        for trigger in triggers_to_execute {
            // Log the trigger
            self.logger
                .gamelog(&format!("Trigger: {} - {}", trigger.source_name, trigger.description));

            // Build trigger context
            let ctx = TriggerContext::new(trigger.source_card_id, trigger.controller);

            // Execute effects with placeholder resolution
            for effect in trigger.effects {
                // Apply shared placeholder resolution
                let mut resolved_effect = resolve_effect_placeholder(&effect, &ctx);

                // SpellCast triggers with a PumpCreature whose target is still a
                // placeholder always mean "pump the source permanent" (Prowess).
                // The generic `resolve_effect_placeholder` leaves PumpCreature
                // unresolved because in other trigger contexts it is ambiguous
                // (e.g. ETB triggers need to find a chosen target).  Here the
                // context is unambiguous: the SpellCast trigger fires on behalf of
                // `source_card_id`, so CardId::placeholder() → source_card_id.
                if let Effect::PumpCreature {
                    target,
                    power_bonus,
                    toughness_bonus,
                    keywords_granted,
                } = &resolved_effect
                {
                    if target.is_placeholder() {
                        resolved_effect = Effect::PumpCreature {
                            target: trigger.source_card_id,
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                            keywords_granted: keywords_granted.clone(),
                        };
                    }
                }

                // Log specific effects with custom messages
                // Wildcard is intentional: only PutCounter and PumpCreature need special logging
                #[allow(clippy::wildcard_enum_match_arm)]
                match &resolved_effect {
                    Effect::PutCounter {
                        target,
                        amount,
                        counter_type,
                    } if *target == trigger.source_card_id => {
                        let current_counters = self
                            .cards
                            .get(trigger.source_card_id)
                            .map(|c| c.get_counter(*counter_type))
                            .unwrap_or(0);
                        self.logger.normal(&format!(
                            "{} gets a {} counter (now {} counters)",
                            trigger.source_name,
                            counter_type,
                            current_counters + amount
                        ));
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                        ..
                    } if *target == trigger.source_card_id => {
                        self.logger.normal(&format!(
                            "{} gets +{}/+{} until end of turn",
                            trigger.source_name, power_bonus, toughness_bonus
                        ));
                    }
                    _ => {}
                }

                self.execute_effect(&resolved_effect)?;
            }
        }

        // Check delayed triggers with SpellCast condition
        // These fire when matching spells are cast (e.g., Jeong Jeong's "When you next cast a Lesson spell")
        self.check_delayed_spellcast_triggers(cast_spell_id, caster_id)?;

        Ok(())
    }

    /// Check and execute delayed SpellCast triggers when a spell is cast
    ///
    /// This handles delayed triggers created by effects like Jeong Jeong:
    /// "When you next cast a Lesson spell this turn, copy it"
    ///
    /// Unlike permanent triggers (which fire repeatedly), delayed triggers fire once
    /// and are removed after firing.
    fn check_delayed_spellcast_triggers(&mut self, cast_spell_id: CardId, caster_id: PlayerId) -> Result<()> {
        // Get the spell's types for matching
        let spell_types: smallvec::SmallVec<[String; 4]> = {
            if let Some(card) = self.cards.try_get(cast_spell_id) {
                // Collect subtypes (like "Lesson", "Human", etc.) and card types (like "Sorcery", "Creature")
                card.subtypes
                    .iter()
                    .map(|st| st.to_string())
                    .chain(card.types.iter().map(|ct| format!("{:?}", ct)))
                    .collect()
            } else {
                return Ok(()); // Card doesn't exist
            }
        };

        // Convert to &str slices for matching
        let spell_type_refs: smallvec::SmallVec<[&str; 4]> = spell_types.iter().map(String::as_str).collect();

        // Find delayed triggers that match this spell cast
        // Use get_matching_ids helper since DelayedTriggerStore doesn't expose iter()
        let matching_trigger_ids: Vec<crate::core::DelayedTriggerId> = self
            .delayed_triggers
            .get_matching_spellcast_trigger_ids(caster_id, &spell_type_refs);

        // Fire and remove matching triggers
        for trigger_id in matching_trigger_ids {
            // Remove the trigger (it fires once)
            if let Some(mut trigger) = self.delayed_triggers.remove(trigger_id) {
                // Update tracked_card to the spell being copied (for CopySpellAbility)
                trigger.tracked_card = cast_spell_id;

                // Log the trigger fire
                let spell_name = self
                    .cards
                    .get(cast_spell_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                self.logger.gamelog(&format!(
                    "Delayed trigger fires: spell {} triggers copy effect",
                    spell_name
                ));

                // Execute the trigger
                self.fire_delayed_trigger(trigger)?;
            }
        }

        Ok(())
    }

    /// Check and execute death triggers for a creature that is dying
    ///
    /// Called BEFORE the creature is moved to the graveyard, so its triggers
    /// are still accessible. This handles "When CARDNAME dies" triggers like Su-Chi.
    ///
    /// MTG Rules 603.6c: Triggered abilities look back in time to determine if
    /// the event occurred. Death triggers trigger when a creature moves from
    /// battlefield to graveyard.
    ///
    /// Note: Wildcard match is intentional - only AddMana effects need player
    /// resolution; others execute as-is.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found or effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_death_triggers(&mut self, dying_card_id: CardId) -> Result<()> {
        // Get the card's triggers and controller while it's still on battlefield
        let (effects_to_execute, controller): (Vec<Effect>, PlayerId) = {
            let card = self.cards.get(dying_card_id)?;

            // Collect LeavesBattlefield triggers (which we use for "dies" events)
            let effects: Vec<Effect> = card
                .triggers
                .iter()
                .filter(|trigger| trigger.event == TriggerEvent::LeavesBattlefield)
                .flat_map(|trigger| trigger.effects.clone())
                .collect();

            (effects, card.controller)
        };

        if !effects_to_execute.is_empty() {
            // Log the trigger (official game action)
            if let Some(card) = self.cards.try_get(dying_card_id) {
                for trigger in &card.triggers {
                    if trigger.event == TriggerEvent::LeavesBattlefield {
                        self.logger
                            .gamelog(&format!("Trigger: {} - {}", card.name, trigger.description));
                    }
                }
            }

            // Build trigger context for placeholder resolution
            let ctx = TriggerContext::new(dying_card_id, controller);

            // Execute each effect with placeholder resolution
            for effect in effects_to_execute {
                let effect = resolve_effect_placeholder(&effect, &ctx);

                // Log AddMana effects specially (Su-Chi death trigger)
                if let Effect::AddMana { .. } = &effect {
                    if let Some(card) = self.cards.try_get(dying_card_id) {
                        let player_name = self
                            .get_player(controller)
                            .map(|p| p.name.as_str().to_string())
                            .unwrap_or_else(|_| "player".to_string());
                        self.logger.gamelog(&format!(
                            "{} dies, {} adds mana to {}'s pool",
                            card.name, card.name, player_name
                        ));
                    }
                }

                self.execute_effect(&effect)?;
            }
        }

        // Check equipment on the battlefield that was attached to the dying creature
        // for EquippedCreatureDies triggers (e.g., Skullclamp "draw two cards")
        let equipment_triggers: Vec<(CardId, PlayerId, Vec<Effect>, String)> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&equip_id| {
                let equip = self.cards.try_get(equip_id)?;
                if !equip.is_equipment() || equip.attached_to != Some(dying_card_id) {
                    return None;
                }
                let effects: Vec<Effect> = equip
                    .triggers
                    .iter()
                    .filter(|t| t.event == TriggerEvent::EquippedCreatureDies)
                    .flat_map(|t| t.effects.clone())
                    .collect();
                if effects.is_empty() {
                    return None;
                }
                let desc = equip
                    .triggers
                    .iter()
                    .find(|t| t.event == TriggerEvent::EquippedCreatureDies)
                    .map(|t| t.description.clone())
                    .unwrap_or_default();
                Some((equip_id, equip.controller, effects, desc))
            })
            .collect();

        for (equip_id, equip_controller, effects, desc) in equipment_triggers {
            // Log the trigger
            if let Some(equip) = self.cards.try_get(equip_id) {
                self.logger.gamelog(&format!("Trigger: {} - {}", equip.name, desc));
            }

            let ctx = TriggerContext::new(equip_id, equip_controller);
            for effect in effects {
                let effect = resolve_effect_placeholder(&effect, &ctx);
                self.execute_effect(&effect)?;
            }
        }

        // Check `DamagedCreatureDies` triggers (Sengir Vampire et al.):
        // for any battlefield permanent whose CardId appears in the dying
        // card's `damaged_by_this_turn` list, fire that permanent's matching
        // trigger with itself as `Defined$ Self`.
        //
        // Snapshot the dying card's damage-source list first to avoid holding
        // a borrow during the trigger-execution loop. Iterating the
        // battlefield in CardId order keeps the trigger fire ordering
        // deterministic.
        let damage_sources: smallvec::SmallVec<[CardId; 2]> = self
            .cards
            .try_get(dying_card_id)
            .map(|c| c.damaged_by_this_turn.clone())
            .unwrap_or_default();

        if !damage_sources.is_empty() {
            let damaged_dies_triggers: Vec<(CardId, PlayerId, Vec<Effect>, String)> = self
                .battlefield
                .cards
                .iter()
                .filter_map(|&source_id| {
                    if !damage_sources.contains(&source_id) {
                        return None;
                    }
                    let source = self.cards.try_get(source_id)?;
                    let effects: Vec<Effect> = source
                        .triggers
                        .iter()
                        .filter(|t| t.event == TriggerEvent::DamagedCreatureDies)
                        .flat_map(|t| t.effects.clone())
                        .collect();
                    if effects.is_empty() {
                        return None;
                    }
                    let desc = source
                        .triggers
                        .iter()
                        .find(|t| t.event == TriggerEvent::DamagedCreatureDies)
                        .map(|t| t.description.clone())
                        .unwrap_or_default();
                    Some((source_id, source.controller, effects, desc))
                })
                .collect();

            for (source_id, source_controller, effects, desc) in damaged_dies_triggers {
                if let Some(source) = self.cards.try_get(source_id) {
                    self.logger.gamelog(&format!("Trigger: {} - {}", source.name, desc));
                }
                let ctx = TriggerContext::new(source_id, source_controller);
                for effect in effects {
                    let effect = resolve_effect_placeholder(&effect, &ctx);
                    self.execute_effect(&effect)?;
                }
            }
        }

        Ok(())
    }

    /// Check and execute "card drawn" triggers for all permanents on the battlefield
    ///
    /// Called after each card is drawn. Handles "When you draw your Nth card each turn"
    /// triggers like Knowledge Seeker ("When you draw your second card each turn, put
    /// a +1/+1 counter on Knowledge Seeker") and Otter-Penguin.
    ///
    /// MTG Rules 603.2a: Draw triggers look at what card was drawn and which player drew.
    ///
    /// # Parameters
    /// - `drawing_player`: The player who drew the card
    /// - `draw_number`: Which draw this was this turn (1 = first, 2 = second, etc.)
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    pub fn check_card_drawn_triggers(&mut self, drawing_player: PlayerId, draw_number: u8) -> Result<()> {
        use smallvec::SmallVec;

        // Fast path: Most games have no CardDrawn triggers, so check first before allocating
        // Scan all permanents on battlefield for CardDrawn triggers
        let battlefield_cards: SmallVec<[CardId; 32]> = self.battlefield.cards.iter().copied().collect();

        struct TriggerInfo {
            card_id: CardId,
            controller: PlayerId,
            card_name: String,
            description: String,
            effects: SmallVec<[Effect; 2]>,
        }

        let mut triggers_to_fire: SmallVec<[TriggerInfo; 2]> = SmallVec::new();

        for card_id in battlefield_cards {
            let Ok(card) = self.cards.get(card_id) else { continue };

            for trigger in &card.triggers {
                if trigger.event != TriggerEvent::CardDrawn {
                    continue;
                }

                // Check if this trigger fires for the current draw
                // 1. If trigger has a draw_number requirement, check it matches
                if let Some(required_draw_num) = trigger.draw_number {
                    if draw_number != required_draw_num {
                        continue;
                    }
                }

                // 2. Check if the drawing player matches trigger's target
                // triggers_on_controller_draw = true: fires when card's controller draws
                // triggers_on_controller_draw = false: fires when opponent draws
                let controller_drew = drawing_player == card.controller;
                let should_fire = if trigger.triggers_on_controller_draw {
                    controller_drew
                } else {
                    !controller_drew
                };

                if !should_fire {
                    continue;
                }

                // This trigger should fire - collect its info
                triggers_to_fire.push(TriggerInfo {
                    card_id,
                    controller: card.controller,
                    card_name: card.name.to_string(),
                    description: trigger.description.clone(),
                    effects: SmallVec::from_iter(trigger.effects.iter().cloned()),
                });
            }
        }

        if triggers_to_fire.is_empty() {
            return Ok(());
        }

        // Execute triggers (we've released the borrow on cards)
        for trigger_info in triggers_to_fire {
            // Log the trigger (official game action)
            self.logger.gamelog(&format!(
                "Trigger: {} - {}",
                trigger_info.card_name, trigger_info.description
            ));

            // Build trigger context with drawing_player for DealDamage resolution
            let ctx =
                TriggerContext::new(trigger_info.card_id, trigger_info.controller).with_drawing_player(drawing_player);

            for effect in trigger_info.effects {
                // Apply shared placeholder resolution first
                let mut resolved_effect = resolve_effect_placeholder(&effect, &ctx);

                // PumpCreature with placeholder CardId::new(0) → "self" for CardDrawn triggers
                // (Otter-Penguin: "this creature gets +1/+2")
                if let Effect::PumpCreature {
                    target,
                    power_bonus,
                    toughness_bonus,
                    keywords_granted,
                } = &resolved_effect
                {
                    if target.is_placeholder() {
                        resolved_effect = Effect::PumpCreature {
                            target: trigger_info.card_id,
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                            keywords_granted: keywords_granted.clone(),
                        };
                    }
                }

                // DebuffCreature with placeholder → "self" for triggers
                if let Effect::DebuffCreature {
                    target,
                    keywords_removed,
                } = &resolved_effect
                {
                    if target.is_placeholder() {
                        resolved_effect = Effect::DebuffCreature {
                            target: trigger_info.card_id,
                            keywords_removed: keywords_removed.clone(),
                        };
                    }
                }

                // GrantCantBeBlocked with placeholder CardId::new(0) → "self" for CardDrawn triggers
                // (Otter-Penguin: "can't be blocked this turn" via SubAbility$ chain)
                if let Effect::GrantCantBeBlocked { target } = &resolved_effect {
                    if target.is_placeholder() {
                        resolved_effect = Effect::GrantCantBeBlocked {
                            target: trigger_info.card_id,
                        };
                    }
                }

                self.execute_effect(&resolved_effect)?;
            }
        }

        Ok(())
    }

    /// Check and execute discard triggers for all battlefield permanents.
    ///
    /// Called from `discard_card` after the card moves to the graveyard.
    /// Handles "Whenever you discard a card" triggers like Monument to Endurance.
    ///
    /// CR 603.1: Whenever the trigger event occurs, the trigger fires. Here the
    /// trigger checks that the discarding player is the controller of the
    /// triggering permanent (`ValidCard$ Card.YouOwn` semantics).
    ///
    /// # Arguments
    ///
    /// - `discarding_player`: The player who discarded
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    pub fn check_card_discarded_triggers(&mut self, discarding_player: PlayerId) -> Result<()> {
        use smallvec::SmallVec;

        // Fast path: check if any battlefield permanent has a CardDiscarded trigger
        let battlefield_cards: SmallVec<[CardId; 32]> = self.battlefield.cards.iter().copied().collect();

        struct TriggerInfo {
            card_id: CardId,
            controller: PlayerId,
            card_name: String,
            description: String,
            effects: SmallVec<[Effect; 2]>,
        }

        let mut triggers_to_fire: SmallVec<[TriggerInfo; 2]> = SmallVec::new();

        for card_id in battlefield_cards {
            let Ok(card) = self.cards.get(card_id) else {
                continue;
            };

            for trigger in &card.triggers {
                if trigger.event != TriggerEvent::CardDiscarded {
                    continue;
                }

                // CR 603.1 + ValidCard$ Card.YouOwn: trigger fires only when the
                // permanent's controller is the one discarding the card.
                if card.controller != discarding_player {
                    continue;
                }

                triggers_to_fire.push(TriggerInfo {
                    card_id,
                    controller: card.controller,
                    card_name: card.name.to_string(),
                    description: trigger.description.clone(),
                    effects: SmallVec::from_iter(trigger.effects.iter().cloned()),
                });
            }
        }

        if triggers_to_fire.is_empty() {
            return Ok(());
        }

        // Execute triggers (released borrow on cards)
        for trigger_info in triggers_to_fire {
            self.logger.gamelog(&format!(
                "Trigger: {} - {}",
                trigger_info.card_name, trigger_info.description
            ));

            let ctx = TriggerContext::new(trigger_info.card_id, trigger_info.controller);

            for effect in trigger_info.effects {
                let resolved_effect = resolve_effect_placeholder(&effect, &ctx);
                // ModalChoice effects (e.g. Monument to Endurance's Charm) cannot be
                // handled via execute_effect (which skips them with a warning). Instead,
                // auto-pick the first mode and execute its effect — this is a simplification
                // that satisfies AI-vs-AI play. Full player-choice integration requires the
                // priority loop and is tracked as TODO (mtg-821).
                if let Effect::ModalChoice { ref modes, .. } = resolved_effect {
                    if let Some(first_mode) = modes.first() {
                        let mode_effect = resolve_effect_placeholder(&first_mode.effect, &ctx);
                        self.execute_effect(&mode_effect)?;
                    }
                } else {
                    self.execute_effect(&resolved_effect)?;
                }
            }
        }

        Ok(())
    }

    /// Check and execute attack triggers for an attacking creature
    ///
    /// Called after each attacker is declared. Handles "Whenever this creature attacks"
    /// triggers like Firebending, which add combat mana.
    ///
    /// MTG Rules 508.1m: Abilities that trigger on declaring attackers go on the stack.
    ///
    /// Note: Wildcard match is intentional - only AddMana/Firebend effects need player
    /// resolution; others execute as-is.
    ///
    /// # Errors
    ///
    /// Returns an error if the attacker card is not found or effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_attack_triggers(&mut self, attacker_id: CardId, _active_player: PlayerId) -> Result<()> {
        use smallvec::SmallVec;

        // Fast path: Check if card has any attack triggers BEFORE any allocation
        // Most cards have no triggers at all, so this skips all work in the common case
        let has_attack_triggers = {
            let card = self.cards.get(attacker_id)?;
            card.triggers.iter().any(|t| t.event == TriggerEvent::Attacks)
        };

        if !has_attack_triggers {
            return Ok(());
        }

        // Slow path: Card has attack triggers - extract data and execute
        // Use SmallVec to avoid heap allocation (most triggers have 1-2 effects)
        struct TriggerData {
            card_name: String,
            description: String,
            effects: SmallVec<[Effect; 2]>,
            optional: bool,
            cost: Option<crate::core::Cost>,
        }

        let (controller, creature_power, triggers): (PlayerId, u8, SmallVec<[TriggerData; 1]>) = {
            let card = self.cards.get(attacker_id)?;
            let power = card.current_power().max(0) as u8;

            let mut triggers: SmallVec<[TriggerData; 1]> = SmallVec::new();
            for trigger in &card.triggers {
                if trigger.event == TriggerEvent::Attacks {
                    triggers.push(TriggerData {
                        card_name: card.name.to_string(),
                        description: trigger.description.clone(),
                        effects: SmallVec::from_iter(trigger.effects.iter().cloned()),
                        optional: trigger.optional,
                        cost: trigger.cost.clone(),
                    });
                }
            }

            (card.controller, power, triggers)
        };

        // Process each trigger - borrow is released, safe to call execute_effect
        for trigger_data in triggers {
            // For optional triggers with costs, check if cost can be paid
            let mut sacrifice_target: Option<CardId> = None;
            let mut sacrificed_power: u8 = 0;

            if trigger_data.optional {
                if let Some(ref cost) = trigger_data.cost {
                    // Check if sacrifice cost can be paid
                    if let Some((count, pattern)) = cost.get_sacrifice_pattern() {
                        if !self.can_pay_sacrifice_pattern(pattern, count, attacker_id, controller) {
                            log::debug!(
                                "Skipping optional attack trigger on {} - sacrifice cost not payable (need {} {})",
                                trigger_data.card_name,
                                count,
                                pattern
                            );
                            continue; // Skip this trigger - can't pay cost
                        }

                        // Choose which permanent to sacrifice (AI heuristic: pick lowest P/T creature)
                        sacrifice_target = self.choose_sacrifice_target(pattern, attacker_id, controller);

                        // Get the power of the creature we're about to sacrifice
                        if let Some(sac_id) = sacrifice_target {
                            if let Ok(sac_card) = self.cards.get(sac_id) {
                                sacrificed_power = sac_card.current_power().max(0) as u8;
                            }
                        }
                    }
                    // TODO: Check other cost types (mana, life, etc.)
                }
            }

            // Log the trigger (official game action)
            self.logger.gamelog(&format!(
                "Trigger: {} - {}",
                trigger_data.card_name, trigger_data.description
            ));

            // Execute sacrifice cost first (if any)
            if let Some(sac_target) = sacrifice_target {
                if let Ok(sac_card) = self.cards.get(sac_target) {
                    let sac_name = sac_card.name.to_string();
                    let sac_owner = sac_card.owner;
                    log::info!(
                        "Sacrificing {} ({}) for attack trigger cost",
                        sac_name,
                        sac_target.as_u32()
                    );

                    self.logger
                        .gamelog(&format!("Sacrifices {} for trigger cost", sac_name));

                    // Move from battlefield to graveyard (or exile if finality counter)
                    let sac_dest = self.death_destination_for_card(sac_target);
                    self.move_card(sac_target, Zone::Battlefield, sac_dest, sac_owner)?;

                    // Check sacrifice triggers (e.g., Pirate Peddlers)
                    self.check_triggers(TriggerEvent::Sacrificed, sac_target)?;
                }
            }

            // Build trigger context with creature power for firebend resolution
            let ctx = TriggerContext::new(attacker_id, controller)
                .with_creature_power(creature_power)
                .with_sacrificed_power(sacrificed_power);

            // Execute each effect with placeholder resolution
            for effect in trigger_data.effects {
                // Apply shared placeholder resolution
                let effect = resolve_effect_placeholder(&effect, &ctx);

                // Log firebend effects
                if let Effect::Firebend { amount, .. } = &effect {
                    if *amount > 0 {
                        self.logger.gamelog(&format!(
                            "{} triggers Firebending {} (adding {} {{R}} to combat mana)",
                            trigger_data.card_name, amount, amount
                        ));
                    }
                }

                self.execute_effect(&effect)?;
            }
        }

        Ok(())
    }

    /// Deal damage to a player target
    ///
    /// # Errors
    ///
    /// Returns an error if the target player does not exist.
    /// Apply any source-filtered damage-prevention shields on `target_id` to a
    /// would-be damage event of `amount` from `source` (CR 615.1, 615.6).
    ///
    /// Returns the damage remaining after prevention. Matching shields are
    /// consumed (a `NextEvent` shield prevents the whole event then expires);
    /// spent shields are removed eagerly. Prevention is logged. Uses only
    /// public card colors, so it is identical on server and client.
    ///
    /// `source == None` means the damage has no tracked source (e.g. a few
    /// internal/test paths) and no source-filtered shield can match.
    /// Capture and log a player's current source-prevention-shield list so a
    /// per-action undo can restore it (mtg-ba6uq #6). Call BEFORE installing or
    /// consuming shields. The list is `#[serde]`-hashed but turn-start rewind
    /// blanket-clears it; this covers the per-action UndoTest / human / MCTS path.
    fn log_source_prevention_shields(&mut self, player_id: PlayerId) {
        let prev = self
            .get_player(player_id)
            .map(|p| p.source_prevention_shields.clone())
            .unwrap_or_default();
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetSourcePreventionShields { player_id, prev },
            prior_log_size,
        );
    }

    /// Capture and log a player's current `combat_mana_pool` so a per-action
    /// undo can restore it (mtg-ba6uq #7). Call BEFORE adding, spending, or
    /// emptying combat mana. Turn-start rewind blanket-clears it; this covers the
    /// per-action UndoTest / human / MCTS path. `ManaPool` is `Copy`.
    pub(crate) fn log_combat_mana_pool(&mut self, player_id: PlayerId) {
        let prev = self.get_player(player_id).map(|p| p.combat_mana_pool).unwrap_or(None);
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetCombatManaPool { player_id, prev },
            prior_log_size,
        );
    }

    /// Snapshot a player's REGULAR `mana_pool` for undo BEFORE a payment
    /// consumes it (mtg-733). `pay_from_total_mana` / `ManaPool::pay_cost`
    /// mutate the pool with no other covering action, so a per-action (MCTS /
    /// human / UndoTest) partial rewind stopping between an `AddMana` and its
    /// consuming payment would otherwise observe the wrong pool. Mirrors
    /// `log_combat_mana_pool` (mtg-ba6uq #7). Cheap: `ManaPool` is `Copy`.
    pub(crate) fn log_mana_pool(&mut self, player_id: PlayerId) {
        let Some(prev) = self.get_player(player_id).ok().map(|p| p.mana_pool) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log
            .log(crate::undo::GameAction::SetManaPool { player_id, prev }, prior_log_size);
    }

    /// Snapshot a creature's marked `damage` for undo BEFORE a `card.damage +=`
    /// mutation (`deal_damage_to_creature` — e.g. Triskelion's ping — and
    /// `Effect::DamageAll`) (mtg-728 sig-2f). Marked damage was applied with no
    /// covering GameAction, so a mid-turn rewind+replay (network/WASM blocking;
    /// per-action MCTS/human undo) left it STALE and replay DOUBLE-applied it.
    /// `undo()` restores the captured value. No-op if the card is missing.
    pub(crate) fn log_damage(&mut self, card_id: CardId) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.damage) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log
            .log(crate::undo::GameAction::SetDamage { card_id, prev }, prior_log_size);
    }

    /// Set a card's `x_paid` (the chosen X for an X-spell/ability, CR 107.3),
    /// snapshotting the prior value for undo FIRST (mtg-728 sig-2g). `x_paid`
    /// was overwritten in the priority loop with no covering GameAction, so a
    /// mid-turn rewind+replay left the chosen X stale on the card (robots42
    /// within-side "cards[N].x_paid changed across rewinds" REWIND/REPLAY FATAL).
    /// No-op if the card is missing.
    pub(crate) fn set_x_paid_logged(&mut self, card_id: CardId, x_value: u8) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.x_paid) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log
            .log(crate::undo::GameAction::SetXPaid { card_id, prev }, prior_log_size);
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.x_paid = x_value;
        }
    }

    fn apply_source_prevention_shields(&mut self, target_id: PlayerId, source: Option<CardId>, amount: i32) -> i32 {
        let Some(source) = source else { return amount };
        if amount <= 0 {
            return amount;
        }
        // No shields installed ⇒ nothing to prevent and nothing to undo.
        if self
            .get_player(target_id)
            .map(|p| p.source_prevention_shields.is_empty())
            .unwrap_or(true)
        {
            return amount;
        }
        // Snapshot the shield list for undo BEFORE consuming/retiring shields
        // (mtg-ba6uq #6).
        self.log_source_prevention_shields(target_id);
        // Snapshot the source's colors up front (immutable borrow) so the
        // shield closure does not borrow the card store while we mutate the
        // player's shield list.
        let source_colors = self.cards.get(source).map(|c| c.colors.clone()).unwrap_or_default();
        let Ok(player) = self.get_player_mut(target_id) else {
            return amount;
        };

        let mut remaining = amount as u32;
        let mut total_prevented = 0u32;
        for shield in &mut player.source_prevention_shields {
            if remaining == 0 {
                break;
            }
            let prevented = shield.apply(source, remaining, |c| source_colors.contains(&c));
            remaining -= prevented;
            total_prevented += prevented;
        }
        // Drop spent shields (NextEvent shields that fired).
        player.source_prevention_shields.retain(|s| !s.is_spent());

        if total_prevented > 0 {
            let player_name = player.name.to_string();
            let source_name = self.cards.get(source).map(|c| c.name.to_string()).unwrap_or_default();
            self.logger.gamelog(&format!(
                "Prevented {} damage to {} from {} ({})",
                total_prevented, player_name, source_name, source
            ));
        }
        remaining as i32
    }

    /// Accumulate damage dealt by the current non-combat damage source into the
    /// per-resolution running total ([`GameState::damage_dealt_by_source`]),
    /// used to fire the "whenever ~ deals damage" trigger once at the end of the
    /// resolution (CR 119.3 lifelink-style aggregation, Spirit Link).
    ///
    /// A no-op when no resolution is in progress (`damage_dealt_by_source` is
    /// `None`) — in particular combat damage is applied with no active source
    /// accumulator, so it never double-fires here; combat fires the same
    /// trigger via its own per-creature path in `resolve_combat_damage`.
    fn accumulate_source_damage(&mut self, amount: i32) {
        if let Some(total) = self.damage_dealt_by_source.as_mut() {
            *total += amount;
        }
    }

    /// Get the noncombat damage modifier for a target controller (e.g. Artist's Talent Level 3)
    pub fn get_noncombat_damage_modifier(&self, target_controller: PlayerId) -> i32 {
        let mut modifier = 0;
        if let Some(source_id) = self.current_damage_source {
            if let Ok(source_card) = self.cards.get(source_id) {
                let source_controller = source_card.controller;
                if target_controller != source_controller {
                    // Check for Artist's Talent level 3
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.name.as_str() == "Artist's Talent"
                                && card.controller == source_controller
                                && card.get_counter(crate::core::CounterType::Level) >= 3
                            {
                                modifier += 2;
                            }
                        }
                    }
                }
            }
        }
        modifier
    }

    /// Deal damage to a player target
    ///
    /// # Errors
    ///
    /// Returns an error if the target player does not exist.
    pub fn deal_damage(&mut self, target_id: PlayerId, amount: i32) -> Result<()> {
        // Check if target is a player
        if self.players.iter().any(|p| p.id == target_id) {
            // Source-filtered prevention shields first (Circle of Protection,
            // CR 615.6): prevent matching damage before the blanket shield.
            let source = self.current_damage_source;
            let amount = self.apply_source_prevention_shields(target_id, source, amount);
            if amount <= 0 {
                return Ok(());
            }

            // Apply replacement effects (e.g. Artist's Talent Level 3)
            let mut final_amount = amount;
            final_amount += self.get_noncombat_damage_modifier(target_id);

            // Apply damage prevention shield (CR 615.1)
            let (actual_amount, prevented) = {
                let player = self.get_player_mut(target_id)?;
                if player.damage_prevention > 0 {
                    let prevented = final_amount.min(player.damage_prevention);
                    player.damage_prevention -= prevented;
                    (final_amount - prevented, prevented)
                } else {
                    (final_amount, 0)
                }
            };

            if prevented > 0 {
                let player = self.get_player(target_id)?;
                self.logger.normal(&format!(
                    "{} damage prevented to {} ({} remaining shield)",
                    prevented, player.name, player.damage_prevention
                ));
            }

            if actual_amount <= 0 {
                return Ok(());
            }

            // Capture log size before life change
            let prior_log_size = self.logger.log_count();

            let player = self.get_player_mut(target_id)?;
            player.lose_life(actual_amount);

            // Log the life change for undo system
            self.undo_log.log(
                crate::undo::GameAction::ModifyLife {
                    player_id: target_id,
                    delta: -actual_amount,
                },
                prior_log_size,
            );

            // Accumulate for the non-combat "deals damage" trigger (Spirit Link,
            // CR 119.3). Fired once per resolution at the end of
            // resolve_spell_execute_effects with the aggregated total.
            self.accumulate_source_damage(actual_amount);

            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Deal damage to a creature
    ///
    /// MTG Rules 120.3: Damage dealt to a creature or planeswalker remains until the cleanup step
    /// MTG Rules 704.5g: State-based actions check if creature has lethal damage and destroys it
    ///
    /// # Errors
    ///
    /// Returns an error if the target is not a creature or cannot be found.
    pub fn deal_damage_to_creature(&mut self, target_id: CardId, amount: i32) -> Result<()> {
        // Get info about the target first (without holding the borrow)
        let (is_creature, is_planeswalker, creature_name) = {
            let card = self.cards.get(target_id)?;
            (card.is_creature(), card.is_planeswalker(), card.name.clone())
        };

        if is_creature {
            // Apply damage prevention shield (CR 615.1)
            let actual_amount = {
                let card = self.cards.get_mut(target_id)?;
                if card.damage_prevention > 0 {
                    let prevented = amount.min(card.damage_prevention);
                    card.damage_prevention -= prevented;
                    let remaining = amount - prevented;
                    if prevented > 0 {
                        self.logger.normal(&format!(
                            "{} damage prevented to {} ({}) ({} remaining shield)",
                            prevented, creature_name, target_id, card.damage_prevention
                        ));
                    }
                    remaining
                } else {
                    amount
                }
            };

            if actual_amount <= 0 {
                return Ok(());
            }

            // Apply replacement effects (e.g. Artist's Talent Level 3)
            let mut final_amount = actual_amount;
            if let Ok(target_card) = self.cards.get(target_id) {
                final_amount += self.get_noncombat_damage_modifier(target_card.controller);
            }

            // Mark damage on the creature (MTG CR 120.3)
            // Damage persists until cleanup step (CR 704.5f)
            // Snapshot marked damage for undo BEFORE mutating (mtg-728 sig-2f).
            self.log_damage(target_id);
            let card = self.cards.get_mut(target_id)?;
            card.damage += final_amount;

            let message = format!(
                "{} ({}) takes {} damage (total: {})",
                creature_name, target_id, final_amount, card.damage
            );
            self.logger.normal(&message);

            // Accumulate for the non-combat "deals damage" trigger (Spirit Link,
            // CR 119.3): damage dealt to creatures counts too. Fired once per
            // resolution at the end of resolve_spell_execute_effects.
            self.accumulate_source_damage(final_amount);

            // Note: We don't destroy the creature here - that happens in state-based actions
            // This allows multiple damage sources to accumulate before checking lethal damage
            return Ok(());
        } else if is_planeswalker {
            // Apply replacement effects (e.g. Artist's Talent Level 3)
            let mut final_amount = amount;
            if let Ok(target_card) = self.cards.get(target_id) {
                final_amount += self.get_noncombat_damage_modifier(target_card.controller);
            }

            if final_amount <= 0 {
                return Ok(());
            }

            // CR 120.3c: Damage dealt to a planeswalker causes that many loyalty counters to be removed from it.
            self.remove_counters(
                target_id,
                crate::core::CounterType::Loyalty,
                final_amount.min(255) as u8,
            )?;

            let new_loyalty = self
                .cards
                .get(target_id)?
                .get_counter(crate::core::CounterType::Loyalty);
            let message = format!(
                "{} ({}) takes {} damage (loyalty: {})",
                creature_name, target_id, final_amount, new_loyalty
            );
            self.logger.normal(&message);

            // Accumulate damage for deals-damage triggers
            self.accumulate_source_damage(final_amount);

            // Note: We don't put the planeswalker in graveyard here - that happens in state-based actions
            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Execute a Balance effect
    ///
    /// Balance equalizes permanents/cards of a specified type across all players.
    /// Each player with more than the minimum must sacrifice/discard down to the minimum.
    ///
    /// # Arguments
    /// * `card_type` - Type filter (e.g., "Creature", "Land", or empty for any)
    /// * `zone` - Zone to balance ("Battlefield" or "Hand")
    ///
    /// # MTG Rules
    /// - 701.17: To sacrifice means to move a permanent to graveyard
    /// - Balance card: Each player chooses, then sacrifices simultaneously
    ///
    /// Note: This is a non-interactive implementation. For proper interactive
    /// sacrifice choice (where players select which permanents to sacrifice),
    /// this must be called through the game loop which has access to controllers.
    ///
    /// # Errors
    ///
    /// Returns an error if balance effect execution fails.
    pub fn execute_balance_effect(&mut self, card_type: &str, zone: &str) -> Result<()> {
        // Get all player IDs
        let player_ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();

        // Handle Hand zone (discard) vs Battlefield zone (sacrifice)
        if zone == "Hand" {
            // Count cards in each player's hand
            let hand_counts: Vec<(PlayerId, usize)> = player_ids
                .iter()
                .map(|&pid| {
                    let count = self
                        .player_zones
                        .iter()
                        .find(|(id, _)| *id == pid)
                        .map(|(_, zones)| zones.hand.cards.len())
                        .unwrap_or(0);
                    (pid, count)
                })
                .collect();

            // Find minimum hand size
            let min_hand = hand_counts.iter().map(|(_, c)| *c).min().unwrap_or(0);

            // Log the balance action
            self.logger
                .gamelog(&format!("Balance: Hand sizes equalize to {}", min_hand));

            // Each player discards down to min (non-interactive: discard from end of hand)
            for (player_id, current_count) in hand_counts {
                if current_count > min_hand {
                    let discard_count = current_count - min_hand;

                    // Get the cards to discard (from end of hand)
                    let cards_to_discard: Vec<CardId> = self
                        .player_zones
                        .iter()
                        .find(|(id, _)| *id == player_id)
                        .map(|(_, zones)| zones.hand.cards.iter().rev().take(discard_count).copied().collect())
                        .unwrap_or_default();

                    // Discard each card
                    for card_id in cards_to_discard {
                        self.move_card(card_id, Zone::Hand, Zone::Graveyard, player_id)?;

                        // Log the discard. mtg-795: emit UNCONDITIONALLY with a
                        // reveal-timing-INDEPENDENT verifier key (mirrors the
                        // move_card Hand→Graveyard discard line and the
                        // discard_card line, both fixed under mtg-677). On a
                        // network shadow the discarded OPPONENT card's public
                        // `RevealCard` can arrive one ChoiceRequest AFTER the
                        // forced Balance resolution that discards it (the shadow's
                        // forward GameLoop runs AHEAD of the reveal stream), so a
                        // `try_get`-gated line is DROPPED on the first forward pass
                        // but PRESENT on a rewind replay (the instance is left
                        // behind) → a spurious line-COUNT offset that shifts every
                        // later entry and trips the rewind/replay verifier
                        // (robots seeds 7/11, Balance hand-equalize). The card is
                        // in the PUBLIC graveyard identically on both passes
                        // (CR 400.2/404; the turn-start hash proves the STATE), so
                        // comparing the server-authoritative CardId keeps full
                        // rigor while not flagging the presentation asymmetry.
                        let player_name = self
                            .get_player(player_id)
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|_| "Player".to_string());
                        let card_name = self
                            .cards
                            .try_get(card_id)
                            .map(|c| c.name.to_string())
                            .unwrap_or_else(|| format!("card#{}", card_id.as_u32()));
                        let verifier_stable = format!("{} discards card#{} to Balance", player_name, card_id.as_u32());
                        self.logger.gamelog_reveal_stable(
                            &format!("{} discards {} to Balance", player_name, card_name),
                            &verifier_stable,
                        );
                    }
                }
            }
        } else {
            // Battlefield zone - sacrifice permanents
            // Filter by card type if specified
            let counts_and_permanents: Vec<(PlayerId, usize, Vec<CardId>)> = player_ids
                .iter()
                .map(|&pid| {
                    // Get this player's permanents matching the type
                    let matching_permanents: Vec<CardId> = self
                        .battlefield
                        .cards
                        .iter()
                        .filter(|&&card_id| {
                            if let Some(card) = self.cards.try_get(card_id) {
                                // Must be controlled by this player
                                if card.controller != pid {
                                    return false;
                                }
                                // Filter by card type
                                match card_type {
                                    "Creature" => card.is_creature(),
                                    "Land" => card.is_land(),
                                    "Artifact" => card.is_artifact(),
                                    "Enchantment" => card.is_enchantment(),
                                    "" => true, // Any permanent
                                    _ => true,  // Default to any
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

            // Find minimum count
            let min_count = counts_and_permanents.iter().map(|(_, c, _)| *c).min().unwrap_or(0);

            // Log the balance action
            let type_str = if card_type.is_empty() { "permanents" } else { card_type };
            self.logger
                .gamelog(&format!("Balance: {} equalize to {}", type_str, min_count));

            // Each player sacrifices down to min
            // Non-interactive: sacrifice from end of list (last in battlefield order)
            for (player_id, current_count, permanents) in counts_and_permanents {
                if current_count > min_count {
                    let sacrifice_count = current_count - min_count;

                    // Get permanents to sacrifice (from end of list)
                    let to_sacrifice: Vec<CardId> = permanents.into_iter().rev().take(sacrifice_count).collect();

                    // Sacrifice each permanent
                    for card_id in to_sacrifice {
                        let owner = self.cards.get(card_id)?.owner;

                        // Log before moving
                        if let Some(card) = self.cards.try_get(card_id) {
                            let player_name = self
                                .get_player(player_id)
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| "Player".to_string());
                            self.logger
                                .gamelog(&format!("{} sacrifices {} to Balance", player_name, card.name));
                        }

                        // Check death triggers BEFORE moving the card
                        let _ = self.check_death_triggers(card_id);

                        // Move to graveyard (or exile if finality counter)
                        let dest = self.death_destination_for_card(card_id);
                        self.move_card(card_id, Zone::Battlefield, dest, owner)?;

                        // Check sacrifice triggers (e.g., Pirate Peddlers)
                        let _ = self.check_triggers(TriggerEvent::Sacrificed, card_id);
                    }
                }
            }
        }

        Ok(())
    }

    /// Evaluate a count expression against the current game state
    ///
    /// Used for variable effects like "gets +X/+X where X is the number of artifacts
    /// your opponents control" (Elephant-Mandrill).
    ///
    /// # Errors
    ///
    /// This function is infallible and always returns `Ok`. The Result type is used
    /// for consistency with other effect evaluation methods.
    /// Snapshot the dynamic characteristics (power and mana value) of each
    /// chosen target at the start of spell resolution, keyed by `CardId`.
    ///
    /// This captures **last-known information** (CR 608.2g/2h) so a chained
    /// `GainLifeDynamic` reads the targeted permanent's power / mana value *as
    /// it existed on the battlefield with its continuous buffs applied*, before
    /// a preceding exile/destroy effect strips those buffs.
    fn snapshot_target_amounts(&self, chosen_targets: &[CardId]) -> std::collections::HashMap<CardId, TargetSnapshot> {
        use crate::core::CounterType;
        let mut snapshots = std::collections::HashMap::with_capacity(chosen_targets.len());
        for &target in chosen_targets {
            // A player-target sentinel CardId is not a real card: its only
            // relevant pre-damage characteristic is the player's LIFE total
            // (Drain Life's cap when the target is a player).
            if let Some(player_id) = crate::core::player_target_from_sentinel(target) {
                if let Ok(p) = self.get_player(player_id) {
                    snapshots.insert(
                        target,
                        TargetSnapshot {
                            power: 0,
                            mana_value: 0,
                            drain_cap: p.life,
                        },
                    );
                }
                continue;
            }
            if let Some(card) = self.cards.try_get(target) {
                // Use the CR 613 layer system (effective power) so continuous
                // static buffs that apply while on the battlefield are counted
                // (e.g. Sedge Troll's "+1/+1 while you control a Swamp"). Fall
                // back to the raw current power if the breakdown is unavailable.
                let power = self
                    .get_effective_power(target)
                    .unwrap_or_else(|_| i32::from(card.current_power()));
                let mana_value = i32::from(card.mana_cost.cmc());
                // Drain Life cap: the target's pre-damage toughness (creature) /
                // loyalty (planeswalker). "before the damage was dealt" — read
                // now, before any effect in this resolution runs (CR 608.2g/2h).
                let drain_cap = if card.is_planeswalker() {
                    i32::from(card.get_counter(CounterType::Loyalty))
                } else {
                    self.get_effective_toughness(target)
                        .unwrap_or_else(|_| i32::from(card.current_toughness()))
                };
                snapshots.insert(
                    target,
                    TargetSnapshot {
                        power,
                        mana_value,
                        drain_cap,
                    },
                );
            }
        }
        snapshots
    }

    /// Lock a `GainLifeDynamic` effect's amount to the pre-resolution snapshot
    /// of its `reference` card. Non-`GainLifeDynamic` effects (and dynamic
    /// amounts not derived from a target characteristic, e.g. `DamageDealt`)
    /// pass through unchanged.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_dynamic_gainlife_snapshot(
        effect: Effect,
        snapshots: &std::collections::HashMap<CardId, TargetSnapshot>,
    ) -> Effect {
        use crate::core::DynamicAmount;
        match effect {
            Effect::GainLifeDynamic {
                player,
                amount,
                reference,
            } => {
                let locked = match (&amount, snapshots.get(&reference)) {
                    (DynamicAmount::TargetPower, Some(snap)) => DynamicAmount::Fixed(snap.power.max(0)),
                    (DynamicAmount::TargetManaValue, Some(snap)) => DynamicAmount::Fixed(snap.mana_value),
                    // Drain Life: lock the cap to the target's PRE-damage
                    // life/loyalty/toughness now; the damage-dealt term is read
                    // at execute time (it isn't known until the chained
                    // DealDamage has run). gain = min(damage dealt, cap).
                    (DynamicAmount::DamageDealtCappedByTarget { .. }, Some(snap)) => {
                        DynamicAmount::DamageDealtCappedByTarget {
                            cap: Some(snap.drain_cap.max(0)),
                        }
                    }
                    // No snapshot (reference not a chosen target) or plain
                    // DamageDealt: keep the dynamic amount for execute-time
                    // resolution.
                    _ => amount,
                };
                Effect::GainLifeDynamic {
                    player,
                    amount: locked,
                    reference,
                }
            }
            other => other,
        }
    }

    /// Resolve a [`DynamicAmount`](crate::core::DynamicAmount) to a concrete
    /// life amount, reading public game state at resolution time.
    ///
    /// For `TargetPower` / `TargetManaValue` the `reference` card is read via
    /// last-known information (CR 608.2g): the card object persists in the
    /// entity store after a zone move, so its power / mana value reflect its
    /// last existence on the battlefield. A negative power yields 0 life
    /// (you cannot "gain" negative life; CR 119.4 — a player gains 0 life).
    ///
    /// This is information-independent (uses only public characteristics), so
    /// it produces identical results on the server and every client / WASM
    /// shadow game.
    fn resolve_dynamic_amount(&self, amount: &crate::core::DynamicAmount, reference: CardId, player: PlayerId) -> i32 {
        use crate::core::DynamicAmount;
        match amount {
            DynamicAmount::Fixed(n) => *n,
            DynamicAmount::TargetPower => self
                .cards
                .try_get(reference)
                .map(|c| i32::from(c.current_power()).max(0))
                .unwrap_or(0),
            DynamicAmount::TargetManaValue => self
                .cards
                .try_get(reference)
                .map(|c| i32::from(c.mana_cost.cmc()))
                .unwrap_or(0),
            DynamicAmount::DamageDealt => {
                // The damage dealt so far in THIS resolution (the running total
                // accumulated by deal_damage; Some(..) during the effect loop,
                // before the deals-damage trigger takes it). Clamp >= 0.
                self.damage_dealt_by_source.unwrap_or(0).max(0)
            }
            DynamicAmount::DamageDealtCappedByTarget { cap } => {
                // Drain Life: gain = min(damage dealt this resolution, cap),
                // where cap = the target's pre-damage life/loyalty/toughness
                // (locked into the snapshot during target resolution). An
                // unlocked cap (None) degrades to plain damage-dealt.
                let dealt = self.damage_dealt_by_source.unwrap_or(0).max(0);
                match cap {
                    Some(c) => dealt.min((*c).max(0)),
                    None => dealt,
                }
            }
            // Count$… expression evaluated against the recipient player (e.g.
            // Ivory Tower: cards in YOUR hand minus 4). Only public state (hand
            // SIZE, permanent counts) is read, so the result is identical on the
            // server and every client/WASM shadow. Clamp to >= 0: a player
            // cannot gain negative life (CR 119.4), e.g. an empty hand under
            // Ivory Tower yields 0, not -4.
            DynamicAmount::Count(expr) => self.evaluate_count_expression(expr, player).unwrap_or(0).max(0),
            // Diamond Valley: the toughness of the creature sacrificed to pay the
            // cost. The creature has already left the battlefield by resolution,
            // so we read its retained (last-known) toughness from the entity store
            // (CR 608.2g). Only public characteristics are read, so the result is
            // identical on server and every client/WASM shadow. Clamp >= 0 (a
            // creature with negative toughness gives 0 life, CR 119.4).
            DynamicAmount::SacrificedToughness => self
                .cards
                .try_get(reference)
                .map(|c| i32::from(c.current_toughness()).max(0))
                .unwrap_or(0),
        }
    }

    /// Evaluate a [`CountExpression`](crate::core::CountExpression) to a
    /// concrete integer against the current game state for `controller`.
    ///
    /// # Errors
    ///
    /// Returns an error if a nested lookup (e.g. a referenced player) fails.
    pub fn evaluate_count_expression(&self, expr: &crate::core::CountExpression, controller: PlayerId) -> Result<i32> {
        use crate::core::CountExpression;
        match expr {
            CountExpression::Fixed(n) => Ok(*n),
            CountExpression::ValidPermanents { filter } => {
                let count = self.count_permanents_matching(filter, controller);
                Ok(i32::try_from(count).unwrap_or(i32::MAX))
            }
            CountExpression::CardsDrawnThisTurn => {
                if let Ok(player) = self.get_player(controller) {
                    Ok(i32::from(player.cards_drawn_this_turn))
                } else {
                    Ok(0)
                }
            }
            CountExpression::CardsInHand { selector: _, modifier } => {
                // Count cards in the hand of the player we are evaluating FOR.
                // Black Vise's `Count$ValidHand Card.ChosenCtrl/Minus.4` is
                // evaluated against the chosen (triggered/active) player passed
                // in `controller` by Effect::DealDamageToTriggeredPlayer, so the
                // hand owner is exactly that player. Only the public hand SIZE is
                // read (never card identities) — information-independent for
                // network determinism. The /Minus.N modifier is applied raw; the
                // caller clamps to >= 0 where MTG requires it (damage: CR 119.4).
                let hand_size = self
                    .get_player_zones(controller)
                    .map(|z| z.hand.cards.len())
                    .unwrap_or(0);
                let raw = i32::try_from(hand_size).unwrap_or(i32::MAX);
                Ok(modifier.apply(raw))
            }
            CountExpression::XPaid => {
                // XPaid is typically resolved during spell resolution via
                // resolve_x_paid_effect(). For variable P/T and other uses,
                // return 0 as fallback (the card's x_paid isn't accessible here
                // without knowing which card to look at).
                log::debug!("evaluate_count_expression: XPaid evaluated as 0 (no card context)");
                Ok(0)
            }
            CountExpression::SpellsCastThisTurn => {
                if let Ok(player) = self.get_player(controller) {
                    Ok(i32::from(player.spells_cast_this_turn))
                } else {
                    Ok(0)
                }
            }
            CountExpression::ValidGraveyard { filter, modifier } => {
                // Count cards in the controller's graveyard matching `filter`,
                // then apply the arithmetic modifier (e.g. `/Plus.2` for
                // Combustion Technique: "Lesson cards in graveyard + 2").
                // Graveyard contents are public (CR 400.2), so this is
                // information-independent for network determinism.
                let raw = self.count_cards_matching_filter(controller, filter, crate::zones::Zone::Graveyard);
                let raw_i32 = i32::try_from(raw).unwrap_or(i32::MAX);
                Ok(modifier.apply(raw_i32))
            }
            CountExpression::Compare {
                source,
                condition,
                true_value,
                false_value,
            } => {
                // Evaluate the source expression
                let source_value = self.evaluate_count_expression(source, controller)?;
                // Apply the condition and return the appropriate value
                if condition.evaluate(source_value) {
                    Ok(*true_value)
                } else {
                    Ok(*false_value)
                }
            }
            CountExpression::Kicked {
                kicked_value: _,
                unkicked_value,
            } => {
                // Kicker state is not yet tracked at resolution time (mtg-820).
                // Conservatively evaluate as unkicked (the lower/safer damage value).
                // TODO: Once kicker tracking is implemented, resolve the actual state.
                Ok(*unkicked_value)
            }
            CountExpression::TargetedCardPower => {
                // TargetedCardPower needs the effect's target card, which this
                // controller-only signature does not carry. The
                // PumpCreatureVariable executor resolves it directly from its
                // `target` (reading power BEFORE applying the pump). If this
                // arm is reached, the expression escaped that path — evaluate to
                // 0 rather than silently mis-counting.
                log::debug!("evaluate_count_expression: TargetedCardPower has no target context here; evaluated as 0");
                Ok(0)
            }
        }
    }

    /// Count permanents on the battlefield matching a filter string
    ///
    /// Filter format examples:
    /// - "Artifact.OppCtrl" - artifacts opponents control
    /// - "Creature.YouCtrl" - creatures you control
    /// - "Land.YouCtrl" - lands you control
    /// - "Permanent" - all permanents
    fn count_permanents_matching(&self, filter: &str, controller: PlayerId) -> usize {
        self.battlefield
            .cards
            .iter()
            .filter(|&&card_id| {
                if let Some(card) = self.cards.try_get(card_id) {
                    // Check card type filter. The leading token (before the first
                    // `.`) is the type/subtype selector; basic-land subtypes
                    // (Swamp/Plains/Island/Mountain/Forest) are recognised via the
                    // pre-computed cache flags so that e.g. Karma's
                    // `Count$Valid Swamp.ActivePlayerCtrl` counts Swamps.
                    let type_token = filter.split('.').next().unwrap_or(filter);
                    let type_matches = match type_token {
                        "Artifact" => card.is_artifact(),
                        "Creature" => card.is_creature(),
                        "Land" => card.is_land(),
                        "Enchantment" => card.is_enchantment(),
                        "Permanent" | "Card" => true, // Any permanent
                        "Swamp" => card.definition.cache.has_swamp_subtype,
                        "Plains" => card.definition.cache.has_plains_subtype,
                        "Island" => card.definition.cache.has_island_subtype,
                        "Mountain" => card.definition.cache.has_mountain_subtype,
                        "Forest" => card.definition.cache.has_forest_subtype,
                        _ => {
                            // Unknown type, assume it matches if we can't parse
                            log::warn!(target: "count", "Unknown filter type in count expression: {}", filter);
                            true
                        }
                    };

                    if !type_matches {
                        return false;
                    }

                    // Check controller filter. `ActivePlayerCtrl` is evaluated
                    // against the player passed in `controller`; callers
                    // resolving an "each player's upkeep" trigger pass the active
                    // player here (see Effect::DealDamageToTriggeredPlayer), so
                    // ActivePlayerCtrl and YouCtrl coincide and mean "the player
                    // we are counting for".
                    if filter.contains("OppCtrl") {
                        // Opponents control - not the controller
                        card.controller != controller
                    } else if filter.contains("YouCtrl") || filter.contains("ActivePlayerCtrl") {
                        // You / active player control
                        card.controller == controller
                    } else {
                        // No controller restriction
                        true
                    }
                } else {
                    false
                }
            })
            .count()
    }
}

#[allow(clippy::collapsible_if)]
fn matches_taps_for_mana_filter(
    card: &crate::core::Card,
    filter: &str,
    trigger_card: &crate::core::Card,
    controller: PlayerId,
) -> bool {
    let type_token = filter.split('.').next().unwrap_or(filter);
    let type_matches = match type_token {
        "Artifact" => card.is_artifact(),
        "Creature" => card.is_creature(),
        "Land" => card.is_land(),
        "Enchantment" => card.is_enchantment(),
        "Permanent" | "Card" => true,
        "Swamp" => card.definition.cache.has_swamp_subtype,
        "Plains" => card.definition.cache.has_plains_subtype,
        "Island" => card.definition.cache.has_island_subtype,
        "Mountain" => card.definition.cache.has_mountain_subtype,
        "Forest" => card.definition.cache.has_forest_subtype,
        _ => false,
    };
    if !type_matches {
        if type_token == "Mountain,Forest,Plains" {
            if !card.definition.cache.has_mountain_subtype
                && !card.definition.cache.has_forest_subtype
                && !card.definition.cache.has_plains_subtype
            {
                return false;
            }
        } else {
            return false;
        }
    }

    if filter.contains(".nonLand") {
        if card.is_land() {
            return false;
        }
    }
    if filter.contains(".nonBasic") {
        let is_basic = matches!(
            card.name.as_str(),
            "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" | "Wastes"
        );
        if is_basic {
            return false;
        }
    }
    if filter.contains(".Basic") {
        let is_basic = matches!(
            card.name.as_str(),
            "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" | "Wastes"
        );
        if !is_basic {
            return false;
        }
    }
    if filter.contains(".token") {
        if !card.is_token {
            return false;
        }
    }
    if filter.contains("OppCtrl") {
        if card.controller == controller {
            return false;
        }
    }
    if filter.contains("YouCtrl") {
        if card.controller != controller {
            return false;
        }
    }
    if filter.contains("AttachedBy") || filter.contains("EnchantedBy") || filter.contains("FortifiedBy") {
        if trigger_card.attached_to != Some(card.id) {
            return false;
        }
    }
    if filter.contains("Self") {
        if card.id != trigger_card.id {
            return false;
        }
    }

    true
}

// Submodules
mod combat;
mod targeting;

#[cfg(test)]
mod tests;
