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

/// Collect all `Effect`s from an SVar body by following `SubAbility$` chains.
///
/// Used by [`GameState::fire_saga_chapter`] to convert a chapter SVar body (e.g.
/// `"DB$ Mill | Defined$ Player | NumCards$ 3 | SubAbility$ DBExile"`) into a
/// flat list of `Effect`s that can be executed by `execute_effect`.
///
/// This mirrors `CardDefinition::follow_sub_ability_chain` in the loader but
/// operates at game-time rather than parse-time, so we can handle Saga chapter
/// abilities that were stored as `K:Chapter` keywords rather than `A:SP$` effects.
fn collect_svar_chain_effects(svar_body: &str, svars: &std::collections::HashMap<String, String>) -> Vec<Effect> {
    use crate::loader::ability_parser::{AbilityParams, ApiType};
    use crate::loader::effect_converter::params_to_effect_with_svars;

    let mut effects = Vec::new();
    let mut current_body = svar_body.to_string();

    // Guard against infinite loops (e.g., malformed SubAbility cycles).
    for _ in 0..20 {
        let prefixed = format!("A:{}", current_body);
        let params = match AbilityParams::parse(&prefixed) {
            Ok(p) => p,
            Err(e) => {
                log::debug!("collect_svar_chain_effects: parse error '{}': {}", current_body, e);
                break;
            }
        };

        // Convert to an Effect using the SVar-aware converter.
        #[allow(clippy::wildcard_enum_match_arm)]
        let effect = match params.api_type {
            ApiType::Charm => crate::loader::effect_converter::params_to_charm_effect_with_svars(&params, svars),
            ApiType::DelayedTrigger => {
                crate::loader::effect_converter::params_to_delayed_trigger_with_svars(&params, svars)
            }
            ApiType::ImmediateTrigger => {
                crate::loader::effect_converter::params_to_immediate_trigger_with_svars(&params, svars)
            }
            _ => params_to_effect_with_svars(&params, svars),
        };

        if let Some(e) = effect {
            effects.push(e);
        }

        // Follow SubAbility$ chain.
        let sub_name = match params.get("SubAbility") {
            Some(n) => n,
            None => break,
        };
        let sub_body = match svars.get(sub_name) {
            Some(b) => b.clone(),
            None => {
                log::debug!("collect_svar_chain_effects: SubAbility$ '{}' not in SVars", sub_name);
                break;
            }
        };
        current_body = sub_body;
    }

    effects
}

/// Resolve placeholder controller/owner fields in a chapter effect so that
/// `execute_effect` sees the real `PlayerId` of the Saga's controller.
///
/// This mirrors the partial placeholder-resolution done by
/// `resolve_effect_target` for spell effects but restricted to the simple
/// You/Opponent cases needed by most Saga chapter abilities.
#[allow(clippy::wildcard_enum_match_arm)]
fn resolve_saga_effect_controller(effect: Effect, controller: PlayerId, opponent: Option<PlayerId>) -> Effect {
    let opp = opponent.unwrap_or(controller);
    // Patch PlayerId::placeholder() (== PlayerId(0) sentinel) to `controller`.
    // For effects that address "each player" we leave them as-is; the
    // `expand_all_players_effect` path handles those.
    let resolve = |p: PlayerId| {
        if p.is_placeholder() {
            controller
        } else {
            p
        }
    };
    match effect {
        Effect::DrawCards { player, count } => Effect::DrawCards {
            player: resolve(player),
            count,
        },
        Effect::DrawCardsXPaid { player } => Effect::DrawCardsXPaid {
            player: resolve(player),
        },
        Effect::GainLife { player, amount } => Effect::GainLife {
            player: resolve(player),
            amount,
        },
        Effect::LoseLife { player, amount } => Effect::LoseLife {
            player: resolve(player),
            amount,
        },
        Effect::DiscardCards {
            player,
            count,
            remember_discarded,
            optional,
            remember_discarding_players,
        } => Effect::DiscardCards {
            player: resolve(player),
            count,
            remember_discarded,
            optional,
            remember_discarding_players,
        },
        Effect::Mill { player, count } => Effect::Mill {
            player: resolve(player),
            count,
        },
        Effect::ForceSacrifice {
            player,
            sac_type,
            count,
        } => Effect::ForceSacrifice {
            player: resolve(player),
            sac_type,
            count,
        },
        Effect::SetLife { player, amount } => Effect::SetLife {
            player: resolve(player),
            amount,
        },
        // DealDamage targets a CardId (sentinel for player) — resolve the player-target sentinel.
        Effect::DealDamage { target, amount } => {
            let resolved_target = match target {
                crate::core::TargetRef::Player(p) if p.is_placeholder() => crate::core::TargetRef::Player(opp),
                other => other,
            };
            Effect::DealDamage {
                target: resolved_target,
                amount,
            }
        }
        other => other,
    }
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
        | Effect::UntapOne { .. }
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
        | Effect::GrantCastWithFlash { .. }
        | Effect::ReturnPermanentToHand { .. }
        | Effect::ExilePermanent { .. }
        | Effect::ExileIfWouldDieThisTurn { .. }
        | Effect::SearchLibrary { .. }
        | Effect::AttachEquipment { .. }
        | Effect::CreateToken { .. }
        | Effect::CreateTokenWithStoredPt { .. }
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
        | Effect::ChooseAndRememberOneOfEach { .. }
        | Effect::AddTurn { .. }
        | Effect::AddPhase { .. }
        | Effect::ChooseColor { .. }
        | Effect::Clone { .. }
        | Effect::SelfExileFromStack { .. }
        | Effect::MoveSelfBetweenZones { .. }
        | Effect::ReturnCardsFromGraveyardToHand { .. }
        | Effect::ReturnGraveyardCardToHand { .. }
        | Effect::ReturnGraveyardCardToZone { .. }
        | Effect::PutCreatureFromHandOnBattlefield { .. }
        | Effect::ReturnSelfAsEnchantment { .. }
        | Effect::PreventAllCombatDamageThisTurn { .. }
        | Effect::ConditionalSelfCounter { .. }
        | Effect::RearrangeTopOfLibrary { .. }
        | Effect::SkipUntapStep { .. }
        | Effect::Unimplemented { .. }
        | Effect::NoOp { .. }
        | Effect::GainLifeDynamic { .. }
        | Effect::ClassLevelUp { .. }
        | Effect::SacrificeSelf { .. }
        | Effect::UnlessCostWrapper { .. }
        | Effect::CreateTokenDynamic { .. }
        | Effect::CreateEmblem { .. }
        | Effect::PutCardsFromHandOnTopOfLibrary { .. }
        | Effect::RevealCardsFromHand { .. }
        | Effect::PlayFromGraveyard { .. }
        | Effect::RepeatEach { .. }
        | Effect::ExtraLandPlay { .. }
        | Effect::TapPermanentsMatchingFilter { .. } => false,
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
            | Effect::UntapOne { .. }
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
            | Effect::GrantCastWithFlash { .. }
            | Effect::ReturnPermanentToHand { .. }
            | Effect::ExilePermanent { .. }
            | Effect::ExileIfWouldDieThisTurn { .. }
            | Effect::SearchLibrary { .. }
            | Effect::AttachEquipment { .. }
            | Effect::CreateToken { .. }
            | Effect::CreateTokenWithStoredPt { .. }
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
            | Effect::ChooseAndRememberOneOfEach { .. }
            | Effect::AddTurn { .. }
            | Effect::AddPhase { .. }
            | Effect::ChooseColor { .. }
            | Effect::Clone { .. }
            | Effect::SelfExileFromStack { .. }
            | Effect::MoveSelfBetweenZones { .. }
            | Effect::ReturnCardsFromGraveyardToHand { .. }
            | Effect::ReturnGraveyardCardToHand { .. }
            | Effect::ReturnGraveyardCardToZone { .. }
            | Effect::PutCreatureFromHandOnBattlefield { .. }
            | Effect::ReturnSelfAsEnchantment { .. }
            | Effect::PreventAllCombatDamageThisTurn { .. }
            | Effect::ConditionalSelfCounter { .. }
            | Effect::RearrangeTopOfLibrary { .. }
            | Effect::SkipUntapStep { .. }
            | Effect::Unimplemented { .. }
            | Effect::NoOp { .. }
            | Effect::GainLifeDynamic { .. }
            | Effect::ClassLevelUp { .. }
            | Effect::SacrificeSelf { .. }
            | Effect::UnlessCostWrapper { .. }
            | Effect::CreateTokenDynamic { .. }
            | Effect::CreateEmblem { .. }
            | Effect::PutCardsFromHandOnTopOfLibrary { .. }
            | Effect::RevealCardsFromHand { .. }
            | Effect::PlayFromGraveyard { .. }
            | Effect::RepeatEach { .. }
            | Effect::ExtraLandPlay { .. }
            | Effect::TapPermanentsMatchingFilter { .. } => unreachable!(),
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
        // SelfCard: only the card that CARRIES this ability (when cast from hand).
        // The battlefield-scan loop should never match SelfCard because that path
        // is for OTHER permanents granting reductions.  SelfCard is handled by the
        // dedicated "self-ReduceCost" pass in calculate_effective_cost instead.
        CostReductionTarget::SelfCard => false,
        CostReductionTarget::NonCreature => !card.is_creature(),
        CostReductionTarget::Creature => card.is_creature(),
        CostReductionTarget::Subtype(subtype) => card.subtypes.contains(subtype),
        CostReductionTarget::Color(color) => card.is_color(*color),
    }
}

impl GameState {
    /// Compute the maximum number of lands the player may play this turn.
    ///
    /// Starts at 1 (the default rule per CR 305.2), then adds:
    /// - `StaticAbility::ExtraLandPlay` on each battlefield permanent controlled by `player_id`
    ///   (Oracle of Mul Daya, Exploration enchantment, Azusa, etc.)
    /// - `PersistentEffectKind::ExtraLandPlay` for each temporary grant in effect
    ///   (e.g., the Explore spell "you may play an additional land this turn")
    pub fn effective_max_lands(&self, player_id: crate::core::PlayerId) -> u8 {
        let mut extra: u8 = 0;

        // Sum permanent statics on the battlefield
        for &card_id in &self.battlefield.cards {
            let Some(card) = self.cards.try_get(card_id) else {
                continue;
            };
            if card.controller != player_id {
                continue;
            }
            for sa in &card.static_abilities {
                if let crate::core::StaticAbility::ExtraLandPlay { amount, .. } = sa {
                    extra = extra.saturating_add(*amount);
                }
            }
        }

        // Sum temporary persistent effects (e.g., Explore)
        extra = extra.saturating_add(self.persistent_effects.extra_land_plays_for_player(player_id));

        1u8.saturating_add(extra)
    }

