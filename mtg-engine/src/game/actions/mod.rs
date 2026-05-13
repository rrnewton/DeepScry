//! Game actions and mechanics

mod triggers;

pub use targeting::is_legal_target;
pub use triggers::{resolve_effect_placeholder, TriggerContext};

use crate::core::{CardId, CardType, Effect, PlayerId, TargetRef, TriggerEvent};
use crate::game::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};

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
        | Effect::CounterSpell { .. }
        | Effect::AddMana { .. }
        | Effect::PutCounter { .. }
        | Effect::MultiplyCounter { .. }
        | Effect::PutCounterAll { .. }
        | Effect::Proliferate
        | Effect::ChangeZoneAll { .. }
        | Effect::RemoveCounter { .. }
        | Effect::ExilePermanent { .. }
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
        | Effect::ModalChoice { .. }
        | Effect::Dig { .. }
        | Effect::CreateDelayedTrigger { .. }
        | Effect::CopySpellAbility { .. }
        | Effect::ImmediateTrigger { .. }
        | Effect::ClearRemembered
        | Effect::AddTurn { .. }
        | Effect::AddPhase { .. }
        | Effect::ChooseColor { .. }
        | Effect::SelfExileFromStack { .. }
        | Effect::Unimplemented { .. }
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
            | Effect::CounterSpell { .. }
            | Effect::AddMana { .. }
            | Effect::PutCounter { .. }
            | Effect::MultiplyCounter { .. }
            | Effect::PutCounterAll { .. }
            | Effect::Proliferate
            | Effect::ChangeZoneAll { .. }
            | Effect::RemoveCounter { .. }
            | Effect::ExilePermanent { .. }
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
            | Effect::ModalChoice { .. }
            | Effect::Dig { .. }
            | Effect::CreateDelayedTrigger { .. }
            | Effect::CopySpellAbility { .. }
            | Effect::ImmediateTrigger { .. }
            | Effect::ClearRemembered
            | Effect::AddTurn { .. }
            | Effect::AddPhase { .. }
            | Effect::ChooseColor { .. }
            | Effect::SelfExileFromStack { .. }
            | Effect::Unimplemented { .. }
            | Effect::UnlessCostWrapper { .. } => unreachable!(),
        })
        .collect()
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

        // Pay the mana cost (from both regular and combat mana pools)
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
        let all_targets_illegal = if !chosen_targets.is_empty() {
            // Check if any permanent target is no longer on the battlefield
            // This handles spells that target permanents
            let any_permanent_gone = chosen_targets
                .iter()
                .any(|&target_id| !self.battlefield.contains(target_id) && !self.stack.contains(target_id));

            // If spell has targets and they're all gone, it fizzles
            any_permanent_gone
        } else {
            false
        };

        // Execute effects only if targets are still valid
        if !all_targets_illegal {
            // Read x_paid once for resolving XPaid effect variants
            let x_paid = self.cards.get(card_id).map(|c| c.x_paid).unwrap_or(0);

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

                    // Expand "all players" effects (e.g., Wheel of Fortune: each player discards/draws)
                    let player_ids: smallvec::SmallVec<[PlayerId; 4]> = self.players.iter().map(|p| p.id).collect();
                    let expanded = expand_all_players_effect(&resolved, &player_ids);
                    for e in &expanded {
                        self.execute_effect(e)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Replace XPaid effect variants with their concrete-amount equivalents.
    /// Called at resolution time when the spell's x_paid value is known.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_x_paid_effect(effect: Effect, x_paid: u8) -> Effect {
        match effect {
            Effect::DealDamageXPaid { target } => Effect::DealDamage {
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
            Effect::DestroyPermanent { target, restriction } if target.is_self_target() => Effect::DestroyPermanent {
                target: source_card_id,
                restriction,
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
                    // implemented in the effect converter — see mtg-abfad9. We get the same
                    // user-visible outcome (the chosen graveyard creature comes back under
                    // our control with the Aura on it, applying its continuous -1/-0 via the
                    // existing `Affected$ Creature.EnchantedBy` static-effect path) by
                    // inlining the reanimation here.
                    //
                    // Caveats not yet handled (tracked in mtg-abfad9):
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
        let all_targets_illegal = if !chosen_targets.is_empty() {
            chosen_targets
                .iter()
                .any(|&target_id| !self.battlefield.contains(target_id) && !self.stack.contains(target_id))
        } else {
            false
        };

        if all_targets_illegal {
            return Ok(None);
        }

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
    /// What is intentionally **not** done here (tracked in mtg-abfad9):
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
                // TODO(mtg-abfad9): symbolic amounts like "X" / "Y" require an
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
        use crate::core::{CostReductionTarget, Keyword, KeywordArgs, StaticAbility};

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

        // Check for ReduceCost static abilities from controlled permanents
        // Example: Gran-Gran reduces non-creature spell costs by {1} with enough Lessons in graveyard
        for &bf_card_id in &self.battlefield.cards {
            let Some(source_card) = self.cards.try_get(bf_card_id) else {
                continue;
            };

            // Only consider permanents controlled by the player casting the spell
            if source_card.controller != player_id {
                continue;
            }

            for static_ability in &source_card.static_abilities {
                if let StaticAbility::ReduceCost {
                    valid_card,
                    amount,
                    condition,
                    description,
                } = static_ability
                {
                    // Check if the spell being cast matches the valid_card filter
                    let spell_matches = match valid_card {
                        CostReductionTarget::AllSpells => true,
                        CostReductionTarget::NonCreature => !card.is_creature(),
                        CostReductionTarget::Creature => card.is_creature(),
                        CostReductionTarget::Subtype(subtype) => card.subtypes.contains(subtype),
                    };

                    if !spell_matches {
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

                // Also check for RaiseCost (mana-based cost increases)
                if let StaticAbility::RaiseCost {
                    valid_card,
                    raised_cost,
                    description,
                } = static_ability
                {
                    use crate::core::RaisedCost;

                    // Check if the spell being cast matches the valid_card filter
                    let spell_matches = match valid_card {
                        CostReductionTarget::AllSpells => true,
                        CostReductionTarget::NonCreature => !card.is_creature(),
                        CostReductionTarget::Creature => card.is_creature(),
                        CostReductionTarget::Subtype(subtype) => card.subtypes.contains(subtype),
                    };

                    if !spell_matches {
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
    fn count_cards_matching_filter(&self, player_id: PlayerId, filter: &str, zone: crate::zones::Zone) -> usize {
        use crate::zones::Zone;

        // Parse filter: "Lesson.YouOwn" -> type="Lesson", ownership="YouOwn"
        let parts: Vec<&str> = filter.split('.').collect();
        let type_filter = parts.first().copied().unwrap_or("");
        let ownership = parts.get(1).copied().unwrap_or("YouOwn");

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

        zone_cards
            .iter()
            .filter(|&&cid| {
                let Some(c) = self.cards.try_get(cid) else {
                    return false;
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

                // Check type filter (subtype match)
                if type_filter.is_empty() {
                    return true;
                }

                // Check if card has the specified subtype
                let subtype = crate::core::Subtype::new(type_filter);
                c.subtypes.contains(&subtype)
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
        let _targets = choose_targets_fn(self, card_id);
        // TODO: Store targets on the spell for resolution
        // For now, we'll use them to update effects immediately (simplified)

        // Step 4: Divide effects
        // TODO: Implement dividing damage/counters among targets

        // Step 5: Determine total cost (after applying Affinity and other reductions)
        // If an override cost is provided (e.g., alternative cost or commander cost),
        // use that instead of calculating from the card's printed cost.
        let mana_cost = if let Some(cost) = override_cost {
            cost
        } else {
            self.calculate_effective_cost(card_id, player_id)
        };

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
            if let Err(e) = self.tap_for_mana_for_cost(player_id, source_id, &remaining_hint) {
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

            // Update remaining hint based on what color this source produced.
            // Check mana production kind to know what color was produced. Multi-mana
            // sources (Sol Ring → 2, Black Lotus → 3) deduct their full per-tap
            // amount so subsequent taps in the same payment see the correct hint.
            if let Some(card) = self.cards.try_get(source_id) {
                let production = &card.definition.cache.mana_production;
                let mut remaining_amount = production.amount.max(1);
                match &production.kind {
                    crate::core::ManaProductionKind::Fixed(color) => {
                        // Deduct the fixed color from remaining hint
                        match color {
                            crate::core::ManaColor::White => {
                                let take = remaining_hint.white.min(remaining_amount);
                                remaining_hint.white -= take;
                                remaining_amount -= take;
                            }
                            crate::core::ManaColor::Blue => {
                                let take = remaining_hint.blue.min(remaining_amount);
                                remaining_hint.blue -= take;
                                remaining_amount -= take;
                            }
                            crate::core::ManaColor::Black => {
                                let take = remaining_hint.black.min(remaining_amount);
                                remaining_hint.black -= take;
                                remaining_amount -= take;
                            }
                            crate::core::ManaColor::Red => {
                                let take = remaining_hint.red.min(remaining_amount);
                                remaining_hint.red -= take;
                                remaining_amount -= take;
                            }
                            crate::core::ManaColor::Green => {
                                let take = remaining_hint.green.min(remaining_amount);
                                remaining_hint.green -= take;
                                remaining_amount -= take;
                            }
                        }
                        // Any leftover mana from this single tap covers generic.
                        let take = remaining_hint.generic.min(remaining_amount);
                        remaining_hint.generic -= take;
                    }
                    crate::core::ManaProductionKind::Colorless => {
                        // Colorless covers colorless first, then generic.
                        let take_c = remaining_hint.colorless.min(remaining_amount);
                        remaining_hint.colorless -= take_c;
                        remaining_amount -= take_c;
                        let take_g = remaining_hint.generic.min(remaining_amount);
                        remaining_hint.generic -= take_g;
                    }
                    crate::core::ManaProductionKind::Choice(_) | crate::core::ManaProductionKind::AnyColor => {
                        // For choice/any-color lands, the chosen colour matches the
                        // first remaining colored requirement; remaining mana from
                        // this tap can satisfy generic. This loop drains all `n`
                        // pips produced by this single activation.
                        while remaining_amount > 0 {
                            if remaining_hint.white > 0 {
                                remaining_hint.white -= 1;
                            } else if remaining_hint.blue > 0 {
                                remaining_hint.blue -= 1;
                            } else if remaining_hint.black > 0 {
                                remaining_hint.black -= 1;
                            } else if remaining_hint.red > 0 {
                                remaining_hint.red -= 1;
                            } else if remaining_hint.green > 0 {
                                remaining_hint.green -= 1;
                            } else if remaining_hint.generic > 0 {
                                remaining_hint.generic -= 1;
                            } else {
                                break; // No more cost to cover
                            }
                            remaining_amount -= 1;
                        }
                    }
                }
            }
        }

        // Step 7: Pay costs (from both regular and combat mana pools)
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
            // Target resolution for permanent-targeting effects
            Effect::DealDamage {
                target: TargetRef::None,
                amount,
            } => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    Effect::DealDamage {
                        target: TargetRef::Permanent(target),
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

            // DealDamageXPaid with no target: resolve target like DealDamage
            Effect::DealDamageXPaid {
                target: TargetRef::None,
            } => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    Effect::DealDamageXPaid {
                        target: TargetRef::Permanent(target),
                    }
                } else if let Some(opp) = opponent_id {
                    Effect::DealDamageXPaid {
                        target: TargetRef::Player(opp),
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

            Effect::DestroyPermanent { target, restriction } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::DestroyPermanent {
                        target: resolved_target,
                        restriction: restriction.clone(),
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
                until_eot,
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
                        until_eot: *until_eot,
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
                    effect.clone()
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
            Effect::CounterSpell { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::CounterSpell {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::ExilePermanent { target } if target.is_placeholder() => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::ExilePermanent {
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
            Effect::DiscardCardsXPaid {
                player,
                remember_discarded,
            } if player.is_placeholder() => Effect::DiscardCardsXPaid {
                player: card_owner,
                remember_discarded: *remember_discarded,
            },
            Effect::GainLife { player, amount } if player.is_placeholder() => Effect::GainLife {
                player: card_owner,
                amount: *amount,
            },
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
                    "TargetedController" => {
                        // Get controller of the targeted permanent/spell
                        // Use last_resolved_target if available, otherwise fall back to opponent
                        if let Some(target_id) = last_resolved_target {
                            self.cards
                                .get(*target_id)
                                .map(|c| c.controller)
                                .unwrap_or_else(|_| opponent_id.unwrap_or(card_owner))
                        } else if let Some(opp) = opponent_id {
                            opp
                        } else {
                            card_owner
                        }
                    }
                    "Player" | "Opponent" => opponent_id.unwrap_or(card_owner),
                    _ => card_owner, // Default to spell controller
                };

                // Create resolved UnlessCost with concrete payer
                let resolved_unless_cost = crate::core::effects::UnlessCost {
                    cost: unless_cost.cost.clone(),
                    payer: resolved_payer.as_u32().to_string(), // Store as numeric ID string
                    switched: unless_cost.switched,
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

    /// Execute a single effect
    ///
    /// # Errors
    ///
    /// Returns an error if the effect cannot be executed (e.g., invalid target).
    pub fn execute_effect(&mut self, effect: &Effect) -> Result<()> {
        match effect {
            Effect::DealDamage { target, amount } => match target {
                TargetRef::Player(player_id) => {
                    self.deal_damage(*player_id, *amount)?;
                }
                TargetRef::Permanent(card_id) => {
                    self.deal_damage_to_creature(*card_id, *amount)?;
                }
                TargetRef::None => {
                    // Spell fizzles - no valid target (CR 608.2b)
                    // This happens when triggered damage effects fire with no valid target
                    log::debug!("DealDamage fizzles - no target specified");
                }
            },

            Effect::EachDamage {
                damagers,
                receiver,
                use_card_power,
                fixed_damage,
            } => {
                // Each damager deals damage to the receiver
                // If receiver is placeholder, the effect wasn't resolved - fizzle
                if receiver.is_placeholder() {
                    log::debug!("EachDamage: receiver not resolved, fizzling");
                    return Ok(());
                }

                // Check if receiver is still valid
                if !self.battlefield.contains(*receiver) {
                    log::debug!("EachDamage: receiver {} no longer on battlefield", receiver.as_u32());
                    return Ok(());
                }

                for damager_id in damagers {
                    // Check if damager is still on battlefield
                    if !self.battlefield.contains(*damager_id) {
                        log::debug!(
                            "EachDamage: damager {} no longer on battlefield, skipping",
                            damager_id.as_u32()
                        );
                        continue;
                    }

                    // Calculate damage amount
                    let damage = if *use_card_power {
                        // Get damager's current power (includes counters and bonuses)
                        self.cards
                            .get(*damager_id)
                            .map(|c| i32::from(c.current_power()))
                            .unwrap_or(0)
                    } else {
                        *fixed_damage
                    };

                    if damage > 0 {
                        // Get names for logging
                        let damager_name = self
                            .cards
                            .get(*damager_id)
                            .map(|c| c.name.to_string())
                            .unwrap_or_else(|_| "creature".to_string());
                        let receiver_name = self
                            .cards
                            .get(*receiver)
                            .map(|c| c.name.to_string())
                            .unwrap_or_else(|_| "creature".to_string());

                        self.logger.normal(&format!(
                            "{} deals {} damage to {}",
                            damager_name, damage, receiver_name
                        ));

                        self.deal_damage_to_creature(*receiver, damage)?;
                    }
                }
            }

            Effect::DrawCards { player, count } => {
                if player.is_remembered_players() {
                    // Draw for each player stored in remembered_players
                    // Clone to avoid borrow conflict during mutation
                    let players: smallvec::SmallVec<[PlayerId; 4]> = self.remembered_players.iter().copied().collect();
                    for pid in players {
                        for _ in 0..*count {
                            let (_, draw_num) = self.draw_card(pid)?;
                            self.check_card_drawn_triggers(pid, draw_num)?;
                        }
                    }
                } else {
                    for _ in 0..*count {
                        let (_, draw_num) = self.draw_card(*player)?;
                        // Check for "second card drawn" triggers
                        self.check_card_drawn_triggers(*player, draw_num)?;
                    }
                }
            }
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
                        return Ok(());
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
                        self.discard_card(*player, card_id)?;
                    }
                    if *remember_discarding_players && did_discard {
                        self.remembered_players.push(*player);
                    }
                } else {
                    // Fixed count: AI chooses which cards to discard
                    let mut did_discard = false;
                    for _ in 0..*count {
                        let card_to_discard = self.choose_card_to_discard(*player)?;
                        if let Some(card_id) = card_to_discard {
                            did_discard = true;
                            if *remember_discarded {
                                self.remembered_cards.push(card_id);
                            }
                            self.discard_card(*player, card_id)?;
                        } else {
                            // No cards in hand to discard
                            break;
                        }
                    }
                    if *remember_discarding_players && did_discard {
                        self.remembered_players.push(*player);
                    }
                }
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
                        self.discard_card(*player, card_id)?;
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
            }
            Effect::GainLife { player, amount } => {
                // Capture log size before life gain
                let prior_log_size = self.logger.log_count();

                let p = self.get_player_mut(*player)?;
                p.gain_life(*amount);

                // Log the life gain
                self.undo_log.log(
                    crate::undo::GameAction::ModifyLife {
                        player_id: *player,
                        delta: *amount,
                    },
                    prior_log_size,
                );
            }
            Effect::LoseLife { player, amount } => {
                let prior_log_size = self.logger.log_count();

                let p = self.get_player_mut(*player)?;
                let player_name = p.name.clone();
                p.lose_life(*amount);
                let new_life = p.life;

                self.logger
                    .gamelog(&format!("{} loses {} life (life: {})", player_name, amount, new_life));

                self.undo_log.log(
                    crate::undo::GameAction::ModifyLife {
                        player_id: *player,
                        delta: -*amount,
                    },
                    prior_log_size,
                );
            }
            Effect::DestroyPermanent { target, .. } => {
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
                } else if has_regen_shield {
                    // CR 701.15a: Regeneration replaces destruction
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
                until_eot,
            } => {
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

                // Change controller
                {
                    let card = self.cards.get_mut(*target)?;
                    card.controller = *new_controller;
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

                let duration = if *until_eot { " until end of turn" } else { "" };
                self.logger.gamelog(&format!(
                    "{} gains control of {}{}",
                    new_ctrl_name, target_name, duration
                ));

                // TODO(mtg-77): Implement EOT control return for until_eot=true
                // This requires end-of-turn delayed trigger infrastructure
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
                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Use helper that handles tap + undo log + mana version
                self.tap_permanent(*target)?;
                // Check for Taps triggers
                self.check_triggers(TriggerEvent::Taps, *target)?;
            }
            Effect::UntapPermanent { target } => {
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
                // Grant keywords
                for keyword in keywords_granted.iter() {
                    card.keywords.insert(*keyword);
                }

                // Log the pump effect
                self.undo_log.log(
                    crate::undo::GameAction::PumpCreature {
                        card_id: *target,
                        power_delta: *power_bonus,
                        toughness_delta: *toughness_bonus,
                        keywords_granted: keywords_granted.clone(),
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

                // Evaluate the count expressions
                let power_bonus = self.evaluate_count_expression(power_count, target_controller)?;
                let toughness_bonus = self.evaluate_count_expression(toughness_count, target_controller)?;

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
                for keyword in keywords_granted.iter() {
                    card.keywords.insert(*keyword);
                }

                // Log for undo
                self.undo_log.log(
                    crate::undo::GameAction::PumpCreature {
                        card_id: *target,
                        power_delta: power_bonus,
                        toughness_delta: toughness_bonus,
                        keywords_granted: keywords_granted.clone(),
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
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .filter_map(|&card_id| {
                        if let Some(card) = self.cards.try_get(card_id) {
                            // Check if it's a creature
                            if !card.is_creature() {
                                return None;
                            }
                            // Check filter: "Creature.YouCtrl" means controller's creatures
                            if filter.contains("YouCtrl") && card.controller != *controller {
                                return None;
                            }
                            // Check filter: "Creature.OppCtrl" means opponent's creatures
                            if filter.contains("OppCtrl") && card.controller == *controller {
                                return None;
                            }
                            Some(card_id)
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
                    if let Ok(card) = self.cards.get_mut(target) {
                        let card_name = card.name.clone();

                        // Set base P/T if specified
                        if let Some(p) = power {
                            card.set_temp_base_power(*p as i8);
                        }
                        if let Some(t) = toughness {
                            card.set_temp_base_toughness(*t as i8);
                        }

                        // Grant keywords
                        for kw in keywords_granted {
                            card.keywords.insert(*kw);
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
            Effect::Mill { player, count } => {
                // Mill cards from library to graveyard
                self.mill_cards(*player, *count)?;
            }
            Effect::Scry { player, count } => {
                // Scry - look at top N cards, put any number on bottom
                self.scry_cards(*player, *count)?;
            }
            Effect::Surveil { player, count } => {
                // Surveil - look at top N cards, put any into graveyard, rest on top (CR 701.42)
                self.surveil_cards(*player, *count)?;
            }
            Effect::AddTurn { player, num_turns } => {
                // Take extra turns (CR 500.7) - Time Walk, Temporal Manipulation, etc.
                // Add extra turns to the turn queue for the specified player
                for _ in 0..*num_turns {
                    self.turn.extra_turns.push(*player);
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
            Effect::CounterSpell { target } => {
                // Counter a spell on the stack
                // Fizzle if target is placeholder (no valid target found) or not on stack
                // This happens when triggered counter effects (e.g., Ulamog's Nullifier ETB)
                // fire when no spell is on the stack to target
                if target.is_placeholder() || !self.stack.contains(*target) {
                    log::debug!("CounterSpell fizzles - target {} not on stack", target.as_u32());
                } else {
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
                origin,
                destination,
            } => {
                // Move all cards matching the restriction from origin zone to destination zone
                let cards_to_move: Vec<(CardId, PlayerId)> = match origin {
                    crate::zones::Zone::Battlefield => self
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
                        .collect(),
                    crate::zones::Zone::Graveyard => {
                        // Collect from all players' graveyards
                        let mut result = Vec::new();
                        for (player_id, zones) in &self.player_zones {
                            for &card_id in &zones.graveyard.cards {
                                if let Some(card) = self.cards.try_get(card_id) {
                                    if restriction.matches(card) {
                                        result.push((card_id, *player_id));
                                    }
                                }
                            }
                        }
                        result
                    }
                    crate::zones::Zone::Hand
                    | crate::zones::Zone::Exile
                    | crate::zones::Zone::Library
                    | crate::zones::Zone::Stack
                    | crate::zones::Zone::Command => Vec::new(), // Other origin zones not yet supported
                };

                for (card_id, owner) in cards_to_move {
                    self.move_card(card_id, *origin, *destination, owner)?;
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
            Effect::SetBasePowerToughness {
                target,
                power,
                toughness,
                keywords_granted,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.is_placeholder() {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Set temporary base P/T override (until end of turn)
                // This is used by Animate effects like Flexible Waterbender and Turtle-Duck
                let card = self.cards.get_mut(*target)?;
                let card_name = card.name.clone();
                let _old_power = card.current_power();
                let _old_toughness = card.current_toughness();

                // Only set power if specified
                if let Some(p) = power {
                    card.set_temp_base_power(*p as i8);
                }
                // Only set toughness if specified
                if let Some(t) = toughness {
                    card.set_temp_base_toughness(*t as i8);
                }

                // Grant temporary keywords (until end of turn)
                // Note: Uses same approach as PumpCreature - keywords added to permanent set
                // TODO: Consider tracking temp keywords separately for proper EOT cleanup
                for kw in keywords_granted {
                    card.keywords.insert(*kw);
                }

                let new_power = card.current_power();
                let new_toughness = card.current_toughness();

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

                // Call the attach_equipment method from Phase 1
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

                // Get card info and apply modifications
                let card_name = {
                    let card = self.cards.get_mut(*target)?;

                    // Must be a land to earthbend
                    if !card.is_land() {
                        return Err(crate::MtgError::InvalidAction(
                            "Earthbend target must be a land".to_string(),
                        ));
                    }

                    // Add Creature type (still remains a land)
                    if !card.is_creature() {
                        card.add_type(CardType::Creature);
                    }

                    // Set temp base power/toughness to 0/0 (animate effect)
                    card.set_temp_base_power(0);
                    card.set_temp_base_toughness(0);

                    // Add Haste keyword so it can attack immediately
                    use crate::core::Keyword;
                    card.keywords.insert(Keyword::Haste);

                    card.name.clone()
                };

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

            Effect::PreventDamage { target, amount } => {
                // Prevent damage: Add a damage prevention shield (CR 615.1)
                // "Prevent the next N damage that would be dealt to [target] this turn."
                match target {
                    TargetRef::Permanent(card_id) => {
                        if card_id.is_placeholder() {
                            return Ok(());
                        }
                        if !self.battlefield.contains(*card_id) {
                            return Ok(()); // Target left battlefield - fizzle
                        }
                        let card = self.cards.get_mut(*card_id)?;
                        card.damage_prevention += amount;
                        let card_name = self.cards.get(*card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                        self.logger.gamelog(&format!(
                            "Prevent the next {} damage that would be dealt to {} ({}) this turn",
                            amount, card_name, card_id
                        ));
                    }
                    TargetRef::Player(player_id) => {
                        let player = self.get_player_mut(*player_id)?;
                        player.damage_prevention += amount;
                        let player_name = self
                            .get_player(*player_id)
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|_| "Unknown".to_string());
                        self.logger.gamelog(&format!(
                            "Prevent the next {} damage that would be dealt to {} this turn",
                            amount, player_name
                        ));
                    }
                    TargetRef::None => {
                        // No target specified - shouldn't happen for PreventDamage
                        log::warn!("PreventDamage with no target");
                    }
                }
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
            } => {
                // Deal damage to all creatures matching the filter
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .copied()
                    .filter(|&card_id| {
                        self.cards
                            .get(card_id)
                            .map(|card| card.is_creature() && valid_cards.matches(card))
                            .unwrap_or(false)
                    })
                    .collect();

                for card_id in targets {
                    let card = self.cards.get_mut(card_id)?;
                    card.damage += *amount;
                    let card_name = card.name.clone();
                    let total_damage = card.damage;
                    self.logger.gamelog(&format!(
                        "{} ({}) takes {} damage (total: {})",
                        card_name, card_id, amount, total_damage
                    ));
                }

                // Optionally damage all players
                if *damage_players {
                    let player_ids: Vec<_> = self.players.iter().map(|p| p.id).collect();
                    for pid in player_ids {
                        let p = self.get_player_mut(pid)?;
                        let player_name = p.name.clone();
                        p.lose_life(*amount);
                        let new_life = p.life;
                        self.logger
                            .gamelog(&format!("{} takes {} damage (life: {})", player_name, amount, new_life));
                    }
                }

                // Check for creatures that took lethal damage
                self.check_lethal_damage()?;
            }

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
                    let card = self.cards.get_mut(card_id)?;
                    card.tapped = true;
                    let card_name = card.name.clone();
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
                    let card = self.cards.get_mut(card_id)?;
                    card.tapped = false;
                    let card_name = card.name.clone();
                    self.logger.gamelog(&format!("{} ({}) is untapped", card_name, card_id));
                }
            }

            Effect::SetLife { player, amount } => {
                // Set a player's life total to a specific amount
                // CR 119.5: "If an effect sets a player's life total, the player gains or loses
                // the necessary amount of life"
                let p = self.get_player_mut(*player)?;
                let player_name = p.name.clone();
                let old_life = p.life;
                p.life = *amount;
                self.logger.gamelog(&format!(
                    "{}'s life total is set to {} (was {})",
                    player_name, amount, old_life
                ));
            }

            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } => {
                // CreateDelayedTrigger effect: Register a delayed trigger that fires on a condition
                // Created by SP$ DelayedTrigger spells (e.g., Fatal Fissure)
                //
                // Implementation:
                // 1. Verify the tracked card is still on battlefield (target still valid)
                // 2. Create a DelayedTrigger with the specified condition
                // 3. Store the effect to execute when triggered
                // 4. Register the trigger in the delayed_triggers store

                // Skip if tracked_card is still placeholder (0) - no valid targets found
                if tracked_card.is_placeholder() {
                    // Spell fizzles - no valid targets
                    log::debug!(target: "actions", "CreateDelayedTrigger: tracked_card is placeholder, spell fizzles");
                    return Ok(());
                }

                // Verify the target is still on battlefield
                if !self.battlefield.contains(*tracked_card) {
                    log::debug!(target: "actions", "CreateDelayedTrigger: target no longer on battlefield, spell fizzles");
                    return Ok(());
                }

                // Get card name for logging
                let card_name = self
                    .cards
                    .get(*tracked_card)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");

                // Get the spell controller
                let controller = self.turn.active_player;

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

                let trigger_id = self.delayed_triggers.add(trigger);

                // Log the delayed trigger creation
                self.logger.gamelog(&format!(
                    "Delayed trigger {} created: watching {} for death",
                    trigger_id.as_u32(),
                    card_name
                ));

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
                non_legendary: _, // TODO(mtg-8pen1): Implement legendary rule removal when legendary is tracked
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

                            let action = if *reveal { "reveals and puts" } else { "puts" };
                            self.logger.gamelog(&format!(
                                "{} {} {} into {:?}",
                                digger_name, action, card_name, destination
                            ));
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
                                    self.logger
                                        .gamelog(&format!("{} puts {} into {}", digger_name, card_name, dest_name));
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
                // 1. Inside a delayed trigger (handled by DelayedEffect::CopySpellAbility)
                // 2. As a SubAbility of another effect (e.g., Chain Lightning)
                //
                // For SubAbility use (Defined$ Parent), we need to copy the current spell.
                // This is complex because we need to:
                // - Track the currently resolving spell
                // - Clone its effects with potentially new targets
                // - Put the copy on the stack under a different controller
                //
                // For now, log that copy would happen but don't actually create it.
                // The opponent pays the cost but the copy is not created - this is
                // a gameplay limitation noted in the tracking issue.
                //
                // TODO(mtg-152): Implement full CopySpellAbility for SubAbility context
                use crate::core::effects::CopySpellSource;
                match defined_source {
                    CopySpellSource::Parent => {
                        log::info!(
                            target: "actions",
                            "CopySpellAbility: would copy parent spell (e.g., Chain Lightning). \
                             may_choose_targets={}, controller={:?}. \
                             Copy not yet implemented - see mtg-152",
                            may_choose_targets,
                            controller
                        );
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
                // Clear both remembered cards and remembered players storage
                self.remembered_cards.clear();
                self.remembered_players.clear();
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
                        // For mana costs, check total mana available
                        // Simplified: just check if generic cost <= lands
                        let lands_count = self
                            .battlefield
                            .cards
                            .iter()
                            .filter(|&&cid| {
                                self.cards
                                    .get(cid)
                                    .is_ok_and(|c| c.is_land() && c.controller == payer_id && !c.tapped)
                            })
                            .count();
                        lands_count >= mana_cost.generic as usize
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
                        UnlessCostType::Mana(_mana_cost) => {
                            // TODO: Implement mana payment
                            // For now, assume payment succeeds if can_pay was true
                            log::debug!("UnlessCost: Mana payment simplified (auto-success)");
                            true
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

            // XPaid variants should be resolved to concrete variants before execution
            // by resolve_x_paid_effect() in resolve_spell_execute_effects().
            // If we reach here, treat as amount=0 (shouldn't happen in normal flow).
            Effect::DealDamageXPaid { target } => {
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

    /// Discard a specific card from the player's hand.
    ///
    /// Moves the card from hand to graveyard and logs the action.
    ///
    /// # Errors
    ///
    /// Returns an error if the card or player cannot be found, or if the
    /// card cannot be moved from hand to graveyard.
    pub fn discard_card(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Get card name for logging before move (unrevealed cards use fallback name)
        let card_name = self
            .cards
            .get(card_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));
        let player_name = self.get_player(player_id)?.name.clone();

        // Move card from hand to graveyard
        self.move_card(card_id, Zone::Hand, Zone::Graveyard, player_id)?;

        // Log the discard
        self.logger.gamelog(&format!("{} discards {}", player_name, card_name));

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
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_triggers(&mut self, event: TriggerEvent, source_card_id: CardId) -> Result<()> {
        use crate::core::Trigger;

        // Info needed to check trigger payability and execute costs
        struct TriggerInfo {
            card_id: CardId,
            card_name: crate::core::types::CardName, // Use Arc<str> instead of String to avoid heap allocation
            controller: PlayerId,
            trigger: Trigger,
        }

        // Collected trigger with cost info for execution
        struct TriggerToExecute {
            source_card_id: CardId,
            effects: Vec<Effect>,
            sacrifice_target: Option<CardId>, // Card to sacrifice for the cost
            sacrificed_power: u8,             // Power of sacrifice target (for Firebend effects)
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

                card.triggers
                    .iter()
                    .filter(move |trigger| {
                        // Check event type matches
                        if trigger.event != event {
                            return false;
                        }

                        // Self-only triggers only fire when the trigger source is the event source
                        if trigger.trigger_self_only && card_id != source_card_id {
                            return false;
                        }

                        // "[other]" triggers only fire when the event source is DIFFERENT from trigger source
                        // (e.g., "whenever you sacrifice another permanent" on Pirate Peddlers)
                        // OPTIMIZATION: Use pre-parsed boolean flag instead of runtime .contains()
                        if trigger.requires_other && card_id == source_card_id {
                            return false;
                        }

                        // "[landfall]" triggers only fire when:
                        // 1. The entering card is a Land
                        // 2. The entering card is controlled by the trigger's controller
                        // OPTIMIZATION: Use pre-parsed boolean flag instead of runtime .contains()
                        if trigger.requires_landfall {
                            if !source_card_is_land {
                                return false;
                            }
                            if source_card_controller != Some(controller) {
                                return false;
                            }
                        }

                        true
                    })
                    .map(move |trigger| TriggerInfo {
                        card_id,
                        card_name: card_name.clone(), // Clone Arc<str> only for matching triggers
                        controller,
                        trigger: trigger.clone(),
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
                    Effect::DestroyPermanent { target, restriction } if target.is_placeholder() => {
                        // Find a valid target (opponent's creature matching restriction),
                        // sorted by CardId for determinism after rewind+replay.
                        let mut candidates: smallvec::SmallVec<[CardId; 8]> = self
                            .battlefield
                            .cards
                            .iter()
                            .filter(|&card_id| {
                                if let Some(card) = self.cards.try_get(*card_id) {
                                    restriction.matches(card)
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
                            effect = Effect::DestroyPermanent {
                                target: target_id,
                                restriction: restriction.clone(),
                            };
                        }
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                        keywords_granted,
                    } if target.is_placeholder() => {
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
                    true
                })
                .flat_map(|trigger| trigger.effects.clone())
                .collect()
        };

        // Build trigger context for placeholder resolution
        let controller = self.cards.get(card_id)?.controller;
        let ctx = TriggerContext::new(card_id, controller);

        // Execute each effect with placeholder resolution
        for effect in effects_to_execute {
            // Step 1: Apply shared placeholder resolution for simple cases
            let mut effect = resolve_effect_placeholder(&effect, &ctx);

            // Step 2: Handle complex targeting that requires battlefield search
            match &effect {
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

        // Check if the cast spell is a creature (for noncreature-only triggers)
        let is_creature_spell = self.cards.get(cast_spell_id).map(|c| c.is_creature()).unwrap_or(false);

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
                let resolved_effect = resolve_effect_placeholder(&effect, &ctx);

                // Log specific effects with custom messages
                // Wildcard is intentional: only PutCounter and PumpCreature need special logging
                #[allow(clippy::wildcard_enum_match_arm)]
                match &resolved_effect {
                    Effect::PutCounter { target, amount, .. } if *target == trigger.source_card_id => {
                        let current_counters = self
                            .cards
                            .get(trigger.source_card_id)
                            .map(|c| c.get_counter(crate::core::CounterType::P1P1))
                            .unwrap_or(0);
                        self.logger.normal(&format!(
                            "{} gets a +1/+1 counter (now {} counters)",
                            trigger.source_name,
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
    pub fn deal_damage(&mut self, target_id: PlayerId, amount: i32) -> Result<()> {
        // Check if target is a player
        if self.players.iter().any(|p| p.id == target_id) {
            // Apply damage prevention shield (CR 615.1)
            let (actual_amount, prevented) = {
                let player = self.get_player_mut(target_id)?;
                if player.damage_prevention > 0 {
                    let prevented = amount.min(player.damage_prevention);
                    player.damage_prevention -= prevented;
                    (amount - prevented, prevented)
                } else {
                    (amount, 0)
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
        // Get info about the creature first (without holding the borrow)
        let (is_creature, creature_name) = {
            let card = self.cards.get(target_id)?;
            (card.is_creature(), card.name.clone())
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

            // Mark damage on the creature (MTG CR 120.3)
            // Damage persists until cleanup step (CR 704.5f)
            let card = self.cards.get_mut(target_id)?;
            card.damage += actual_amount;

            let message = format!(
                "{} ({}) takes {} damage (total: {})",
                creature_name, target_id, actual_amount, card.damage
            );
            self.logger.normal(&message);

            // Note: We don't destroy the creature here - that happens in state-based actions
            // This allows multiple damage sources to accumulate before checking lethal damage
            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Tap a land for mana (without cost hint)
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana.
    pub fn tap_for_mana(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Create an empty cost hint
        let empty_cost = crate::core::ManaCost::new();
        self.tap_for_mana_for_cost(player_id, card_id, &empty_cost)
    }

    /// Tap a permanent for mana with a cost hint to guide color production
    ///
    /// This method handles both:
    /// - Lands with implicit mana abilities (based on subtypes)
    /// - Creatures/artifacts with explicit mana abilities (e.g., "Guy in the Chair", Black Lotus)
    ///
    /// For mana abilities with sacrifice costs (e.g., Black Lotus), this will also
    /// sacrifice the permanent after activating the mana ability.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana or is already tapped.
    pub fn tap_for_mana_for_cost(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        cost_hint: &crate::core::ManaCost,
    ) -> Result<()> {
        let card = self.cards.get_mut(card_id)?;

        // Check if card is untapped
        if card.tapped {
            return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
        }

        // Check if card can produce mana (either land or has mana ability)
        let is_land = card.is_land();
        let has_mana_ability = card.activated_abilities.iter().any(|ab| ab.is_mana_ability);

        if !is_land && !has_mana_ability {
            return Err(MtgError::InvalidAction("Permanent cannot produce mana".to_string()));
        }

        // Check for explicit mana ability and its cost before tapping
        // We need both the mana production and the full cost (for sacrifice, etc.)
        let (explicit_mana, mana_ability_cost) = if !is_land && has_mana_ability {
            // For non-lands (creatures, artifacts) with mana abilities,
            // extract the mana from the activated ability's AddMana effect
            // and also capture the full cost for non-tap costs (like sacrifice)
            card.activated_abilities
                .iter()
                .find(|ab| ab.is_mana_ability)
                .map(|ab| {
                    let mana = ab.effects.iter().find_map(|effect| {
                        if let crate::core::Effect::AddMana { mana, .. } = effect {
                            Some(*mana)
                        } else {
                            None
                        }
                    });
                    (mana, Some(ab.cost.clone()))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        // Capture log size before tap
        let prior_log_size = self.logger.log_count();

        // Tap the permanent
        card.tap();

        // Log the tap
        self.undo_log.log(
            crate::undo::GameAction::TapCard { card_id, tapped: true },
            prior_log_size,
        );

        // Update mana caches (event-driven incremental update)
        // Read card data to avoid borrow conflicts
        if let Some(card) = self.cards.try_get(card_id) {
            for (_, cache) in &mut self.mana_caches {
                cache.on_tap(card_id, card);
            }
        }

        // Increment mana state version to invalidate ManaEngine cache
        self.increment_mana_version();

        // Check for Taps triggers (e.g., Gran-Gran: "Whenever ~ becomes tapped")
        self.check_triggers(TriggerEvent::Taps, card_id)?;

        // Handle non-land mana sources with explicit mana abilities
        if let Some(mana_to_add) = explicit_mana {
            // For creatures with "Add mana of any color", we need to choose based on cost hint
            // Check if this is an any-color source using the pre-computed cache
            // (derived from parsed abilities, not text)
            let is_any_color = self
                .cards
                .get(card_id)
                .map(|c| {
                    matches!(
                        c.definition.cache.mana_production.kind,
                        crate::core::ManaProductionKind::AnyColor
                    )
                })
                .unwrap_or(false);

            // For multi-mana any-color sources like Black Lotus (`Amount$ 3`),
            // each activation produces N mana of the chosen colour, not 1. Read
            // the per-activation amount from the cached production BEFORE
            // borrowing the player mutably (otherwise we'd hold incompatible
            // borrows on `self.cards` and `self.players`).
            let any_color_amount = if is_any_color {
                self.cards
                    .get(card_id)
                    .map(|c| c.definition.cache.mana_production.amount.max(1))
                    .unwrap_or(1)
            } else {
                1
            };

            // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
            let prior_log_size = self.logger.log_count();

            let player = self.get_player_mut(player_id)?;

            if is_any_color {
                // Choose color based on cost hint
                let color = if cost_hint.white > 0 {
                    crate::core::Color::White
                } else if cost_hint.blue > 0 {
                    crate::core::Color::Blue
                } else if cost_hint.black > 0 {
                    crate::core::Color::Black
                } else if cost_hint.red > 0 {
                    crate::core::Color::Red
                } else if cost_hint.green > 0 {
                    crate::core::Color::Green
                } else {
                    // Default to green if no specific color needed
                    crate::core::Color::Green
                };

                // Use the per-activation amount captured above before the
                // mutable borrow.
                let amount = any_color_amount;

                let mut mana = crate::core::ManaCost::new();
                let color_symbol = match color {
                    crate::core::Color::White => {
                        player.mana_pool.white += amount;
                        mana.white = amount;
                        "W"
                    }
                    crate::core::Color::Blue => {
                        player.mana_pool.blue += amount;
                        mana.blue = amount;
                        "U"
                    }
                    crate::core::Color::Black => {
                        player.mana_pool.black += amount;
                        mana.black = amount;
                        "B"
                    }
                    crate::core::Color::Red => {
                        player.mana_pool.red += amount;
                        mana.red = amount;
                        "R"
                    }
                    crate::core::Color::Green => {
                        player.mana_pool.green += amount;
                        mana.green = amount;
                        "G"
                    }
                    crate::core::Color::Colorless => {
                        player.mana_pool.colorless += amount;
                        mana.colorless = amount;
                        "C"
                    }
                };
                self.undo_log
                    .log(crate::undo::GameAction::AddMana { player_id, mana }, prior_log_size);

                // Log visible message (use gamelog for official action)
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    // Render amount-many pips, e.g. Black Lotus → "Tap Black Lotus for {G}{G}{G}".
                    let pip = format!("{{{}}}", color_symbol);
                    let pips: String = pip.repeat(amount as usize);
                    let message = format!("Tap {} for {}", name, pips);
                    self.logger.gamelog(&message);
                }
            } else {
                // Add the specific mana from the ability
                if mana_to_add.white > 0 {
                    player.mana_pool.white += mana_to_add.white;
                }
                if mana_to_add.blue > 0 {
                    player.mana_pool.blue += mana_to_add.blue;
                }
                if mana_to_add.black > 0 {
                    player.mana_pool.black += mana_to_add.black;
                }
                if mana_to_add.red > 0 {
                    player.mana_pool.red += mana_to_add.red;
                }
                if mana_to_add.green > 0 {
                    player.mana_pool.green += mana_to_add.green;
                }
                if mana_to_add.colorless > 0 {
                    player.mana_pool.colorless += mana_to_add.colorless;
                }

                self.undo_log.log(
                    crate::undo::GameAction::AddMana {
                        player_id,
                        mana: mana_to_add,
                    },
                    prior_log_size,
                );

                // Log visible message (use gamelog for official action)
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let message = format!("Tap {} for mana", name);
                    self.logger.gamelog(&message);
                }
            }

            // Pay any additional costs from the mana ability (e.g., sacrifice for Black Lotus)
            // For non-land mana sources, handle sacrifice costs before returning
            if let Some(cost) = mana_ability_cost {
                use crate::core::Cost;
                match cost {
                    Cost::Tap => {
                        // Already handled above
                    }
                    Cost::SacrificePattern { .. } | Cost::Sacrifice { .. } => {
                        // Pay the sacrifice cost (moves permanent to graveyard)
                        self.pay_ability_cost(player_id, card_id, &cost)?;
                    }
                    Cost::Composite(costs) => {
                        // For composite costs, pay everything except tap (already paid)
                        for sub_cost in costs {
                            if !matches!(sub_cost, Cost::Tap) {
                                self.pay_ability_cost(player_id, card_id, &sub_cost)?;
                            }
                        }
                    }
                    // Other costs not yet handled by mana abilities:
                    Cost::Untap
                    | Cost::Mana(_)
                    | Cost::TapAndMana(_)
                    | Cost::PayLife { .. }
                    | Cost::Discard { .. }
                    | Cost::DiscardHand
                    | Cost::Waterbend { .. }
                    | Cost::AddLoyalty { .. }
                    | Cost::SubLoyalty { .. }
                    | Cost::SubCounter { .. } => {
                        // These cost types aren't currently used in mana ability costs
                    }
                }
            }

            return Ok(());
        }

        // Add mana to player's pool based on land type
        // For basic lands and simple cases, check subtypes
        // For dual lands (e.g., Underground Sea = Island Swamp), we need smarter logic
        // First, check subtypes and mana production cache before we borrow player_mut
        // Get mana production info and build available colors from BOTH subtypes AND mana production cache
        // This handles both basic lands (with subtypes) and non-basic dual lands (with Choice abilities)
        let (is_any_color_land, produces_colorless, available_colors) = {
            let card = self.cards.get(card_id)?;
            // Use pre-computed cache for mana production type (derived from abilities, not text)
            let is_any_color = matches!(
                card.definition.cache.mana_production.kind,
                crate::core::ManaProductionKind::AnyColor
            );
            let is_colorless = matches!(
                card.definition.cache.mana_production.kind,
                crate::core::ManaProductionKind::Colorless
            );

            // Build available_colors from BOTH sources:
            // 1. Land subtypes (Island, Forest, etc.) - for basic/dual lands with land types
            // 2. ManaProductionKind::Choice - for non-basic duals like Blooming Marsh
            let mut colors = Vec::new();

            // First, add colors from land subtypes
            if card.definition.cache.has_plains_subtype {
                colors.push(crate::core::Color::White);
            }
            if card.definition.cache.has_island_subtype {
                colors.push(crate::core::Color::Blue);
            }
            if card.definition.cache.has_swamp_subtype {
                colors.push(crate::core::Color::Black);
            }
            if card.definition.cache.has_mountain_subtype {
                colors.push(crate::core::Color::Red);
            }
            if card.definition.cache.has_forest_subtype {
                colors.push(crate::core::Color::Green);
            }

            // Second, add colors from mana production cache (for non-basic lands)
            // This handles lands without basic land subtypes
            use crate::core::ManaColor;
            match &card.definition.cache.mana_production.kind {
                crate::core::ManaProductionKind::Fixed(mana_color) => {
                    // Non-basic land that produces a fixed color (e.g., Ba Sing Se produces {G})
                    let color = match mana_color {
                        ManaColor::White => crate::core::Color::White,
                        ManaColor::Blue => crate::core::Color::Blue,
                        ManaColor::Black => crate::core::Color::Black,
                        ManaColor::Red => crate::core::Color::Red,
                        ManaColor::Green => crate::core::Color::Green,
                    };
                    if !colors.contains(&color) {
                        colors.push(color);
                    }
                }
                crate::core::ManaProductionKind::Choice(mana_colors) => {
                    // Dual/multi lands (e.g., Blooming Marsh)
                    if mana_colors.contains(ManaColor::White) && !colors.contains(&crate::core::Color::White) {
                        colors.push(crate::core::Color::White);
                    }
                    if mana_colors.contains(ManaColor::Blue) && !colors.contains(&crate::core::Color::Blue) {
                        colors.push(crate::core::Color::Blue);
                    }
                    if mana_colors.contains(ManaColor::Black) && !colors.contains(&crate::core::Color::Black) {
                        colors.push(crate::core::Color::Black);
                    }
                    if mana_colors.contains(ManaColor::Red) && !colors.contains(&crate::core::Color::Red) {
                        colors.push(crate::core::Color::Red);
                    }
                    if mana_colors.contains(ManaColor::Green) && !colors.contains(&crate::core::Color::Green) {
                        colors.push(crate::core::Color::Green);
                    }
                }
                crate::core::ManaProductionKind::AnyColor | crate::core::ManaProductionKind::Colorless => {
                    // Handled by is_any_color and is_colorless checks
                }
            }

            // Third, add chosen_color for lands like Thriving Grove
            // (cards with "choose a color" ETB effects that produce mana of that color)
            if let Some(chosen) = card.chosen_color {
                if !colors.contains(&chosen) {
                    colors.push(chosen);
                }
            }

            (is_any_color, is_colorless, colors)
        };

        // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
        let prior_log_size = self.logger.log_count();

        let player = self.get_player_mut(player_id)?;

        let color = if is_any_color_land || available_colors.len() > 1 {
            // Multi-color or any-color land: choose based on cost hint
            // Produce the first color needed by the cost that this land can produce
            if cost_hint.white > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::White)) {
                Some(crate::core::Color::White)
            } else if cost_hint.blue > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::Blue))
            {
                Some(crate::core::Color::Blue)
            } else if cost_hint.black > 0
                && (is_any_color_land || available_colors.contains(&crate::core::Color::Black))
            {
                Some(crate::core::Color::Black)
            } else if cost_hint.red > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::Red)) {
                Some(crate::core::Color::Red)
            } else if cost_hint.green > 0
                && (is_any_color_land || available_colors.contains(&crate::core::Color::Green))
            {
                Some(crate::core::Color::Green)
            } else {
                // Cost doesn't need a specific color - produce the first available color
                available_colors.first().copied().or(Some(crate::core::Color::White))
            }
        } else if available_colors.len() == 1 {
            // Single-color land
            available_colors.first().copied()
        } else if produces_colorless {
            // Colorless mana land (e.g., Mishra's Factory, Wastes)
            Some(crate::core::Color::Colorless)
        } else {
            // Unknown land type
            None
        };

        if let Some(color) = color {
            player.mana_pool.add_color(color);

            // Log the mana addition
            let mut mana = crate::core::ManaCost::new();
            let color_symbol = match color {
                crate::core::Color::White => {
                    mana.white = 1;
                    "W"
                }
                crate::core::Color::Blue => {
                    mana.blue = 1;
                    "U"
                }
                crate::core::Color::Black => {
                    mana.black = 1;
                    "B"
                }
                crate::core::Color::Red => {
                    mana.red = 1;
                    "R"
                }
                crate::core::Color::Green => {
                    mana.green = 1;
                    "G"
                }
                crate::core::Color::Colorless => {
                    mana.colorless = 1;
                    "C"
                }
            };
            self.undo_log
                .log(crate::undo::GameAction::AddMana { player_id, mana }, prior_log_size);

            // Log visible message for mana tapping (use gamelog for official action)
            if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                let message = format!("Tap {} for {{{}}}", card_name, color_symbol);
                self.logger.gamelog(&message);
            }
        }

        // Note: For lands, mana_ability_cost is None (set at line 1623), so no additional
        // costs need to be paid. Non-land mana sources with sacrifice costs are handled
        // in the explicit_mana path above (lines 1760-1784), which returns early.

        Ok(())
    }

    /// Pay the cost for an activated ability
    ///
    /// This method pays costs in the correct order:
    /// 1. Tap costs (must happen before zone changes)
    /// 2. Mana costs (pay from mana pool)
    /// 3. Other costs (sacrifice, discard, etc.) - TODO
    ///
    /// Returns Ok(()) if costs were successfully paid, Err otherwise.
    ///
    /// Note: This is a simplified implementation. Full implementation would:
    /// - Support cost refund if payment fails midway
    /// - Handle cost ordering more comprehensively
    /// - Support all cost types (sacrifice, discard, pay life, etc.)
    ///
    /// # Errors
    ///
    /// Returns an error if the cost cannot be paid.
    pub fn pay_ability_cost(&mut self, player_id: PlayerId, card_id: CardId, cost: &crate::core::Cost) -> Result<()> {
        use crate::core::{Cost, ManaCost};

        match cost {
            Cost::Tap => {
                // Tap the permanent (this updates cache and increments mana_version)
                self.tap_permanent(card_id)?;
                // Check for Taps triggers (e.g., Gran-Gran: "Whenever ~ becomes tapped")
                self.check_triggers(TriggerEvent::Taps, card_id)?;
                Ok(())
            }

            Cost::Mana(mana_cost) => {
                // Pay mana from pool
                let player = self.get_player_mut(player_id)?;
                if !player.mana_pool.can_pay(mana_cost) {
                    return Err(MtgError::InvalidAction("Cannot pay mana cost".to_string()));
                }
                player.mana_pool.pay_cost(mana_cost).map_err(MtgError::InvalidAction)?;
                Ok(())
            }

            Cost::TapAndMana(mana_cost) => {
                // Pay both tap and mana
                // Tap first (must happen before zone changes)
                // Check if already tapped
                {
                    let card = self.cards.get(card_id)?;
                    if card.tapped {
                        return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
                    }
                }

                // Tap the permanent (this updates cache and increments mana_version)
                self.tap_permanent(card_id)?;
                // Check for Taps triggers (e.g., Gran-Gran: "Whenever ~ becomes tapped")
                self.check_triggers(TriggerEvent::Taps, card_id)?;

                // Then pay mana
                let player = self.get_player_mut(player_id)?;
                if !player.mana_pool.can_pay(mana_cost) {
                    // TODO: Should refund the tap here
                    return Err(MtgError::InvalidAction("Cannot pay mana cost".to_string()));
                }
                player.mana_pool.pay_cost(mana_cost).map_err(MtgError::InvalidAction)?;
                Ok(())
            }

            Cost::PayLife { amount } => {
                // Pay life
                let player = self.get_player_mut(player_id)?;
                if player.life < *amount {
                    return Err(MtgError::InvalidAction("Not enough life".to_string()));
                }
                player.life -= amount;
                Ok(())
            }

            Cost::Untap => {
                // Untap the permanent
                let card = self.cards.get_mut(card_id)?;
                if !card.tapped {
                    return Err(MtgError::InvalidAction("Permanent is not tapped".to_string()));
                }
                card.untap();
                Ok(())
            }

            Cost::SacrificePattern { count, card_type } => {
                // Find permanents matching the pattern and sacrifice them
                // For now, automatically choose without asking the controller
                // TODO: Let controller choose which permanents to sacrifice

                let mut to_sacrifice = Vec::new();

                // Special case: CARDNAME means the card with this ability
                if card_type == "CARDNAME" {
                    to_sacrifice.push(card_id);
                } else {
                    // Find permanents on battlefield matching the type
                    // Collect IDs first to avoid borrowing issues
                    let battlefield_cards = self.battlefield.cards.to_vec();

                    for permanent_id in battlefield_cards {
                        if to_sacrifice.len() >= *count as usize {
                            break;
                        }

                        let card = self.cards.get(permanent_id)?;

                        // Check ownership
                        if card.owner != player_id {
                            continue;
                        }

                        // Check if it matches the pattern
                        let matches = if card_type == "Land" {
                            card.is_land()
                        } else if card_type.starts_with("Creature") {
                            if card_type == "Creature.Other" {
                                // Other means not the card with the ability
                                card.is_creature() && permanent_id != card_id
                            } else {
                                card.is_creature()
                            }
                        } else if card_type == "Artifact" {
                            card.is_artifact()
                        } else {
                            // Generic type match - check if any type contains the string
                            card.types.iter().any(|t| format!("{t:?}").contains(card_type))
                        };

                        if matches {
                            to_sacrifice.push(permanent_id);
                        }
                    }
                }

                // Check if we found enough permanents to sacrifice
                if to_sacrifice.len() < *count as usize {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough permanents of type {} to sacrifice (need {}, found {})",
                        card_type,
                        count,
                        to_sacrifice.len()
                    )));
                }

                // Sacrifice the permanents (move to graveyard or exile if finality) and check triggers
                for sac_id in to_sacrifice.iter().take(*count as usize) {
                    let owner = self.cards.get(*sac_id)?.owner;
                    let dest = self.death_destination_for_card(*sac_id);
                    self.move_card(*sac_id, Zone::Battlefield, dest, owner)?;
                    // Check sacrifice triggers (e.g., Pirate Peddlers)
                    self.check_triggers(TriggerEvent::Sacrificed, *sac_id)?;
                }

                Ok(())
            }

            Cost::Sacrifice { card_id: sac_id } => {
                // Sacrifice a specific permanent (move to graveyard or exile if finality)
                let owner = self.cards.get(*sac_id)?.owner;
                let dest = self.death_destination_for_card(*sac_id);
                self.move_card(*sac_id, Zone::Battlefield, dest, owner)?;
                // Check sacrifice triggers
                self.check_triggers(TriggerEvent::Sacrificed, *sac_id)
            }

            Cost::Discard { card_id: _ } => {
                // TODO: Implement discard cost for specific card
                Err(MtgError::InvalidAction(format!(
                    "Cost type {cost:?} not yet implemented"
                )))
            }

            Cost::DiscardHand => {
                // Discard entire hand (e.g., Slate of Ancestry)
                if let Some(zones) = self.get_player_zones(player_id) {
                    let hand_cards: Vec<CardId> = zones.hand.cards.clone();
                    for &hand_card_id in &hand_cards {
                        self.move_card(hand_card_id, Zone::Hand, Zone::Graveyard, player_id)?;
                    }
                    self.logger.normal(&format!(
                        "{} discards their hand ({} cards)",
                        self.get_player(player_id)?.name,
                        hand_cards.len()
                    ));
                }
                Ok(())
            }

            Cost::Composite(costs) => {
                // Pay each cost in order
                for sub_cost in costs {
                    self.pay_ability_cost(player_id, card_id, sub_cost)?;
                }
                Ok(())
            }

            Cost::Waterbend { amount } => {
                // Waterbend cost - Avatar set mechanic (like Convoke)
                // Player can tap untapped creatures/artifacts to pay for {1} each.
                // Player can also tap lands to produce mana.
                // Total payment = mana from lands + tapped creatures/artifacts + floating mana

                // Get current floating mana
                let floating_mana = {
                    let player = self.get_player(player_id)?;
                    player.mana_pool.total()
                };

                // Find untapped mana sources (lands) controlled by this player
                let battlefield_cards = self.battlefield.cards.to_vec();
                let mana_sources: Vec<CardId> = battlefield_cards
                    .iter()
                    .filter(|&&cid| {
                        if cid == card_id {
                            return false; // Can't tap the source to pay its own cost
                        }
                        if let Some(card) = self.cards.try_get(cid) {
                            // Must be untapped land controlled by player with mana ability
                            !card.tapped && card.controller == player_id && card.is_land()
                        } else {
                            false
                        }
                    })
                    .copied()
                    .collect();

                // Find untapped creatures and artifacts controlled by this player
                // (excluding the source card and mana sources - they're counted above)
                let tappable_permanents: Vec<CardId> = battlefield_cards
                    .into_iter()
                    .filter(|&cid| {
                        if cid == card_id {
                            return false; // Can't tap the source to pay its own cost
                        }
                        if mana_sources.contains(&cid) {
                            return false; // Already counted as mana source
                        }
                        if let Some(card) = self.cards.try_get(cid) {
                            // Must be untapped, controlled by player, and be creature or artifact
                            !card.tapped && card.controller == player_id && (card.is_creature() || card.is_artifact())
                        } else {
                            false
                        }
                    })
                    .collect();

                let total_available = floating_mana + mana_sources.len() as u8 + tappable_permanents.len() as u8;

                if total_available < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Cannot pay Waterbend {}: only {} available (floating: {}, lands: {}, tappable: {})",
                        amount,
                        total_available,
                        floating_mana,
                        mana_sources.len(),
                        tappable_permanents.len()
                    )));
                }

                // Payment strategy: prefer tapping creatures/artifacts first, then lands
                // This preserves mana sources for future use when possible
                let mut remaining = *amount;

                // First use floating mana
                if remaining > 0 && floating_mana > 0 {
                    let use_from_pool = remaining.min(floating_mana);
                    let mana_cost = ManaCost::from_string(&use_from_pool.to_string());
                    let player = self.get_player_mut(player_id)?;
                    player.mana_pool.pay_cost(&mana_cost).map_err(MtgError::InvalidAction)?;
                    remaining -= use_from_pool;
                }

                // Then tap creatures/artifacts for waterbend
                for &perm_id in &tappable_permanents {
                    if remaining == 0 {
                        break;
                    }
                    if let Ok(card) = self.cards.get_mut(perm_id) {
                        card.tapped = true;
                        remaining -= 1;
                    }
                }

                // Finally tap lands to produce mana
                for &land_id in &mana_sources {
                    if remaining == 0 {
                        break;
                    }
                    if let Ok(card) = self.cards.get_mut(land_id) {
                        card.tapped = true;
                        remaining -= 1;
                        // Note: We're not adding mana to pool since we're directly counting
                        // each land tap as {1} payment for simplicity
                    }
                }

                Ok(())
            }

            Cost::AddLoyalty { amount } => {
                // Planeswalker +N loyalty ability: add N loyalty counters
                use crate::core::CounterType;
                let prior_log_size = self.logger.log_count();
                let card = self.cards.get_mut(card_id)?;
                card.add_counter(CounterType::Loyalty, *amount);
                let old_loyalty_flag = card.loyalty_activated_this_turn;
                card.loyalty_activated_this_turn = true; // MTG CR 606.3: once per turn
                self.undo_log.log(
                    crate::undo::GameAction::SetLoyaltyActivated {
                        card_id,
                        old_value: old_loyalty_flag,
                        new_value: true,
                    },
                    prior_log_size,
                );
                let new_loyalty = card.get_counter(CounterType::Loyalty);
                self.logger
                    .verbose(&format!("{} gains {} loyalty (now {})", card.name, amount, new_loyalty));
                Ok(())
            }

            Cost::SubLoyalty { amount } => {
                // Planeswalker -N loyalty ability: remove N loyalty counters
                use crate::core::CounterType;
                let prior_log_size = self.logger.log_count();
                let current = self.cards.get(card_id)?.get_counter(CounterType::Loyalty);
                if current < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough loyalty counters ({} < {}) on {}",
                        current,
                        amount,
                        self.cards.get(card_id)?.name
                    )));
                }
                let card = self.cards.get_mut(card_id)?;
                card.remove_counter(CounterType::Loyalty, *amount);
                let old_loyalty_flag = card.loyalty_activated_this_turn;
                card.loyalty_activated_this_turn = true; // MTG CR 606.3: once per turn
                self.undo_log.log(
                    crate::undo::GameAction::SetLoyaltyActivated {
                        card_id,
                        old_value: old_loyalty_flag,
                        new_value: true,
                    },
                    prior_log_size,
                );
                let new_loyalty = card.get_counter(CounterType::Loyalty);
                let card_name = card.name.to_string();
                self.logger
                    .verbose(&format!("{} loses {} loyalty (now {})", card_name, amount, new_loyalty));

                // Check if loyalty reaches 0 - planeswalker dies (MTG CR 704.5i)
                if new_loyalty == 0 {
                    self.logger
                        .normal(&format!("{} has 0 loyalty and is put into the graveyard", card_name));
                    let owner = self.cards.get(card_id)?.owner;
                    let dest = self.death_destination_for_card(card_id);
                    self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                }
                Ok(())
            }

            Cost::SubCounter { amount, counter_type } => {
                // Generic counter-removal cost (e.g. Triskelion's
                // SubCounter<1/P1P1>). Distinct from Cost::SubLoyalty so we
                // don't tag the activation as the once-per-turn planeswalker
                // ability and don't enforce "0 counters → graveyard" — Triskelion
                // happily lives at 1/1 with zero P1P1 counters.
                let current = self.cards.get(card_id)?.get_counter(*counter_type);
                if current < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough {:?} counters ({} < {}) on {}",
                        counter_type,
                        current,
                        amount,
                        self.cards.get(card_id)?.name
                    )));
                }
                let card = self.cards.get_mut(card_id)?;
                card.remove_counter(*counter_type, *amount);
                let card_name = card.name.to_string();
                let new_count = card.get_counter(*counter_type);
                self.logger.verbose(&format!(
                    "{} loses {} {:?} counter(s) (now {})",
                    card_name, amount, counter_type, new_count
                ));
                Ok(())
            }
        }
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

                        // Log the discard
                        if let Some(card) = self.cards.try_get(card_id) {
                            let player_name = self
                                .get_player(player_id)
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| "Player".to_string());
                            self.logger
                                .gamelog(&format!("{} discards {} to Balance", player_name, card.name));
                        }
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
                    // Check card type filter
                    let type_matches = if filter.starts_with("Artifact") {
                        card.is_artifact()
                    } else if filter.starts_with("Creature") {
                        card.is_creature()
                    } else if filter.starts_with("Land") {
                        card.is_land()
                    } else if filter.starts_with("Enchantment") {
                        card.is_enchantment()
                    } else if filter.starts_with("Permanent") || filter.starts_with("Card") {
                        true // Any permanent
                    } else {
                        // Unknown type, assume it matches if we can't parse
                        log::warn!(target: "count", "Unknown filter type in count expression: {}", filter);
                        true
                    };

                    if !type_matches {
                        return false;
                    }

                    // Check controller filter
                    if filter.contains("OppCtrl") {
                        // Opponents control - not the controller
                        card.controller != controller
                    } else if filter.contains("YouCtrl") {
                        // You control
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

// Submodules
mod combat;
mod targeting;

#[cfg(test)]
mod tests;