    /// Check whether `player_id` is still permitted to play a land this turn,
    /// taking into account extra land-play grants (Oracle of Mul Daya, Explore, …).
    ///
    /// Replaces direct `player.can_play_land()` calls where extra-land-play
    /// statics/effects must be respected.
    pub fn can_play_land_effective(&self, player_id: crate::core::PlayerId) -> bool {
        match self.get_player(player_id) {
            Ok(player) => player.lands_played_this_turn < self.effective_max_lands(player_id),
            Err(_) => false,
        }
    }

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
        // Check if player can play a land (respecting extra land-play grants)
        if !self.can_play_land_effective(player_id) {
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

    /// Play the top card of the library as a land (Experimental Frenzy, Future Sight).
    ///
    /// Mirrors `play_land` but sources the card from `Zone::Library` rather than
    /// `Zone::Hand`.  The card must be both a land and the top card of the
    /// controller's library.
    ///
    /// # Errors
    ///
    /// Returns `MtgError::InvalidAction` if the player cannot play a land, the
    /// card is not a land, or the card is not at the top of the library.
    pub fn play_land_from_library(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        if !self.can_play_land_effective(player_id) {
            return Err(MtgError::InvalidAction("Cannot play more lands this turn".to_string()));
        }

        let card = self.cards.get(card_id)?;
        if !card.is_land() {
            return Err(MtgError::InvalidAction("Card is not a land".to_string()));
        }

        // Verify it is the top of the library.
        let owner = card.owner;
        let is_top = self
            .get_player_zones(owner)
            .and_then(|z| z.library.cards.last().copied())
            == Some(card_id);
        if !is_top {
            return Err(MtgError::InvalidAction("Card is not the top of library".to_string()));
        }

        // Move card to battlefield (reveals automatically via move_card).
        self.move_card(card_id, Zone::Library, Zone::Battlefield, player_id)?;

        // Record turn entered battlefield.
        if let Ok(card) = self.cards.get_mut(card_id) {
            let old_value = card.turn_entered_battlefield;
            let prior_log_size = self.logger.log_count();
            let new_value = Some(self.turn.turn_number);
            card.turn_entered_battlefield = new_value;
            self.undo_log.log(
                crate::undo::GameAction::SetTurnEnteredBattlefield {
                    card_id,
                    old_value,
                    new_value,
                },
                prior_log_size,
            );
        }

        // Increment lands played.
        let old_value = self.get_player(player_id)?.lands_played_this_turn;
        let prior_log_size = self.logger.log_count();
        let player = self.get_player_mut(player_id)?;
        player.play_land();
        let new_value = player.lands_played_this_turn;
        self.undo_log.log(
            crate::undo::GameAction::SetLandsPlayedThisTurn {
                player_id,
                old_value,
                new_value,
            },
            prior_log_size,
        );

        self.apply_etb_counters(card_id)?;
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

        // Set current_spell_controller so that execute_counter_spell can determine
        // whether a countered creature spell was countered by an opponent (Summoning
        // Trap condition). Cleared at the end of this function.
        self.current_spell_controller = Some(card_owner);

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
                        Some(card_id),
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

        // Clear transient spell-controller context now that effects are done.
        self.current_spell_controller = None;

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
            // `DestroyAll` with a dynamic `cmcEQX` SVar (Ratchet Bomb): record the
            // source CardId so execute_effect can read its charge-counter count at
            // resolution time and set `exact_cmc` on the restriction.
            Effect::DestroyAll {
                restriction,
                no_regenerate,
                cmc_eq_source: None,
            } if restriction.cmc_eq_svar => Effect::DestroyAll {
                restriction,
                no_regenerate,
                cmc_eq_source: Some(source_card_id),
            },
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

        // CR 702.88: Epic — handle before determining the normal destination.
        // If the resolving spell has the Epic keyword (e.g. Enduring Ideal):
        //   (a) Exile the card instead of sending it to the graveyard.
        //   (b) Register a repeating upkeep delayed trigger that re-executes the
        //       spell's effects (excluding the Epic keyword itself) at the
        //       beginning of each subsequent upkeep for the rest of the game.
        //   (c) The controller can't cast spells for the rest of the game.
        let has_epic = self
            .cards
            .get(card_id)
            .map(|c| c.keywords.contains(crate::core::Keyword::Epic))
            .unwrap_or(false);
        if has_epic {
            let owner = self.cards.get(card_id)?.owner;

            // (c) CR 702.88b: controller can't cast spells for the rest of the game.
            {
                let old_value = self.get_player(owner).map(|p| p.cant_cast_spells).unwrap_or(false);
                if let Ok(player) = self.get_player_mut(owner) {
                    player.cant_cast_spells = true;
                    log::debug!(target: "epic", "Epic: player {:?} can no longer cast spells", owner);
                }
                let prior_log_size = self.logger.log_count();
                self.undo_log.log(
                    crate::undo::GameAction::SetCantCastSpells {
                        player_id: owner,
                        old_value,
                    },
                    prior_log_size,
                );
            }

            // (b) Register a repeating upkeep delayed trigger. The effect clones
            // the card's spell effects so the library-search fires every upkeep.
            // We use DelayedEffect::ExecuteEffect wrapping a ChangeZone (search
            // library for enchantment → battlefield) — the exact effect list from
            // the card is not yet available in a clone-free way, so we re-execute
            // the card's effects by firing a "copy spell" approach:
            // use DelayedEffect::CopySpellAbility on a saved card ref.
            //
            // Implementation: store the resolving card id as tracked_card so the
            // trigger can re-execute its effects. Since the card will be in Exile
            // after resolution (not the Stack), we store each effect of the spell
            // body (the non-Epic part — all effects the parser produced) for
            // replay by wrapping them in ExecuteEffect on the first one.
            // For Enduring Ideal the single effect is ChangeZone(Library→Battlefield,Enchantment).
            //
            // CR 702.88c: "copy the spell except for its epic ability" — the
            // copy doesn't have Epic (so it won't self-exile / re-trigger again
            // from THIS copy; the original delayed trigger handles looping).
            // We implement this by executing each effect from the card's effect
            // list directly (using execute_effect) inside the trigger.
            //
            // For now we wrap the FIRST non-trivial effect as an ExecuteEffect.
            // TODO(mtg-920): For spells with multiple non-Epic effects, wrap each
            // one separately or extend to an EffectList variant.
            {
                use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerCondition, TriggerPhase, TurnOwner};
                use smallvec::smallvec;

                // Collect the spell's effects (non-Epic body) to re-execute.
                let spell_effects: Vec<crate::core::Effect> =
                    self.cards.get(card_id).map(|c| c.effects.clone()).unwrap_or_default();

                if let Some(first_effect) = spell_effects.into_iter().next() {
                    let epic_trigger = DelayedTrigger::new(
                        crate::core::DelayedTriggerId::new(0),
                        card_id,
                        card_id,
                        owner,
                        DelayedTriggerCondition::Phase {
                            phases: smallvec![TriggerPhase::Upkeep],
                            whose_turn: TurnOwner::You,
                        },
                        DelayedEffect::ExecuteEffect {
                            effect: Box::new(first_effect),
                        },
                    )
                    .repeating();
                    self.delayed_triggers.add(epic_trigger);
                    log::debug!(target: "epic", "Epic: registered repeating upkeep trigger for {:?}", card_id);
                }
            }

            // (a) Exile the card instead of graveyard.
            if self.stack.contains(card_id) {
                self.move_card(card_id, Zone::Stack, Zone::Exile, owner)?;
            }
            return Ok(());
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

        // CR 614 replacement: if `exile_if_would_go_to_graveyard_this_turn` is
        // set (from `Effect::PlayFromGraveyard` / Chandra −2's
        // `ReplaceGraveyard$ Exile`), redirect graveyard → exile instead.
        let destination = if destination == Zone::Graveyard
            && self
                .cards
                .get(card_id)
                .map(|c| c.exile_if_would_go_to_graveyard_this_turn)
                .unwrap_or(false)
        {
            log::debug!(
                target: "resolve_spell",
                "resolve_spell_finalize: card {} has exile_if_would_go_to_graveyard flag — exiling instead",
                card_id.as_u32()
            );
            Zone::Exile
        } else {
            destination
        };

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

            // CR 714.2: Sagas get a lore counter on ETB, immediately firing chapter I.
            // `advance_saga_lore_counter` is a no-op for non-Saga cards.
            self.advance_saga_lore_counter(card_id)?;

            // Check for ETB triggers on all permanents (including the one that just entered)
            self.check_triggers(TriggerEvent::EntersBattlefield, card_id)?;

            // CR 702.198: Offspring — if the caster paid the Offspring additional cost,
            // create a 1/1 token copy of this creature on the battlefield.
            self.create_offspring_token_if_paid(card_id)?;
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
                    Some(card_id),
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

        // --- EtbCounter keyword (generic: charge, P1P1, age, etc.) ---
        let etb_grant: Option<(CounterType, u8, String)> = {
            let Some(card) = self.cards.try_get(card_id) else {
                return Ok(());
            };
            let card_name_str = card.name.to_string();
            if let Some(KeywordArgs::EtbCounter {
                counter_type,
                amount,
                condition: _,
            }) = card.keywords.get_args(Keyword::EtbCounter)
            {
                let Some(ct) = CounterType::parse(counter_type) else {
                    log::warn!(
                        "apply_etb_counters: unknown counter type '{}' on {}",
                        counter_type,
                        card_name_str
                    );
                    return Ok(());
                };
                // Clone the fields we need before the borrow of `args` ends.
                let amount_str = amount.clone();
                let x_paid = card.x_paid;
                let times_kicked = card.times_kicked;
                // Clone SVars so we can look up SVar expressions without holding
                // a borrow on `card` (which is borrowed through `args`).
                let svars = card.svars.clone();
                // Resolve the counter amount: numeric literal, the symbolic "X"/"Y"
                // (X-cost spells, CR 107.3), or a named SVar (e.g. "XKicked" for
                // Multikicker cards, CR 702.33a).  For X-cost permanents like
                // Hangarback Walker, `x_paid` is set by the priority loop before the
                // spell resolves.  For Multikicker permanents like Everflowing Chalice,
                // `times_kicked` is set by the priority loop.
                let amt = match amount_str.parse::<u8>() {
                    Ok(n) => n,
                    Err(_) if amount_str.eq_ignore_ascii_case("X") || amount_str.eq_ignore_ascii_case("Y") => x_paid,
                    Err(_) => {
                        // Try looking up `amount_str` as a named SVar on the card itself
                        // (e.g. "XKicked" → SVar:XKicked:Count$TimesKicked).
                        if let Some(svar) = svars.get(&amount_str) {
                            let expr = crate::core::CountExpression::parse(&amount_str, &svars);
                            match expr {
                                crate::core::CountExpression::TimesKicked => times_kicked,
                                crate::core::CountExpression::Fixed(n) => n.max(0) as u8,
                                crate::core::CountExpression::ValidPermanents { .. }
                                | crate::core::CountExpression::CardsDrawnThisTurn
                                | crate::core::CountExpression::CardsInHand { .. }
                                | crate::core::CountExpression::XPaid
                                | crate::core::CountExpression::TargetedCardPower
                                | crate::core::CountExpression::TriggeredCardPower
                                | crate::core::CountExpression::SpellsCastThisTurn
                                | crate::core::CountExpression::ValidGraveyard { .. }
                                | crate::core::CountExpression::Compare { .. }
                                | crate::core::CountExpression::Kicked { .. }
                                | crate::core::CountExpression::Bargain { .. } => {
                                    log::warn!(
                                        "apply_etb_counters: SVar '{}' on {} resolves to unsupported \
                                         expression '{}' — skipping",
                                        amount_str,
                                        card_name_str,
                                        svar
                                    );
                                    return Ok(());
                                }
                            }
                        } else {
                            log::warn!(
                                "apply_etb_counters: non-numeric amount '{}' on {} not yet supported",
                                amount_str,
                                card_name_str
                            );
                            return Ok(());
                        }
                    }
                };
                Some((ct, amt, card_name_str))
            } else {
                None
            }
        };

        if let Some((counter_type, amount, card_name)) = etb_grant {
            if amount > 0 {
                self.logger.gamelog(&format!(
                    "{} enters the battlefield with {} {} counter{}",
                    card_name,
                    amount,
                    counter_type.display_name(),
                    if amount == 1 { "" } else { "s" }
                ));
                self.add_counters(card_id, counter_type, amount)?;
            }
        }

        // --- Fading / Vanishing: enter with N fade / time counters (CR 702.32 / CR 702.63) ---
        // These keywords are parsed as KeywordArgs::Fading { counters } /
        // KeywordArgs::Vanishing { counters } but have no EtbCounter entry, so
        // we handle them here explicitly.
        let fading_grant: Option<(CounterType, u8, String)> = {
            let Some(card) = self.cards.try_get(card_id) else {
                return Ok(());
            };
            if let Some(KeywordArgs::Fading { counters }) = card.keywords.get_args(Keyword::Fading) {
                Some((CounterType::Fade, *counters, card.name.to_string()))
            } else if let Some(KeywordArgs::Vanishing { counters }) = card.keywords.get_args(Keyword::Vanishing) {
                Some((CounterType::Time, *counters, card.name.to_string()))
            } else {
                None
            }
        };

        if let Some((counter_type, amount, card_name)) = fading_grant {
            if amount > 0 {
                self.logger.gamelog(&format!(
                    "{} enters the battlefield with {} {} counter{}",
                    card_name,
                    amount,
                    counter_type.display_name(),
                    if amount == 1 { "" } else { "s" }
                ));
                self.add_counters(card_id, counter_type, amount)?;
            }
        }

        Ok(())
    }

    /// Add one lore counter to a Saga and fire the corresponding chapter ability.
    ///
    /// Called (a) when a Saga enters the battlefield (fires chapter I) and (b) after
    /// the active player's mandatory draw each turn (fires the next chapter).
    ///
    /// After firing the final chapter (`lore_count >= max_chapter_number`), the Saga
    /// is sacrificed (CR 714.4).
    ///
    /// # Rules (CR 714)
    /// - 714.1: A Saga has chapter abilities triggered by lore counters.
    /// - 714.2: As a Saga enters the battlefield, its controller adds a lore counter.
    /// - 714.3: After the active player's mandatory draw, if they control a Saga,
    ///   they add a lore counter to it.
    /// - 714.4: After a Saga's final chapter ability has left the stack, the Saga's
    ///   controller sacrifices it.
    ///
    /// AI/network simplification: the chapter ability is executed immediately
    /// (no stack push) so we avoid implementing a full chapter-ability stack entry.
    /// This is consistent with how other triggered effects without target selection
    /// are handled in the engine. Targeted chapter abilities may therefore miss
    /// their targeting prompts and fizzle; this is a known limitation for the wave-4
    /// compatibility pass (mtg-901).
    pub(crate) fn advance_saga_lore_counter(&mut self, saga_id: CardId) -> Result<()> {
        use crate::core::{CounterType, KeywordArgs};

        // Guard: card must still be on battlefield.
        if !self.battlefield.cards.contains(&saga_id) {
            return Ok(());
        }

        // Collect chapter metadata before mutating the card.
        let (max_chapter, card_name) = {
            let card = match self.cards.try_get(saga_id) {
                Some(c) => c,
                None => return Ok(()),
            };
            // Sagas have exactly one Chapter keyword whose chapter_number is the
            // total number of chapters (all abilities listed in that one keyword).
            let mut max_chap = 0u8;
            for kw_args in card.keywords.iter_args() {
                if let KeywordArgs::Chapter { chapter_number, .. } = kw_args {
                    max_chap = max_chap.max(*chapter_number);
                }
            }
            if max_chap == 0 {
                // Not a real Saga (no Chapter keyword); skip silently.
                return Ok(());
            }
            (max_chap, card.name.to_string())
        };

        // Add 1 lore counter (CR 714.2 / 714.3).
        self.add_counters(saga_id, CounterType::Lore, 1)?;

        let lore_count = self
            .cards
            .try_get(saga_id)
            .map(|c| c.get_counter(CounterType::Lore))
            .unwrap_or(0);

        self.logger.gamelog(&format!(
            "{} gets a lore counter ({}/{})",
            card_name, lore_count, max_chapter
        ));

        // Fire the chapter ability for the current lore count.
        self.fire_saga_chapter(saga_id, lore_count)?;

        // CR 714.4: After the final chapter ability has left the stack (here:
        // immediately after firing since we execute inline), sacrifice the Saga.
        if lore_count >= max_chapter && self.battlefield.cards.contains(&saga_id) {
            let owner = self
                .cards
                .try_get(saga_id)
                .map(|c| c.owner)
                .unwrap_or_else(|| self.players.first().map(|p| p.id).unwrap_or_else(|| PlayerId::new(0)));
            let dest = self.death_destination_for_card(saga_id);
            self.logger
                .gamelog(&format!("{} is sacrificed (final chapter)", card_name));
            self.move_card(saga_id, Zone::Battlefield, dest, owner)?;
        }

        Ok(())
    }

    /// Execute the chapter ability for a Saga at the given `lore_count`.
    ///
    /// Looks up the SVar name from the `K:Chapter` keyword abilities list (comma-
    /// separated, 1-indexed by chapter number), then collects all `Effect`s from
    /// that SVar body following any `SubAbility$` chain, and executes them in order.
    fn fire_saga_chapter(&mut self, saga_id: CardId, lore_count: u8) -> Result<()> {
        use crate::core::KeywordArgs;

        // Collect the chapter abilities list and SVars before mutating.
        let (chapter_svar_names, card_name, svars): (Vec<String>, String, std::collections::HashMap<String, String>) = {
            let card = match self.cards.try_get(saga_id) {
                Some(c) => c,
                None => return Ok(()),
            };
            let mut abilities_str = None;
            for kw_args in card.keywords.iter_args() {
                if let KeywordArgs::Chapter { abilities, .. } = kw_args {
                    abilities_str = Some(abilities.clone());
                    break;
                }
            }
            let Some(abilities) = abilities_str else {
                return Ok(());
            };
            let svar_names: Vec<String> = abilities.split(',').map(|s| s.trim().to_string()).collect();
            (svar_names, card.name.to_string(), card.definition.svars.clone())
        };

        // Chapter numbering is 1-indexed; SVars are stored in order.
        let chapter_idx = lore_count.saturating_sub(1) as usize;
        let Some(svar_name) = chapter_svar_names.get(chapter_idx) else {
            log::debug!(
                "fire_saga_chapter: no SVar for chapter {} on {} (only {} chapters)",
                lore_count,
                card_name,
                chapter_svar_names.len()
            );
            return Ok(());
        };
        let Some(svar_body) = svars.get(svar_name.as_str()).cloned() else {
            log::debug!("fire_saga_chapter: SVar '{}' not found on {}", svar_name, card_name);
            return Ok(());
        };

        log::info!(
            "fire_saga_chapter: {} chapter {} ({}): {}",
            card_name,
            lore_count,
            svar_name,
            svar_body
        );

        // Collect all effects from the SVar body following SubAbility$ chains.
        let effects = collect_svar_chain_effects(&svar_body, &svars);

        if effects.is_empty() {
            log::debug!(
                "fire_saga_chapter: SVar '{}' on {} produced no executable effects (not yet supported)",
                svar_name,
                card_name
            );
        }

        for effect in &effects {
            // Resolve any self/controller placeholders using the Saga's controller.
            let resolved = if let Some(card) = self.cards.try_get(saga_id) {
                let ctrl = card.controller;
                let opp = self.players.iter().map(|p| p.id).find(|&id| id != ctrl);
                resolve_saga_effect_controller(effect.clone(), ctrl, opp)
            } else {
                effect.clone()
            };
            self.execute_effect(&resolved)?;
        }

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

        // Self-ReduceCost: apply ReduceCost static abilities carried on the card
        // being cast itself (ValidCard$ Card.Self | EffectZone$ All).
        // These reduce the card's own casting cost based on game state, and fire
        // even though the card is still in the hand (not yet on the battlefield).
        // Example: Eddymurk Crab — "costs {1} less for each instant/sorcery in
        // your graveyard" (SVar:X:Count$ValidGraveyard Instant.YouOwn,Sorcery.YouOwn).
        // CR 601.2e: cost reductions from the spell itself are applied first.
        for self_ability in &card.static_abilities {
            if let StaticAbility::ReduceCost {
                valid_card: crate::core::CostReductionTarget::SelfCard,
                amount,
                condition,
                description,
            } = self_ability
            {
                // Condition check (if any)
                let condition_met = if let Some(cond) = condition {
                    self.count_cards_matching_filter(player_id, &cond.is_present, cond.present_zone)
                        >= cond.min_count as usize
                } else {
                    true
                };

                if condition_met {
                    let reduce_by = match amount {
                        crate::core::CostReductionAmount::Fixed(n) => *n,
                        crate::core::CostReductionAmount::Dynamic(expr) => {
                            self.evaluate_count_expression(expr, player_id).unwrap_or(0).max(0) as u8
                        }
                    };

                    let old_generic = effective_cost.generic;
                    effective_cost.generic = effective_cost.generic.saturating_sub(reduce_by);

                    if old_generic != effective_cost.generic {
                        log::debug!(
                            "Self-ReduceCost on {}: {} (reducing generic by {}, was {}, now {})",
                            card.name,
                            description,
                            reduce_by,
                            old_generic,
                            effective_cost.generic
                        );
                    }
                }
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
                        // Resolve the reduction amount: fixed constant or dynamic
                        // CountExpression evaluated against the caster's game state.
                        // Dynamic example: Eddymurk Crab (`Amount$X` with
                        // `SVar:X:Count$ValidGraveyard Instant.YouOwn,Sorcery.YouOwn`).
                        let reduce_by = match amount {
                            crate::core::CostReductionAmount::Fixed(n) => *n,
                            crate::core::CostReductionAmount::Dynamic(expr) => {
                                self.evaluate_count_expression(expr, player_id).unwrap_or(0).max(0) as u8
                            }
                        };

                        let old_generic = effective_cost.generic;
                        effective_cost.generic = effective_cost.generic.saturating_sub(reduce_by);

                        if old_generic != effective_cost.generic {
                            log::debug!(
                                "ReduceCost from {}: {} (reducing generic by {}, was {}, now {})",
                                source_card.name,
                                description,
                                reduce_by,
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

        // Add Multikicker cost: times_kicked × kick_cost (CR 702.33a).
        // times_kicked is set by the priority loop (Step 2c) before this is called.
        if card.times_kicked > 0 {
            if let Some(crate::core::KeywordArgs::Multikicker { cost }) =
                card.keywords.get_args(crate::core::Keyword::Multikicker)
            {
                let kick_generic = cost.generic * card.times_kicked;
                let kick_white = cost.white * card.times_kicked;
                let kick_blue = cost.blue * card.times_kicked;
                let kick_black = cost.black * card.times_kicked;
                let kick_red = cost.red * card.times_kicked;
                let kick_green = cost.green * card.times_kicked;
                effective_cost.generic = effective_cost.generic.saturating_add(kick_generic);
                effective_cost.white = effective_cost.white.saturating_add(kick_white);
                effective_cost.blue = effective_cost.blue.saturating_add(kick_blue);
                effective_cost.black = effective_cost.black.saturating_add(kick_black);
                effective_cost.red = effective_cost.red.saturating_add(kick_red);
                effective_cost.green = effective_cost.green.saturating_add(kick_green);
            }
        }

        // Add Kicker cost (CR 702.32) when kicker_paid is set by Step 2c.5.
        if card.kicker_paid {
            if let Some(crate::core::KeywordArgs::Kicker { cost }) =
                card.keywords.get_args(crate::core::Keyword::Kicker)
            {
                effective_cost.generic = effective_cost.generic.saturating_add(cost.generic);
                effective_cost.white = effective_cost.white.saturating_add(cost.white);
                effective_cost.blue = effective_cost.blue.saturating_add(cost.blue);
                effective_cost.black = effective_cost.black.saturating_add(cost.black);
                effective_cost.red = effective_cost.red.saturating_add(cost.red);
                effective_cost.green = effective_cost.green.saturating_add(cost.green);
            }
        }

        // Add ModeCost extra (tiered modal spells like Fire Magic: Fire={0}/Fira={2}/Firaga={5}).
        // mode_cost_paid is set by apply_selected_modes after mode selection in the priority loop.
        if card.mode_cost_paid > 0 {
            effective_cost.generic = effective_cost.generic.saturating_add(card.mode_cost_paid);
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

        // Comma-separated filter (e.g. "Instant.YouOwn,Sorcery.YouOwn"): a card
        // matches if it matches ANY of the comma-separated parts. Sum across
        // distinct parts (a card can only match once — we count distinct cards).
        // Implementation: collect into a set of matching card ids, then count.
        // Simple/fast: iterate zone cards once per part, use a HashSet to dedup.
        if filter.contains(',') {
            let mut matched: std::collections::HashSet<crate::core::CardId> = std::collections::HashSet::new();
            for part in filter.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                // Re-use single-filter logic by calling ourselves recursively.
                // To count unique matching cards across all parts we can't just
                // sum, so instead we collect matching card ids per part.
                // As an approximation (good enough for dedup), rebuild per-zone
                // ids here and re-check the single-part filter via a local closure.
                let zone_cards: &[crate::core::CardId] = match zone {
                    Zone::Graveyard => {
                        if let Some(zones) = self.get_player_zones(player_id) {
                            zones.graveyard.cards.as_slice()
                        } else {
                            continue;
                        }
                    }
                    Zone::Hand => {
                        if let Some(zones) = self.get_player_zones(player_id) {
                            zones.hand.cards.as_slice()
                        } else {
                            continue;
                        }
                    }
                    Zone::Battlefield => self.battlefield.cards.as_slice(),
                    Zone::Exile => {
                        if let Some(zones) = self.get_player_zones(player_id) {
                            zones.exile.cards.as_slice()
                        } else {
                            continue;
                        }
                    }
                    Zone::Library => {
                        if let Some(zones) = self.get_player_zones(player_id) {
                            zones.library.cards.as_slice()
                        } else {
                            continue;
                        }
                    }
                    Zone::Stack | Zone::Command => continue,
                };
                // Build a temporary single-filter count and collect matching ids.
                // We recurse with the single part to reuse all the type-matching
                // logic; then track which cards match via a secondary scan.
                let single_count = self.count_cards_matching_filter(player_id, part, zone);
                if single_count == 0 {
                    continue;
                }
                // To get the actual card ids, re-scan with the same single-filter.
                // parse the single-part filter inline (mirrors the logic below).
                let mut sections = part.splitn(2, '.');
                let type_filter = sections.next().unwrap_or("");
                let quals: Vec<&str> = sections.next().map(|q| q.split('+').collect()).unwrap_or_default();
                let ownership = quals
                    .iter()
                    .copied()
                    .find(|q| matches!(*q, "YouOwn" | "OppOwn" | "YouCtrl" | "OppCtrl"))
                    .unwrap_or("YouOwn");
                for &cid in zone_cards {
                    let Some(c) = self.cards.try_get(cid) else {
                        continue;
                    };
                    let ownership_ok = match ownership {
                        "YouOwn" => c.owner == player_id,
                        "OppOwn" => c.owner != player_id,
                        "YouCtrl" => c.controller == player_id,
                        "OppCtrl" => c.controller != player_id,
                        _ => true,
                    };
                    if !ownership_ok {
                        continue;
                    }
                    let type_ok = match type_filter {
                        "" | "Card" | "Permanent" => true,
                        "Land" => c.is_land(),
                        "Artifact" => c.is_artifact(),
                        "Creature" => c.is_creature(),
                        "Enchantment" => c.is_enchantment(),
                        "Instant" => c.types.contains(&crate::core::CardType::Instant),
                        "Sorcery" => c.types.contains(&crate::core::CardType::Sorcery),
                        "Planeswalker" => c.types.contains(&crate::core::CardType::Planeswalker),
                        other => c.subtypes.contains(&crate::core::Subtype::new(other)),
                    };
                    if type_ok {
                        matched.insert(cid);
                    }
                }
            }
            return matched.len();
        }

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
                // card types (Land/Artifact/Creature/Enchantment/Instant/Sorcery/…)
                // match c.types; otherwise treat as a creature subtype.
                let type_ok = match type_filter {
                    "" | "Card" | "Permanent" => true,
                    "Land" => c.is_land(),
                    "Artifact" => c.is_artifact(),
                    "Creature" => c.is_creature(),
                    "Enchantment" => c.is_enchantment(),
                    "Instant" => c.types.contains(&crate::core::CardType::Instant),
                    "Sorcery" => c.types.contains(&crate::core::CardType::Sorcery),
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
    /// Whether `card` matches a simple bare type/subtype filter string: one of
    /// the permanent card types (`Land`/`Creature`/`Artifact`/`Enchantment`),
    /// `Permanent` (any), else treated as a subtype. This is the ONE shared
    /// implementation for the "count/find permanents of a controlled type"
    /// helpers (mtg-907 DRY: the byte-identical `GameLoop::card_matches_type_filter`
    /// copy was deleted and routed here).
    ///
    /// NOTE: this is the *simple* type/subtype filter (no `.`/`+` qualifier
    /// grammar). It is distinct from `card_matches_search_filter` (the
    /// `Type.Subtype` / comma-list fetch grammar) and from
    /// `TargetRestriction::parse` (the `ValidTgts` `.`-modifier grammar) — those
    /// are genuinely different DSLs, not duplicates of this one.
    pub(crate) fn card_matches_type_filter_static(card: &crate::core::Card, type_filter: &str) -> bool {
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
            Zone::Library => {
                // Casting from the top of library (Experimental Frenzy, Future Sight).
                // The card must be the top card of the controller's library.
                let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(player_id);
                let is_top = self
                    .get_player_zones(owner)
                    .and_then(|z| z.library.cards.last().copied())
                    == Some(card_id);
                if !is_top {
                    return Err(MtgError::InvalidAction("Card is not the top of library".to_string()));
                }
            }
            Zone::Battlefield | Zone::Graveyard | Zone::Stack => {
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
    #[allow(clippy::too_many_arguments)]
    fn resolve_effect_target(
        &self,
        effect: &Effect,
        chosen_targets: &[CardId],
        target_index: &mut usize,
        card_owner: PlayerId,
        opponent_id: Option<PlayerId>,
        last_resolved_target: &mut Option<CardId>,
        source_card_id: Option<CardId>,
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
            // `Count$ValidGraveyard Lesson.YouOwn/Plus.2`, or Torch the Tower's
            // `Count$Bargain.3.2`) against the caster's controller right now,
            // then produce a concrete DealDamage so execute_effect can run
            // without needing the controller context.
            Effect::DealDamageDynamic {
                target: TargetRef::None,
                count,
            } => {
                let amount = self
                    .evaluate_count_with_source(count, card_owner, source_card_id)
                    .unwrap_or(0)
                    .max(0);
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
                let amount = self
                    .evaluate_count_with_source(count, card_owner, source_card_id)
                    .unwrap_or(0)
                    .max(0);
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
                keyword_args_granted,
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
                        keyword_args_granted: keyword_args_granted.clone(),
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
                keyword_args_granted,
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
                        keyword_args_granted: keyword_args_granted.clone(),
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
            Effect::ReturnPermanentToHand { target, restriction } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    // Record bounced permanent so chained SubAbilities (e.g. Teferi's draw)
                    // can find it via last_resolved_target / Defined$ TargetedController.
                    *last_resolved_target = Some(resolved_target);
                    Effect::ReturnPermanentToHand {
                        target: resolved_target,
                        restriction: restriction.clone(),
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
            // PlayFromGraveyard: bind the chosen graveyard card to the effect's target.
            Effect::PlayFromGraveyard {
                target,
                exile_on_resolution,
                type_filter,
                max_mana_value,
            } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::PlayFromGraveyard {
                        target: resolved_target,
                        exile_on_resolution: *exile_on_resolution,
                        type_filter: type_filter.clone(),
                        max_mana_value: *max_mana_value,
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
            // "Put N cards from your hand on top of your library" (Brainstorm sub-ability).
            // Always targets the spell's controller. Resolve placeholder here.
            Effect::PutCardsFromHandOnTopOfLibrary { player, count } if player.is_placeholder() => {
                Effect::PutCardsFromHandOnTopOfLibrary {
                    player: card_owner,
                    count: *count,
                }
            }
            // "Reveal any number of cards from your hand" (Metalworker-style Reveal ability).
            // Always targets the ability's controller. Resolve placeholder here.
            Effect::RevealCardsFromHand {
                player,
                filter,
                any_number,
                remember_count,
            } if player.is_placeholder() => Effect::RevealCardsFromHand {
                player: card_owner,
                filter: filter.clone(),
                any_number: *any_number,
                remember_count: *remember_count,
            },
            // "Return one matching card from your graveyard to hand" — also targets
            // the spell/trigger controller by default. Resolve placeholder.
            Effect::ReturnGraveyardCardToHand { player, type_filter } if player.is_placeholder() => {
                Effect::ReturnGraveyardCardToHand {
                    player: card_owner,
                    type_filter: type_filter.clone(),
                }
            }
            // "Return one matching card from graveyard to any zone" — resolve
            // player placeholder to the spell/trigger controller.
            Effect::ReturnGraveyardCardToZone {
                player,
                type_filter,
                destination,
                gain_control,
                library_position,
                remember_changed,
            } if player.is_placeholder() => Effect::ReturnGraveyardCardToZone {
                player: card_owner,
                type_filter: type_filter.clone(),
                destination: *destination,
                gain_control: *gain_control,
                library_position: *library_position,
                remember_changed: *remember_changed,
            },
            // "Put a creature from your hand onto the battlefield" (Sneak Attack).
            // Resolve player placeholder to the ability's controller.
            Effect::PutCreatureFromHandOnBattlefield {
                player,
                type_filter,
                remember_changed,
            } if player.is_placeholder() => Effect::PutCreatureFromHandOnBattlefield {
                player: card_owner,
                type_filter: type_filter.clone(),
                remember_changed: *remember_changed,
            },
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
            Effect::SetLife { player, amount } if player.is_placeholder() => {
                // If a target was chosen (Sorin Markov -3: "target opponent's life total
                // becomes 10"), consume the chosen target from the list and decode the
                // player sentinel. If no target was chosen, default to card_owner
                // (Angel of Grace: "your life total becomes 10"). (CR 119.5)
                let resolved_player = if *target_index < chosen_targets.len() {
                    let raw = chosen_targets[*target_index];
                    if let Some(pid) = crate::core::player_target_from_sentinel(raw) {
                        *target_index += 1;
                        *last_resolved_target = Some(raw);
                        pid
                    } else {
                        // Target is a permanent, not a player — fall back to card_owner
                        card_owner
                    }
                } else {
                    card_owner
                };
                Effect::SetLife {
                    player: resolved_player,
                    amount: *amount,
                }
            }
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
            Effect::Scry {
                player,
                count,
                only_if_bargained,
            } if player.is_placeholder() => Effect::Scry {
                player: card_owner,
                count: *count,
                only_if_bargained: *only_if_bargained,
            },
            Effect::Surveil { player, count } if player.is_placeholder() => Effect::Surveil {
                player: card_owner,
                count: *count,
            },
            // RearrangeTopOfLibrary — Defined$ You → controller (card_owner).
            Effect::RearrangeTopOfLibrary { player, count } if player.is_placeholder() => {
                Effect::RearrangeTopOfLibrary {
                    player: card_owner,
                    count: *count,
                }
            }
            // SkipUntapStep — ValidTgts$ Player → opponent; Defined$ You → self.
            Effect::SkipUntapStep { player } if player.is_placeholder() => Effect::SkipUntapStep { player: card_owner },
            Effect::SkipUntapStep { player } if player.is_target_opponent() => Effect::SkipUntapStep {
                player: opponent_id.unwrap_or(card_owner),
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
                    source_card_id,
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

            // Resolve CreateTokenWithStoredPt placeholders.  The loader emits
            // `source_card: CardId::placeholder()` and `controller:
            // PlayerId::new(0)` since the activating card is not known at parse
            // time.  At resolution we substitute the actual source card (the
            // Phyrexian Processor) and its controller.
            Effect::CreateTokenWithStoredPt {
                source_card,
                controller,
                token_script,
            } => {
                let resolved_source = if source_card.is_placeholder() {
                    source_card_id.unwrap_or(*source_card)
                } else {
                    *source_card
                };
                let resolved_controller = if controller.is_placeholder() {
                    card_owner
                } else {
                    *controller
                };
                Effect::CreateTokenWithStoredPt {
                    source_card: resolved_source,
                    controller: resolved_controller,
                    token_script: token_script.clone(),
                }
            }

            // Resolve CreateEmblem controller placeholder to the actual caster.
            // The loader sets controller to PlayerId::new(0) as placeholder;
            // at runtime we resolve it to the spell's owner (the planeswalker's
            // controller who activated the ultimate).
            Effect::CreateEmblem {
                controller,
                emblem_name,
                static_abilities,
                triggers,
            } if controller.is_placeholder() => Effect::CreateEmblem {
                controller: card_owner,
                emblem_name: emblem_name.clone(),
                static_abilities: static_abilities.clone(),
                triggers: triggers.clone(),
            },

            // RepeatEach (Pattern A): fill in the chosen targets so execute_effect
            // can iterate over them.  The targets list starts empty at parse time
            // and is populated here once we know the spell's chosen_targets.
            // Pattern B (AllPlayers) carries no targets so it passes through unchanged.
            Effect::RepeatEach {
                sub_effects,
                iterate_over:
                    crate::core::RepeatEachIterate::Cards {
                        targets,
                        require_in_graveyard,
                    },
            } if targets.is_empty() && !chosen_targets.is_empty() => {
                // Determine which chosen targets belong to this RepeatEach.
                // Two cases:
                // (a) RepeatEach appears BEFORE the Destroy/ChangeZone effects in
                //     the chain (unusual): take remaining chosen targets
                //     (chosen_targets[target_index..]).
                // (b) RepeatEach appears AFTER the consuming effects (Terastodon):
                //     the Destroy effects already advanced target_index past all
                //     chosen targets, so chosen_targets[target_index..] is empty.
                //     In this case, iterate over ALL chosen targets (0..target_index),
                //     since RepeatEach with DefinedCards$ Targeted means "for each of
                //     the targets we chose at cast time", not "new targets". target_index
                //     is not advanced (RepeatEach does not consume additional targets).
                let remaining = &chosen_targets[*target_index..];
                let resolved_targets = if remaining.is_empty() {
                    // Terastodon case: consuming effects ran first; re-use all chosen targets.
                    chosen_targets.to_vec()
                } else {
                    // Standard case: take remaining targets and mark them consumed.
                    let v = remaining.to_vec();
                    *target_index = chosen_targets.len();
                    v
                };
                Effect::RepeatEach {
                    sub_effects: sub_effects.clone(),
                    iterate_over: crate::core::RepeatEachIterate::Cards {
                        targets: resolved_targets,
                        require_in_graveyard: *require_in_graveyard,
                    },
                }
            }

            // ExtraLandPlay: resolve placeholder player to the spell's controller.
            // Explore / similar spells encode `Defined$ You` (player 0) here.
            Effect::ExtraLandPlay { player, amount } if player.is_placeholder() => Effect::ExtraLandPlay {
                player: card_owner,
                amount: *amount,
            },

            // GrantCastWithFlash: resolve placeholder player to the spell's controller.
            Effect::GrantCastWithFlash { player, valid_card } if player.is_placeholder() => {
                Effect::GrantCastWithFlash {
                    player: card_owner,
                    valid_card: valid_card.clone(),
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
            } => self.execute_destroy_permanent(*target, *no_regenerate)?,
            Effect::GainControl {
                target,
                new_controller,
                untap,
                duration,
                source,
                ..
            } => self.execute_gain_control(*target, *new_controller, *untap, duration, *source)?,
            Effect::Fight { fighter, target } => self.execute_fight(*fighter, *target)?,
            Effect::TapPermanent { target } => self.execute_tap_permanent(*target)?,
            Effect::TapPermanentsMatchingFilter {
                player,
                choices_filter,
                count,
            } => self.execute_tap_permanents_matching_filter(*player, choices_filter, *count)?,
            Effect::UntapPermanent { target } => self.execute_untap_permanent(*target)?,
            Effect::TapOrUntapPermanent { target } => self.execute_tap_or_untap_permanent(*target)?,
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
                keywords_granted,
                keyword_args_granted,
            } => self.execute_pump_creature(
                *target,
                *power_bonus,
                *toughness_bonus,
                keywords_granted,
                keyword_args_granted,
            )?,
            Effect::PumpCreatureVariable {
                target,
                power_count,
                toughness_count,
                keywords_granted,
                keyword_args_granted,
            } => self.execute_pump_creature_variable(
                *target,
                power_count,
                toughness_count,
                keywords_granted,
                keyword_args_granted,
            )?,
            Effect::DebuffCreature {
                target,
                keywords_removed,
            } => self.execute_debuff_creature(*target, keywords_removed)?,
            Effect::PumpAllCreatures {
                controller,
                filter,
                power_bonus,
                toughness_bonus,
            } => self.execute_pump_all_creatures(*controller, filter, *power_bonus, *toughness_bonus)?,
            Effect::AnimateAll {
                controller,
                filter,
                power,
                toughness,
                keywords_granted,
                keyword_args_granted,
            } => self.execute_animate_all(
                *controller,
                filter,
                *power,
                *toughness,
                keywords_granted,
                keyword_args_granted,
            )?,
            Effect::Mill { player, count } => self.execute_mill(*player, *count)?,
            Effect::DrainMana { player } => self.execute_drain_mana(*player)?,
            Effect::Scry {
                player,
                count,
                only_if_bargained,
            } => {
                // Condition$ Bargain: only scry if the source spell was bargained
                // (CR 702.162). We use current_damage_source as the source card
                // reference because the scry fires as a SubAbility of DealDamage
                // (Torch the Tower) while the damage source is still set.
                if *only_if_bargained {
                    let is_bargained = self
                        .current_damage_source
                        .and_then(|id| self.cards.try_get(id))
                        .is_some_and(|c| c.bargain_paid);
                    if !is_bargained {
                        // Condition not met — skip the scry silently.
                    } else {
                        self.execute_scry(*player, *count)?;
                    }
                } else {
                    self.execute_scry(*player, *count)?;
                }
            }
            Effect::Surveil { player, count } => self.execute_surveil(*player, *count)?,
            Effect::AddTurn { player, num_turns } => self.execute_add_turn(*player, *num_turns)?,
            Effect::CounterSpell {
                target,
                remember_mana_value,
                ..
            } => self.execute_counter_spell(*target, *remember_mana_value)?,
            Effect::AddMana {
                player,
                mana,
                produces_chosen_color,
                amount_var,
            } => self.execute_add_mana(*player, mana, *produces_chosen_color, amount_var.as_deref())?,
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } => self.execute_put_counter(*target, *counter_type, *amount)?,
            Effect::MultiplyCounter {
                target,
                counter_type,
                multiplier,
            } => self.execute_multiply_counter(*target, *counter_type, *multiplier)?,
            Effect::PutCounterAll {
                restriction,
                counter_type,
                amount,
            } => self.execute_put_counter_all(restriction, *counter_type, *amount)?,
            Effect::Proliferate => self.execute_proliferate()?,
            Effect::ChangeZoneAll {
                restriction,
                origins,
                destination,
                shuffle,
            } => self.execute_change_zone_all(restriction, origins, *destination, *shuffle)?,
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } => self.execute_remove_counter(*target, *counter_type, *amount)?,
            Effect::ReturnPermanentToHand { target, .. } => self.execute_return_permanent_to_hand(*target)?,
            Effect::ExilePermanent { target } => self.execute_exile_permanent(*target)?,
            Effect::ExileIfWouldDieThisTurn { target } => self.execute_exile_if_would_die_this_turn(*target)?,
            Effect::PlayFromGraveyard {
                target,
                exile_on_resolution,
                ..
            } => self.execute_play_from_graveyard(*target, *exile_on_resolution)?,
            Effect::SelfExileFromStack {
                source,
                remember_changed,
            } => self.execute_self_exile_from_stack(*source, *remember_changed)?,
            Effect::MoveSelfBetweenZones {
                source,
                origin,
                destination,
            } => self.execute_move_self_between_zones(*source, *origin, *destination)?,
            Effect::ReturnCardsFromGraveyardToHand { player } => {
                self.execute_return_cards_from_graveyard_to_hand(*player)?
            }
            Effect::PutCardsFromHandOnTopOfLibrary { player, count } => {
                // Non-interactive fallback: pick the lowest-CMC cards from hand.
                // The interactive path (priority.rs) overrides this with
                // controller-chosen cards before calling
                // execute_put_cards_from_hand_on_top_of_library directly.
                let hand: smallvec::SmallVec<[CardId; 8]> = self
                    .get_player_zones(*player)
                    .map(|z| z.hand.cards.iter().copied().collect())
                    .unwrap_or_default();
                let chosen = self.pick_cards_to_put_back_heuristic(&hand, *count as usize);
                self.execute_put_cards_from_hand_on_top_of_library(*player, &chosen)?;
            }
            Effect::RevealCardsFromHand {
                player,
                filter,
                any_number: _,
                remember_count,
            } => {
                // Count matching cards in hand, reveal them (log), and optionally
                // store the count in remembered_amount for chained sub-abilities.
                // Non-interactive: we reveal ALL matching cards (the heuristic equivalent
                // of "reveal as many as possible to maximise the Mana sub-ability").
                self.execute_reveal_cards_from_hand(*player, filter, *remember_count)?;
            }
            Effect::ReturnGraveyardCardToHand { player, type_filter } => {
                self.execute_return_graveyard_card_to_hand(*player, type_filter)?
            }
            Effect::ReturnGraveyardCardToZone {
                player,
                type_filter,
                destination,
                gain_control,
                library_position,
                remember_changed,
            } => self.execute_return_graveyard_card_to_zone(
                *player,
                type_filter,
                *destination,
                *gain_control,
                *library_position,
                *remember_changed,
            )?,

            Effect::PutCreatureFromHandOnBattlefield {
                player,
                type_filter,
                remember_changed,
            } => self.execute_put_creature_from_hand_on_battlefield(*player, type_filter, *remember_changed)?,

            Effect::ReturnSelfAsEnchantment { source } => self.execute_return_self_as_enchantment(*source)?,

            Effect::ConditionalSelfCounter {
                source,
                condition,
                inner,
            } => self.execute_conditional_self_counter(*source, condition, inner)?,
            Effect::SetBasePowerToughness {
                target,
                power,
                toughness,
                keywords_granted,
                keyword_args_granted,
                types_added,
                subtypes_added,
                remove_creature_subtypes,
                at_eot,
            } => self.execute_set_base_power_toughness(
                *target,
                *power,
                *toughness,
                keywords_granted,
                keyword_args_granted,
                types_added,
                subtypes_added,
                *remove_creature_subtypes,
                *at_eot,
            )?,
            Effect::SearchLibrary {
                player,
                card_type_filter,
                destination,
                enters_tapped,
                shuffle,
            } => self.execute_search_library(*player, card_type_filter, *destination, *enters_tapped, *shuffle)?,
            Effect::AttachEquipment {
                source_equipment,
                target_creature,
            } => self.execute_attach_equipment(*source_equipment, *target_creature)?,
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
            } => self.execute_create_token(*controller, token_script, *amount, *for_each_player)?,

            Effect::CreateTokenWithStoredPt {
                source_card,
                controller,
                token_script,
            } => self.execute_create_token_with_stored_pt(*source_card, *controller, token_script)?,

            Effect::Airbend { target } => self.execute_airbend(*target)?,

            Effect::Earthbend { target, num_counters } => self.execute_earthbend(*target, *num_counters)?,

            Effect::Firebend { controller, amount } => self.execute_firebend(*controller, *amount)?,

            Effect::GrantCantBeBlocked { target } => self.execute_grant_cant_be_blocked(*target)?,

            Effect::ExtraLandPlay { player, amount } => self.execute_extra_land_play(*player, *amount)?,
            Effect::GrantCastWithFlash { player, valid_card } => {
                self.execute_grant_cast_with_flash(*player, valid_card.clone())?
            }

            Effect::Regenerate { target } => self.execute_regenerate(*target)?,

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
                cmc_eq_source,
            } => {
                // Resolve dynamic `cmcEQX` SVar from the source card's charge-counter
                // count (Ratchet Bomb). If `cmc_eq_svar` is set and a source is known,
                // materialise `exact_cmc` on the restriction clone before matching.
                let resolved_restriction;
                let effective_restriction = if restriction.cmc_eq_svar {
                    if let Some(source_id) = cmc_eq_source {
                        let charge_count = self
                            .cards
                            .try_get(*source_id)
                            .map(|c| c.get_counter(crate::core::CounterType::Charge))
                            .unwrap_or(0);
                        let mut r = restriction.clone();
                        r.exact_cmc = Some(charge_count);
                        r.cmc_eq_svar = false; // resolved; no further SVar lookup needed
                        resolved_restriction = r;
                        &resolved_restriction
                    } else {
                        restriction
                    }
                } else {
                    restriction
                };
                self.execute_destroy_all(effective_restriction, *no_regenerate)?
            }

            Effect::SacrificeAll { restriction } => self.execute_sacrifice_all(restriction)?,

            Effect::DamageAll {
                amount,
                valid_cards,
                damage_players,
            } => self.execute_damage_all(*amount, valid_cards, *damage_players)?,

            Effect::ForceSacrifice {
                player,
                sac_type,
                count,
            } => self.execute_force_sacrifice(*player, sac_type, *count)?,

            // SacrificeSelf: sacrifice the source card itself.
            // Resolved from placeholder at phase-trigger fire time; if the
            // source has already left the battlefield (removed in response),
            // silently skip (CR 603.10 / 608.2c fizzle rule).
            Effect::SacrificeSelf { source } => {
                if self.battlefield.cards.contains(source) {
                    let owner = self
                        .cards
                        .get(*source)
                        .map(|c| c.owner)
                        .unwrap_or_else(|_| self.players.first().map(|p| p.id).unwrap_or_else(|| PlayerId::new(0)));
                    let dest = self.death_destination_for_card(*source);
                    self.move_card(*source, Zone::Battlefield, dest, owner)?;
                    log::debug!("SacrificeSelf: {:?} moved to {:?}", source, dest);
                } else {
                    log::debug!("SacrificeSelf: {:?} not on battlefield — fizzle", source);
                }
            }

            Effect::TapAll { restriction } => self.execute_tap_all(restriction)?,
            Effect::UntapAll { restriction } => self.execute_untap_all(restriction)?,
            Effect::UntapOne { restriction } => self.execute_untap_one(restriction)?,

            Effect::SetLife { player, amount } => self.execute_set_life(*player, *amount)?,

            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } => self.execute_create_delayed_trigger(*tracked_card, condition, delayed_effect, expiry)?,

            Effect::ModalChoice { modes, .. } => self.execute_modal_choice_fallback(modes.len())?,

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
                self.execute_copy_permanent(*target, *controller, *set_power, *set_toughness, add_types, *num_copies)?
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
            } => self.execute_dig(
                *dig_count,
                *change_count,
                *change_all,
                *destination,
                *rest_destination,
                *may_play,
                *may_play_without_mana_cost,
                *target_self,
                *optional,
                *rest_random,
                *reveal,
                change_valid,
            )?,

            Effect::CopySpellAbility {
                may_choose_targets,
                defined_source,
                controller,
            } => self.execute_copy_spell_ability(*may_choose_targets, defined_source, controller.as_deref())?,
            Effect::ImmediateTrigger { condition, sub_effects } => {
                self.execute_immediate_trigger(condition, sub_effects)?
            }
            Effect::ClearRemembered => self.execute_clear_remembered()?,

            // ChooseAndRememberOneOfEach: for each type in the list, choose one
            // permanent of that type controlled by the current loop player
            // (remembered_players[0]) and push it onto remembered_cards.
            Effect::ChooseAndRememberOneOfEach { types } => self.execute_choose_and_remember_one_of_each(types)?,

            // RepeatEach: for each member of iterate_over, set it as the
            // remembered card/player, then execute each sub-effect once.
            // CR 609.3: actions repeat sequentially for each member.
            Effect::RepeatEach {
                sub_effects,
                iterate_over,
            } => self.execute_repeat_each(sub_effects, iterate_over)?,

            Effect::UnlessCostWrapper {
                inner_effect,
                unless_cost,
            } => self.execute_unless_cost_wrapper(inner_effect, unless_cost)?,

            Effect::ChooseColor { player, source } => self.execute_choose_color(*player, *source)?,

            Effect::AddPhase { count } => self.execute_add_phase(*count)?,
            Effect::Clone { .. } => self.execute_clone_fallback()?,
            Effect::RearrangeTopOfLibrary { player, count } => {
                self.execute_rearrange_top_of_library(*player, *count)?
            }
            Effect::SkipUntapStep { player } => self.execute_skip_untap_step(*player)?,
            Effect::Unimplemented { api_type } => self.execute_unimplemented(api_type)?,
            Effect::NoOp { api_type } => self.execute_noop(api_type)?,

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

            // CreateTokenDynamic should be resolved to CreateToken by
            // resolve_effect_placeholder() in the trigger-fire path before
            // reaching execute_effect.  If it arrives here unresolved (e.g.
            // a non-death-trigger path that doesn't call
            // resolve_effect_placeholder), fall back to amount=1 so the card
            // does something visible rather than silently no-op.
            Effect::CreateTokenDynamic {
                controller,
                token_script,
                amount: crate::core::DynamicAmount::Count(expr),
                for_each_player,
            } => {
                // Count$… expression (e.g. Avenger of Zendikar: TokenAmount$ X,
                // SVar:X:Count$Valid Land.YouCtrl). Evaluate the count expression
                // against the controller's current game state at resolution time.
                let n = self.evaluate_count_expression(expr, *controller).unwrap_or(0).max(0) as u8;
                self.execute_create_token(*controller, token_script, n, *for_each_player)?;
            }

            Effect::CreateTokenDynamic {
                controller,
                token_script,
                for_each_player,
                ..
            } => {
                log::warn!(
                    "CreateTokenDynamic reached execute_effect unresolved — \
                     falling back to amount=1 (token: {})",
                    token_script
                );
                self.execute_create_token(*controller, token_script, 1, *for_each_player)?;
            }

            Effect::CreateEmblem {
                controller,
                emblem_name,
                static_abilities,
                triggers,
            } => {
                self.execute_create_emblem(*controller, emblem_name, static_abilities, triggers)?;
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
        // Comma-separated filter: a card matches if it matches ANY comma-part.
        // Each part is itself a full filter, so this recurses — a part may be a
        // card TYPE (`Instant`), a land SUBTYPE (`Plains`), or a dotted
        // `Type.Subtype` (`Land.Basic`). (mtg-907)
        //
        // The previous code hard-required `card.is_land()` for EVERY comma-list,
        // assuming all comma-lists were fetch-land subtype lists (`Plains,Island`).
        // That silently broke every comma-separated TYPE list — most visibly
        // `ChangeType$ Instant,Sorcery | Origin$ Library` (Goblin Tutor's "search
        // for an instant or sorcery" mode, Knowledge Exploitation): an instant is
        // not a land, so the whole search returned nothing. Recursing per-part
        // matches the land-subtype lists identically (each `Plains`/`Island`
        // single-word part still routes through the land-subtype branch below)
        // while ALSO correctly matching type lists. CR 109.1/205 (card types),
        // CR 701.19a (search a zone by a stated quality).
        if filter.contains(',') {
            return filter
                .split(',')
                .any(|part| Self::card_matches_search_filter(card, part.trim()));
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
                        // Strip ".Other" modifier for base type matching
                        let base_pattern = p.trim_end_matches(".Other");

                        // Use the shared type-filter helper which handles both
                        // main types (Land, Creature, Artifact) and subtypes
                        // (Forest, Island, Mountain, etc.)
                        let mut matches = Self::card_matches_type_filter_static(card, base_pattern);

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

        // Torpor Orb check (CR 603.6b): if a DisableCreatureEtbTriggers static is in
        // play and the triggering event is a creature entering the battlefield, ALL
        // EntersBattlefield triggers are suppressed for that creature. We pre-compute
        // this flag so the Phase 1 filter can short-circuit without re-scanning the
        // battlefield for every candidate trigger.
        let creature_etb_suppressed = event == TriggerEvent::EntersBattlefield
            && self
                .cards
                .try_get(source_card_id)
                .is_some_and(|c| self.is_creature_etb_trigger_suppressed(c));

        if creature_etb_suppressed {
            if let Some(c) = self.cards.try_get(source_card_id) {
                log::info!(
                    "Torpor Orb: suppressing all ETB triggers for creature {} (CR 603.6b)",
                    c.name
                );
            }
            return Ok(());
        }

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

            // Accumulate all DestroyPermanent targets chosen during this trigger's
            // effect chain. Used by RepeatEach with DefinedCards$ Targeted to iterate
            // over every permanent the Destroy effect picked — e.g. Terastodon's ETB
            // destroys up to 3 permanents then creates one 3/3 Elephant token per
            // destroyed permanent for its controller. (mtg-914 B2 fix: RepeatEach
            // iterates over trigger_destroy_targets when its own targets list is empty.)
            let mut trigger_destroy_targets: smallvec::SmallVec<[CardId; 4]> = smallvec::SmallVec::new();

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
                            // Record for RepeatEach (DefinedCards$ Targeted / ChangeZoneTable$ True)
                            // so Terastodon's token sub-ability can iterate over all destroyed targets.
                            trigger_destroy_targets.push(target_id);
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
                        keyword_args_granted,
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
                                keyword_args_granted: keyword_args_granted.clone(),
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
                                    keyword_args_granted: keyword_args_granted.clone(),
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

                // RepeatEach (Pattern A — DefinedCards$ Targeted) in a trigger: if the
                // targets list is empty and DestroyPermanent already picked targets in
                // this same trigger chain, fill in those targets now. This covers
                // Terastodon's ETB: "destroy up to 3 noncreature permanents; for each,
                // its controller creates a 3/3 Elephant token" (mtg-914 B2 fix).
                if let Effect::RepeatEach {
                    sub_effects,
                    iterate_over:
                        crate::core::RepeatEachIterate::Cards {
                            ref targets,
                            require_in_graveyard,
                        },
                } = effect.clone()
                {
                    if targets.is_empty() && !trigger_destroy_targets.is_empty() {
                        effect = Effect::RepeatEach {
                            sub_effects,
                            iterate_over: crate::core::RepeatEachIterate::Cards {
                                targets: trigger_destroy_targets.to_vec(),
                                require_in_graveyard,
                            },
                        };
                    }
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
            // Accumulate DestroyPermanent targets for RepeatEach (DefinedCards$ Targeted).
            // See check_triggers for detailed comments; same logic applies here.
            let mut trigger_destroy_targets: smallvec::SmallVec<[CardId; 4]> = smallvec::SmallVec::new();

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
                                // Record for RepeatEach (DefinedCards$ Targeted / ChangeZoneTable$ True)
                                // so Terastodon's token sub-ability can iterate over all destroyed targets.
                                trigger_destroy_targets.push(chosen_id);
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
                    Effect::ReturnPermanentToHand {
                        target: ref mut target_id,
                        restriction: ref restr,
                    } => {
                        if target_id.is_placeholder() {
                            let restr_clone = restr.clone();
                            let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&card_id| {
                                    if let Some(card) = self.cards.try_get(*card_id) {
                                        card.controller != controller
                                            && restr_clone.matches(card)
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

                // RepeatEach (Pattern A — DefinedCards$ Targeted) in a trigger: populate
                // targets from what DestroyPermanent picked in this same trigger chain.
                // See check_triggers for detailed comments (mtg-914 B2 fix).
                if let Effect::RepeatEach {
                    sub_effects,
                    iterate_over:
                        crate::core::RepeatEachIterate::Cards {
                            ref targets,
                            require_in_graveyard,
                        },
                } = effect.clone()
                {
                    if targets.is_empty() && !trigger_destroy_targets.is_empty() {
                        effect = Effect::RepeatEach {
                            sub_effects,
                            iterate_over: crate::core::RepeatEachIterate::Cards {
                                targets: trigger_destroy_targets.to_vec(),
                                require_in_graveyard,
                            },
                        };
                    }
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
                    // Mode gate (Palace Siege): only fire if the source card's
                    // chosen_mode matches the gate string.
                    if let Some(gate) = &trigger.mode_gate {
                        if card.chosen_mode.as_deref() != Some(gate.as_str()) {
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
        let opponent = self.get_other_player_id(controller);
        let mut ctx = TriggerContext::new(card_id, controller);
        // Populate the opponent so that `Defined$ Player.Opponent` effects
        // (e.g. Palace Siege Dragons drain: `DB$ LoseLife | Defined$ Player.Opponent`)
        // resolve to the correct player rather than defaulting to the controller.
        ctx.opponent = opponent;
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
                // SacrificeSelf stored on a phase trigger has a placeholder
                // source (`CardId::new(0)`). Resolve it to the actual trigger
                // source card so the executor can move the right card to the
                // graveyard. The UnlessCost payer (UnlessPayer$ You = controller)
                // is also resolved here to a concrete numeric string, mirroring
                // the Paralyze/UntapPermanent pattern.
                // Pattern: Stasis / Aura Flux / Arcades Sabboth
                //   "At the beginning of your upkeep, sacrifice CARDNAME unless you pay {cost}."
                Effect::UnlessCostWrapper {
                    inner_effect,
                    unless_cost,
                } if matches!(
                    inner_effect.as_ref(),
                    Effect::SacrificeSelf { source } if source.is_placeholder()
                ) =>
                {
                    effect = Effect::UnlessCostWrapper {
                        inner_effect: Box::new(Effect::SacrificeSelf { source: card_id }),
                        unless_cost: crate::core::effects::UnlessCost {
                            cost: unless_cost.cost.clone(),
                            payer: controller.as_u32().to_string(),
                            switched: unless_cost.switched,
                        },
                    };
                }
                // SacrificeSelf without an UnlessCost wrapper (bare self-sacrifice
                // trigger). Also resolve the placeholder to the actual card.
                Effect::SacrificeSelf { source } if source.is_placeholder() => {
                    effect = Effect::SacrificeSelf { source: card_id };
                }

                // Tangle Wire: "that player taps N permanents" where N = fade
                // counter count on Tangle Wire (Count$CardCounters.FADE) and the
                // player is the one whose upkeep fired (Defined$ TriggeredPlayer).
                // Both are stored as placeholders at parse time and resolved here.
                Effect::TapPermanentsMatchingFilter {
                    player,
                    choices_filter,
                    count,
                } => {
                    // Resolve player placeholder (0) to active_player
                    let resolved_player = if player.is_placeholder() {
                        active_player
                    } else {
                        *player
                    };
                    // Resolve count placeholder (0) to the source card's FADE
                    // counter count (Count$CardCounters.FADE).
                    let fade_count = self
                        .cards
                        .try_get(card_id)
                        .map(|c| c.get_counter(crate::core::CounterType::Fade))
                        .unwrap_or(0);
                    let resolved_count = if *count == 0 { fade_count } else { *count };
                    if resolved_count == 0 {
                        // No fade counters left — Tangle Wire about to be
                        // sacrificed by Fading; skip the tap obligation.
                        continue;
                    }
                    effect = Effect::TapPermanentsMatchingFilter {
                        player: resolved_player,
                        choices_filter: choices_filter.clone(),
                        count: resolved_count,
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

    /// Like [`check_triggers_for_controller`] but injects a known `opponent` into
    /// the [`TriggerContext`] so effects that target the defending / opposing player
    /// are resolved against the correct player.
    ///
    /// Used by `AttackerUnblocked` triggers (Floral Spuzzem) where the defending
    /// player is the explicit opponent in the trigger context.
    ///
    /// # Errors
    ///
    /// Returns an error if the source card or any effect execution fails.
    pub fn check_triggers_for_controller_with_opponent(
        &mut self,
        event: TriggerEvent,
        card_id: CardId,
        active_player: PlayerId,
        opponent: Option<PlayerId>,
    ) -> Result<()> {
        // Get the card's triggers
        let effects_to_execute: Vec<Effect> = {
            let card = self.cards.get(card_id)?;
            card.triggers
                .iter()
                .filter(|trigger| trigger.event == event)
                .flat_map(|trigger| trigger.effects.clone())
                .collect()
        };

        if effects_to_execute.is_empty() {
            return Ok(());
        }

        let controller = self.cards.get(card_id)?.controller;
        let trigger_source_colors: smallvec::SmallVec<[crate::core::Color; 2]> =
            self.cards.get(card_id)?.colors.clone();

        let mut ctx = TriggerContext::new(card_id, controller);
        if let Some(opp) = opponent {
            ctx = ctx.with_opponent(opp);
        }

        for effect in effects_to_execute {
            let mut effect = resolve_effect_placeholder(&effect, &ctx);

            // Resolve DestroyPermanent placeholder against the defending player's
            // permanents (OppCtrl means "controlled by the attacker's opponent"
            // which is the defending player). We use `active_player` here as the
            // "active player" for restriction evaluation, but pass `opponent` as the
            // "target pool" player.
            if let Effect::DestroyPermanent {
                target,
                ref restriction,
                no_regenerate,
            } = effect.clone()
            {
                if target.is_placeholder() {
                    // For OppCtrl targets, the target pool is the defending player
                    // (= the attacker's opponent).
                    let pool_player = opponent.unwrap_or(active_player);
                    if let Some(target_id) = self.choose_triggered_destroy_target(
                        restriction,
                        controller,
                        pool_player,
                        &trigger_source_colors,
                    ) {
                        effect = Effect::DestroyPermanent {
                            target: target_id,
                            restriction: restriction.clone(),
                            no_regenerate,
                        };
                    } else {
                        // No legal target — trigger fizzles
                        log::debug!(
                            target: "triggers",
                            "AttackerUnblocked destroy trigger fizzled — no valid target for {:?}",
                            restriction
                        );
                        continue;
                    }
                }
            }

            if let Some(card) = self.cards.try_get(card_id) {
                self.logger
                    .normal(&format!("{} AttackerUnblocked trigger effect", card.name));
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
        let is_instant = self.cards.get(cast_spell_id).map(|c| c.is_instant()).unwrap_or(false);
        let is_enchantment = self
            .cards
            .get(cast_spell_id)
            .map(|c| c.is_enchantment())
            .unwrap_or(false);

        // Collect SpellCast triggers from permanents on the battlefield.
        //
        // Triggers with `fires_for_any_caster = false` (the default) only fire
        // for their controller's spells (Prowess, Storm, etc.).
        // Triggers with `fires_for_any_caster = true` fire for any player's
        // spells (world enchantments: In the Eye of Chaos, Presence of the
        // Master — "whenever A player casts ...").
        struct TriggerToExecute {
            source_card_id: CardId,
            controller: PlayerId,
            source_name: String,
            effects: Vec<Effect>,
            description: String,
            /// True if this trigger fires for any caster (not just its controller).
            fires_for_any_caster: bool,
        }

        let triggers_to_execute: Vec<TriggerToExecute> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Some(card) = self.cards.try_get(card_id) {
                    // Find SpellCast triggers on this permanent
                    let matching_triggers: Vec<&Trigger> = card
                        .triggers
                        .iter()
                        .filter(|trigger| {
                            if trigger.event != TriggerEvent::SpellCast {
                                return false;
                            }

                            // Controller-scoped triggers only fire for the caster's own spells.
                            // Global triggers (`fires_for_any_caster`) fire for everyone.
                            if !trigger.fires_for_any_caster && card.controller != caster_id {
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

                            // Check instant-only triggers (In the Eye of Chaos)
                            if trigger.requires_instant && !is_instant {
                                return false;
                            }

                            // Check enchantment-only triggers (Presence of the Master)
                            if trigger.requires_enchantment && !is_enchantment {
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
                                    fires_for_any_caster: trigger.fires_for_any_caster,
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

            // Build trigger context. For global ("any caster") triggers, thread
            // through the cast_spell_id, caster_id, and the cast spell's mana
            // value so that `Defined$ TriggeredSpellAbility`,
            // `UnlessPayer$ TriggeredActivator`, and `UnlessCost$ X`
            // (where X = `TriggeredCard$CardManaCost`) can be resolved.
            let ctx = if trigger.fires_for_any_caster {
                let mana_value = self.cards.get(cast_spell_id).map(|c| c.mana_cost.cmc()).unwrap_or(0);
                TriggerContext::new(trigger.source_card_id, trigger.controller)
                    .with_cast_spell(cast_spell_id, caster_id)
                    .with_cast_spell_mana_value(mana_value)
            } else {
                TriggerContext::new(trigger.source_card_id, trigger.controller)
            };

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
                    keyword_args_granted,
                } = &resolved_effect
                {
                    if target.is_placeholder() {
                        resolved_effect = Effect::PumpCreature {
                            target: trigger.source_card_id,
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                            keywords_granted: keywords_granted.clone(),
                            keyword_args_granted: keyword_args_granted.clone(),
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
        // Get the card's triggers, controller, creature-ness, and counter
        // snapshot while it's still on battlefield (CR 608.2g — LKI).
        // The `is_creature` flag gates the broad "whenever a creature dies"
        // scan below — mtg-913 B12.
        let (effects_to_execute, controller, dying_is_creature, counter_snapshot, dying_card_power) = {
            let card = self.cards.get(dying_card_id)?;

            // Collect LeavesBattlefield triggers (which we use for "dies" events)
            let effects: Vec<Effect> = card
                .triggers
                .iter()
                .filter(|trigger| trigger.event == TriggerEvent::LeavesBattlefield)
                .flat_map(|trigger| trigger.effects.clone())
                .collect();

            // Capture counter amounts for LKI (needed by CreateTokenDynamic /
            // DynamicAmount::TriggeredCardCounters, e.g. Hangarback Walker).
            let counters = card.counters.clone();

            // Capture the dying card's current power for LKI (CR 608.2g).
            // Used by CountExpression::TriggeredCardPower expressions — e.g.
            // Anax, Hardened in the Forge: "create 2 Satyr tokens if the dying
            // creature had power >= 4, else 1". Power is public (CR 613), so
            // this is information-independent for network determinism.
            let power = i32::from(card.current_power());

            (effects, card.controller, card.is_creature(), counters, power)
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

            // Build trigger context with LKI counter snapshot for
            // DynamicAmount::TriggeredCardCounters resolution, and LKI power
            // for CountExpression::TriggeredCardPower (Anax).
            let ctx = TriggerContext::new(dying_card_id, controller)
                .with_triggered_card_counters(counter_snapshot)
                .with_triggered_card_power(dying_card_power);

            // Execute each effect with placeholder resolution
            for effect in effects_to_execute {
                let mut effect = resolve_effect_placeholder(&effect, &ctx);

                // Resolve ReturnSelfAsEnchantment placeholder — the dying card IS the source.
                // This fires for Enduring Vitality's "when this dies, return it as enchantment" trigger.
                if let Effect::ReturnSelfAsEnchantment { source } = &effect {
                    if source.is_placeholder() {
                        effect = Effect::ReturnSelfAsEnchantment { source: dying_card_id };
                    }
                }

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

        // Check equipment and auras on the battlefield that were attached to the dying
        // permanent for EquippedCreatureDies triggers.
        // Equipment: Skullclamp — "Whenever equipped creature dies, draw two cards."
        //   (ValidCard$ Card.EquippedBy)
        // Auras: Pattern of Rebirth — "When enchanted creature dies, search library …"
        //   (ValidCard$ Card.AttachedBy)
        // Both are parsed as TriggerEvent::EquippedCreatureDies.
        let equipment_triggers: Vec<(CardId, PlayerId, Vec<Effect>, String)> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&equip_id| {
                let equip = self.cards.try_get(equip_id)?;
                // Accept both equipment (is_equipment()) and auras (is_aura()) as long
                // as they are attached to the dying permanent.
                if !(equip.is_equipment() || equip.is_aura()) || equip.attached_to != Some(dying_card_id) {
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

        // Check broad `CreatureDies` triggers (Fecundity et al.): a permanent on
        // the battlefield (the trigger SOURCE, not the dying card) carries a
        // "whenever a creature dies, ..." trigger and watches OTHER creatures die
        // (mtg-409 follow-up, mtg-913 B12). Only fires when the dying card is a
        // creature. The `ValidCard$` controller qualifier filters the dying
        // creature relative to the SOURCE's controller (`.YouCtrl`/`.OppCtrl`),
        // and `.Other` excludes the source's own death. The dying creature's
        // controller is threaded as the `TriggerContext` controller so a
        // placeholder draw (`Defined$ TriggeredCardController`) resolves to that
        // player — Fecundity gives the draw to the dead creature's controller,
        // NOT to Fecundity's controller. Scan in CardId order for deterministic
        // fire ordering.
        //
        // Optionality: Fecundity says "MAY draw" (`OptionalDecider$
        // TriggeredCardController`). The engine does not yet model the optional
        // decline for triggered draws — like every other triggered draw today
        // (Howling Mine, Sylvan Library), the draw is taken unconditionally.
        // This matches existing behavior and is strictly the pre-existing
        // OptionalDecider gap, NOT a regression introduced here.
        if dying_is_creature {
            let creature_dies_triggers: Vec<(CardId, Vec<Effect>, String)> = self
                .battlefield
                .cards
                .iter()
                .filter_map(|&source_id| {
                    let source = self.cards.try_get(source_id)?;
                    // Match each CreatureDies trigger on this source, applying its
                    // controller/self qualifiers against the dying creature.
                    let mut effects: Vec<Effect> = Vec::new();
                    let mut desc = String::new();
                    for trigger in &source.triggers {
                        let TriggerEvent::CreatureDies {
                            you_control,
                            exclude_self,
                        } = trigger.event
                        else {
                            continue;
                        };
                        // `.Other`: the source's own death does not fire it.
                        if exclude_self && source_id == dying_card_id {
                            continue;
                        }
                        // `.YouCtrl` / `.OppCtrl`: filter the dying creature's
                        // controller relative to the source's controller.
                        let controller_ok = match you_control {
                            None => true,
                            Some(true) => controller == source.controller,
                            Some(false) => controller != source.controller,
                        };
                        if !controller_ok {
                            continue;
                        }
                        effects.extend(trigger.effects.iter().cloned());
                        if desc.is_empty() {
                            desc = trigger.description.clone();
                        }
                    }
                    if effects.is_empty() {
                        return None;
                    }
                    Some((source_id, effects, desc))
                })
                .collect();

            for (source_id, effects, desc) in creature_dies_triggers {
                if let Some(source) = self.cards.try_get(source_id) {
                    self.logger.gamelog(&format!("Trigger: {} - {}", source.name, desc));
                }
                // ctx.controller = the DYING creature's controller, so a
                // placeholder draw lands on them (Defined$ TriggeredCardController).
                // The trigger source stays the source for any Defined$ Self refs;
                // event_source is the dying creature.
                let ctx = TriggerContext::new(source_id, controller).with_event_source(dying_card_id);
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
                    keyword_args_granted,
                } = &resolved_effect
                {
                    if target.is_placeholder() {
                        resolved_effect = Effect::PumpCreature {
                            target: trigger_info.card_id,
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                            keywords_granted: keywords_granted.clone(),
                            keyword_args_granted: keyword_args_granted.clone(),
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
            requires_defender_hand_gt_controller: bool,
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
                        requires_defender_hand_gt_controller: trigger.requires_defender_hand_gt_controller,
                    });
                }
            }

            (card.controller, power, triggers)
        };

        // Process each trigger - borrow is released, safe to call execute_effect
        for trigger_data in triggers {
            // Evaluate intervening-if condition: defending player must have more cards in hand
            // than the attacker's controller (CR 603.4).
            // Robber of the Rich: CheckSVar$ X | SVarCompare$ GTY where
            //   X = Count$ValidHand Card.DefenderCtrl (defender's hand)
            //   Y = Count$ValidHand Card.YouOwn (controller's hand)
            if trigger_data.requires_defender_hand_gt_controller {
                let defending_player_opt = self.combat.get_defending_player(attacker_id);
                let condition_met = if let Some(defending_player) = defending_player_opt {
                    let defender_hand_size = self
                        .get_player_zones(defending_player)
                        .map(|zones| zones.hand.cards.len())
                        .unwrap_or(0);
                    let controller_hand_size = self
                        .get_player_zones(controller)
                        .map(|zones| zones.hand.cards.len())
                        .unwrap_or(0);
                    defender_hand_size > controller_hand_size
                } else {
                    false
                };
                if !condition_met {
                    log::debug!(
                        "Skipping attack trigger on {} — intervening-if condition not met (defender hand <= controller hand)",
                        trigger_data.card_name
                    );
                    continue;
                }
            }

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

    /// Set a card's `times_kicked` (number of times Multikicker was paid, CR 702.33a),
    /// snapshotting the prior value for undo first. Mirrors `set_x_paid_logged`.
    /// No-op if the card is missing.
    pub(crate) fn set_times_kicked_logged(&mut self, card_id: CardId, count: u8) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.times_kicked) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetTimesKicked { card_id, prev },
            prior_log_size,
        );
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.times_kicked = count;
        }
    }

    /// Set a card's `kicker_paid` flag (CR 702.32 — Kicker optional additional cost),
    /// snapshotting the prior value for undo first. Mirrors `set_bargain_paid_logged`.
    /// No-op if the card is missing.
    pub(crate) fn set_kicker_paid_logged(&mut self, card_id: CardId, paid: bool) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.kicker_paid) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log
            .log(crate::undo::GameAction::SetKickerPaid { card_id, prev }, prior_log_size);
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.kicker_paid = paid;
        }
    }

    /// Set a card's `offspring_paid` flag (CR 702.198 — Offspring optional additional cost),
    /// snapshotting the prior value for undo first. Mirrors `set_kicker_paid_logged`.
    /// No-op if the card is missing.
    pub(crate) fn set_offspring_paid_logged(&mut self, card_id: CardId, paid: bool) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.offspring_paid) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetOffspringPaid { card_id, prev },
            prior_log_size,
        );
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.offspring_paid = paid;
        }
    }

    /// Set a card's `mode_cost_paid` value (extra generic mana cost for the chosen
    /// mode of a tiered modal spell like Fire Magic), snapshotting the prior value
    /// for undo first. Mirrors `set_offspring_paid_logged`. No-op if the card is missing.
    pub(crate) fn set_mode_cost_paid_logged(&mut self, card_id: CardId, cost: u8) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.mode_cost_paid) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetModeCostPaid { card_id, prev },
            prior_log_size,
        );
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.mode_cost_paid = cost;
        }
    }

    /// If this creature spell was cast with Offspring paid (CR 702.198), create a
    /// 1/1 token copy of it on the battlefield under the same controller.
    /// Called immediately after ETB triggers fire for `card_id`.
    /// No-op if the card is not a creature, not on the battlefield, or `offspring_paid` is false.
    pub(crate) fn create_offspring_token_if_paid(&mut self, card_id: CardId) -> crate::error::Result<()> {
        let (paid, controller, is_creature) = match self.cards.try_get(card_id) {
            Some(c) => (c.offspring_paid, c.controller, c.is_creature()),
            None => return Ok(()),
        };
        if !paid || !is_creature || !self.battlefield.contains(card_id) {
            return Ok(());
        }
        // Create a 1/1 token copy of the creature (CR 702.198a).
        self.execute_copy_permanent(card_id, controller, Some(1), Some(1), &[], 1)?;
        Ok(())
    }

    /// Set a card's `bargain_paid` flag (CR 702.162 — Bargain optional sacrifice cost),
    /// snapshotting the prior value for undo first. Mirrors `set_times_kicked_logged`.
    /// No-op if the card is missing.
    pub(crate) fn set_bargain_paid_logged(&mut self, card_id: CardId, paid: bool) {
        let Some(prev) = self.cards.try_get(card_id).map(|c| c.bargain_paid) else {
            return;
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetBargainPaid { card_id, prev },
            prior_log_size,
        );
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.bargain_paid = paid;
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
            modifier += self.get_damage_boost_for_source(source_id, target_controller);
        }
        modifier
    }

    /// Compute the total damage bonus granted by battlefield continuous effects
    /// (CR 614.1a damage-increase replacements) when `source_id` deals damage to
    /// a target whose controller is `target_controller`.
    ///
    /// Currently handles:
    /// - Artist's Talent level 3: +2 when a YOUR source deals damage to an OPPONENT.
    /// - Torbran, Thane of Red Fell (`StaticAbility::DamageIncrease`): +N when a
    ///   RED source YOU control deals damage to an opponent or opponent permanent.
    ///
    /// Called from both the spell-damage path (via `get_noncombat_damage_modifier`)
    /// and the combat-damage path (directly, since `current_damage_source` is not
    /// set during combat assignments).
    pub fn get_damage_boost_for_source(&self, source_id: CardId, target_controller: PlayerId) -> i32 {
        let Ok(source_card) = self.cards.get(source_id) else {
            return 0;
        };
        let source_controller = source_card.controller;
        // Damage-increase effects only apply when the source damages an OPPONENT
        // (a player/permanent controlled by someone other than the source's controller).
        if target_controller == source_controller {
            return 0;
        }
        let source_is_red = source_card.colors.contains(&crate::core::Color::Red);
        let mut modifier = 0;
        for &card_id in &self.battlefield.cards {
            let Ok(card) = self.cards.get(card_id) else {
                continue;
            };
            // Only effects controlled by the source's controller matter here
            // (Torbran boosts YOUR red sources; Artist's Talent boosts YOUR sources).
            if card.controller != source_controller {
                continue;
            }
            for static_ability in &card.static_abilities {
                if let crate::core::StaticAbility::DamageIncrease { bonus, .. } = static_ability {
                    // DamageIncrease (Torbran shape): requires source to be red.
                    if source_is_red {
                        modifier += *bonus as i32;
                    }
                }
            }
            // Artist's Talent level 3 (name-based, pre-existing).
            if card.name.as_str() == "Artist's Talent" && card.get_counter(crate::core::CounterType::Level) >= 3 {
                modifier += 2;
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

            // --- Crumbling Sanctuary replacement (CR 614.1a) ---
            // If any battlefield permanent carries DamageToExileLibrary, damage
            // to the player is redirected: that player exiles that many cards from
            // the top of their library instead of losing life.
            if self.has_damage_to_exile_library() {
                let player_name = self.get_player(target_id)?.name.clone();
                self.logger.normal(&format!(
                    "{} would take {} damage — Crumbling Sanctuary redirects: exile {} cards from library",
                    player_name, actual_amount, actual_amount
                ));
                // Exile cards from the top of the player's library.
                self.exile_top_of_library(target_id, actual_amount as usize)?;
                // Accumulate for source-damage triggers (Spirit Link etc.) even
                // when the damage is redirected — the damage "was dealt" (CR 119.6).
                self.accumulate_source_damage(actual_amount);
                return Ok(());
            }

            // --- Worship life-floor replacement (CR 614.1e) ---
            // If the damaged player controls a creature and has a LifeFloor
            // static on the battlefield (from Worship), damage cannot reduce
            // their life below 1.
            let capped_amount = self.apply_life_floor(target_id, actual_amount);
            if capped_amount < actual_amount {
                let player_name = self.get_player(target_id)?.name.clone();
                self.logger.normal(&format!(
                    "Worship: {} damage to {} capped to {} (life cannot go below 1 while you control a creature)",
                    actual_amount, player_name, capped_amount
                ));
            }

            // Capture log size before life change
            let prior_log_size = self.logger.log_count();

            let player = self.get_player_mut(target_id)?;
            player.lose_life(capped_amount);

            // Log the life change for undo system
            self.undo_log.log(
                crate::undo::GameAction::ModifyLife {
                    player_id: target_id,
                    delta: -capped_amount,
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

    /// Check whether any battlefield permanent carries a `DamageToExileLibrary`
    /// static ability (Crumbling Sanctuary).
    fn has_damage_to_exile_library(&self) -> bool {
        use crate::core::StaticAbility;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|c| {
                c.static_abilities
                    .iter()
                    .any(|sa| matches!(sa, StaticAbility::DamageToExileLibrary { .. }))
            })
        })
    }

    /// Exile `count` cards from the top of `player_id`'s library.
    ///
    /// Used by Crumbling Sanctuary's damage-redirect replacement
    /// (CR 614.1a: damage to a player is replaced by that player exiling
    /// that many cards from the top of their library).
    fn exile_top_of_library(&mut self, player_id: PlayerId, count: usize) -> Result<()> {
        use crate::zones::Zone;
        for _ in 0..count {
            // Top of library is at the end of the vec.
            let card_id = self
                .get_player_zones(player_id)
                .and_then(|z| z.library.cards.last().copied());
            let Some(card_id) = card_id else {
                break; // Library exhausted — game-loss is handled by state-based actions.
            };
            self.move_card(card_id, Zone::Library, Zone::Exile, player_id)?;
        }
        Ok(())
    }

    /// Apply the Worship life-floor replacement (CR 614.1e).
    ///
    /// If the player has a `LifeFloor` static on the battlefield (from Worship),
    /// they control at least one creature, and their current life >= 1, cap
    /// `amount` so that `life - amount >= 1` (i.e., return `life - 1` if
    /// unmodified damage would go below 1).
    ///
    /// Returns the (possibly capped) damage amount to deal.
    /// Caller is responsible for logging if the returned amount differs.
    fn apply_life_floor(&self, player_id: PlayerId, amount: i32) -> i32 {
        use crate::core::StaticAbility;

        // Check if a LifeFloor static is active (any battlefield permanent
        // controlled by player_id carries it — Worship is enchantment so
        // controller check matches the enchantment's owner).
        let has_life_floor = self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|c| {
                c.controller == player_id
                    && c.static_abilities
                        .iter()
                        .any(|sa| matches!(sa, StaticAbility::LifeFloor { .. }))
            })
        });
        if !has_life_floor {
            return amount;
        }

        // Does the player currently control a creature?
        let controls_creature = self.battlefield.cards.iter().any(|&id| {
            self.cards
                .try_get(id)
                .is_some_and(|c| c.controller == player_id && c.is_creature())
        });
        if !controls_creature {
            return amount;
        }

        // Current life must be >= 1 for the floor to apply.
        let current_life = self
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| p.life)
            .unwrap_or(0);
        if current_life < 1 {
            return amount;
        }

        // Cap: life - capped_amount >= 1 ⟹ capped_amount <= life - 1.
        amount.min(current_life - 1).max(0)
    }

    /// Check whether damage from `source_id` to `target_id` (a creature) is
    /// prevented by an attached Aura's `PreventDamageToEnchantedByChosenColor`
    /// static ability (CR 615.1 — Prismatic Ward shape).
    ///
    /// Returns `true` if the damage should be fully prevented (source has the
    /// chosen color of an attached prevention Aura), `false` otherwise.
    /// Information-independent: only reads the source's public color identity.
    pub fn is_color_prevented_by_aura(&self, target_id: CardId, source_id: CardId) -> bool {
        let source_colors: smallvec::SmallVec<[crate::core::Color; 2]> =
            self.cards.get(source_id).map(|c| c.colors.clone()).unwrap_or_default();
        if source_colors.is_empty() {
            return false;
        }
        for aura_id in self.get_attached_auras(target_id) {
            let Ok(aura) = self.cards.get(aura_id) else {
                continue;
            };
            let has_prevent_static = aura.static_abilities.iter().any(|a| {
                matches!(
                    a,
                    crate::core::StaticAbility::PreventDamageToEnchantedByChosenColor { .. }
                )
            });
            if !has_prevent_static {
                continue;
            }
            if let Some(chosen_color) = aura.chosen_color {
                if source_colors.contains(&chosen_color) {
                    return true;
                }
            }
        }
        false
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
            // Check Prismatic Ward / color-prevention Aura statics (CR 615.1):
            // If any attached Aura has StaticAbility::PreventDamageToEnchantedByChosenColor
            // AND the damage source has the chosen color, all damage is prevented.
            if let Some(source_id) = self.current_damage_source {
                if self.is_color_prevented_by_aura(target_id, source_id) {
                    // Find the aura name for logging (use first matching aura).
                    let aura_name = self
                        .get_attached_auras(target_id)
                        .into_iter()
                        .find(|&aura_id| {
                            self.cards
                                .get(aura_id)
                                .map(|a| {
                                    a.static_abilities.iter().any(|s| {
                                        matches!(
                                            s,
                                            crate::core::StaticAbility::PreventDamageToEnchantedByChosenColor { .. }
                                        )
                                    })
                                })
                                .unwrap_or(false)
                        })
                        .and_then(|aura_id| self.cards.get(aura_id).ok().map(|a| a.name.clone()))
                        .unwrap_or_else(|| "Prevention Aura".into());
                    self.logger.normal(&format!(
                        "{} prevents {} damage to {} ({}) (chosen color prevention)",
                        aura_name, amount, creature_name, target_id
                    ));
                    return Ok(());
                }
            }

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

                        // Move to graveyard/exile FIRST (CR 704.3), then fire death triggers.
                        let dest = self.death_destination_for_card(card_id);
                        self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                        let _ = self.check_death_triggers(card_id);

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
            // TriggeredCardCounters is resolved via TriggerContext before this
            // function is called (see resolve_effect_placeholder in triggers.rs).
            // If it somehow reaches here, read the counter count from the
            // reference card directly (last-known information fallback).
            DynamicAmount::TriggeredCardCounters(counter_type) => self
                .cards
                .try_get(reference)
                .map(|c| i32::from(c.get_counter(*counter_type)))
                .unwrap_or(0),
        }
    }

    /// Evaluate a [`CountExpression`](crate::core::CountExpression) with optional
    /// source-spell context for per-cast fields (`bargain_paid`, `kicker_paid`,
    /// `times_kicked`).
    ///
    /// Wraps `evaluate_count_expression`; the difference is that
    /// `CountExpression::Bargain` reads `source_card.bargain_paid` and
    /// `CountExpression::Kicked` reads `source_card.kicker_paid` when
    /// `source_card_id` is `Some`, rather than conservatively returning the
    /// unbargained/unkicked value. Use this in spell-resolution paths
    /// (DealDamageDynamic) where `card_id` (the resolving spell) is available.
    fn evaluate_count_with_source(
        &self,
        expr: &crate::core::CountExpression,
        controller: PlayerId,
        source_card_id: Option<CardId>,
    ) -> Result<i32> {
        use crate::core::CountExpression;
        if let CountExpression::Bargain {
            bargained_value,
            unbargained_value,
        } = expr
        {
            // Use the actual bargain_paid state on the resolving spell when available.
            let is_bargained = source_card_id
                .and_then(|id| self.cards.try_get(id))
                .is_some_and(|c| c.bargain_paid);
            return Ok(if is_bargained {
                *bargained_value
            } else {
                *unbargained_value
            });
        }
        if let CountExpression::Kicked {
            kicked_value,
            unkicked_value,
        } = expr
        {
            // Use the actual kicker_paid state on the resolving spell when available.
            // Firebending Lesson: SVar:X:Count$Kicked.5.2 — deals 5 if kicked, 2 if not.
            let is_kicked = source_card_id
                .and_then(|id| self.cards.try_get(id))
                .is_some_and(|c| c.kicker_paid);
            return Ok(if is_kicked { *kicked_value } else { *unkicked_value });
        }
        self.evaluate_count_expression(expr, controller)
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
            CountExpression::ValidPermanents { filter, modifier } => {
                let count = self.count_permanents_matching(filter, controller);
                let raw = i32::try_from(count).unwrap_or(i32::MAX);
                Ok(modifier.apply(raw))
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
            CountExpression::TimesKicked => {
                // TimesKicked is resolved via apply_etb_counters which passes the
                // card's times_kicked directly. This path (called from the mana-
                // ability / pump evaluators) has no card context, so returns 0.
                log::debug!("evaluate_count_expression: TimesKicked evaluated as 0 (no card context)");
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
                // No source card available here — conservatively return unkicked value.
                // Call evaluate_count_with_source() when the resolving spell's card_id
                // is available (e.g. from DealDamageDynamic resolution) to get the
                // correct kicked_value when kicker_paid is true.
                Ok(*unkicked_value)
            }
            CountExpression::Bargain {
                bargained_value: _,
                unbargained_value,
            } => {
                // Bargain (CR 702.162) is an optional additional cost: sacrifice an
                // artifact, enchantment, or token when casting. We don't yet track
                // whether the optional sacrifice was paid at resolution time, so we
                // conservatively evaluate as unbargained (the base damage value).
                // This ensures Torch the Tower always deals at least its printed
                // base damage (2) instead of 0.
                // TODO(mtg-863): track bargain-paid state per spell on the stack so
                // the bargained_value (3) is used when the player actually sacrificed.
                Ok(*unbargained_value)
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
            CountExpression::TriggeredCardPower => {
                // TriggeredCardPower (SVar:Z:TriggeredCard$CardPower — Anax,
                // Hardened in the Forge) must be resolved before this function
                // is called, by patching the Compare expression to Fixed in
                // `resolve_effect_placeholder` using the captured last-known
                // power from `TriggerContext::triggered_card_power`. If this
                // arm is reached the patching did not happen; evaluate to 0 to
                // avoid a panic. The token count will silently default to the
                // false-value branch of the enclosing Compare (1 token).
                log::debug!("evaluate_count_expression: TriggeredCardPower reached without context; evaluated as 0");
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
                        other => {
                            // Fall back to a subtype check (e.g. "Shrine", "Sliver",
                            // "Goblin"). Subtypes are stored as `Subtype` wrappers
                            // whose `as_str()` returns the raw string. CR 205.3
                            // (enchantment subtypes), 205.2 (creature types), etc.
                            card.subtypes.iter().any(|st| st.as_str().eq_ignore_ascii_case(other))
                                || card
                                    .temp_animate_subtypes
                                    .iter()
                                    .any(|st| st.as_str().eq_ignore_ascii_case(other))
                        }
                    };

                    if !type_matches {
                        return false;
                    }

                    // Check `withDefender` qualifier (CR 702.6). The keyword
                    // filter appears as `Creature.withDefender+YouCtrl` in SVars
                    // like Overgrown Battlement / Axebane Guardian.
                    if filter.contains("withDefender") && !card.has_keyword(crate::core::Keyword::Defender) {
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
