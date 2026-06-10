//! Damage-effect family handlers extracted from the `execute_effect`
//! dispatcher (see `game/actions/mod.rs`).
//!
//! This module groups every `Effect` variant whose primary job is dealing or
//! preventing damage:
//! - direct/divided/dynamic damage to a target ([`Effect::DealDamage`],
//!   [`Effect::DealDamageDivided`], [`Effect::DealDamageDynamic`],
//!   [`Effect::DealDamageToTriggeredPlayer`], [`Effect::DealDamageXPaid`]),
//! - "each X deals damage to Y" ([`Effect::EachDamage`]),
//! - mass damage ([`Effect::DamageAll`]),
//! - damage prevention shields ([`Effect::PreventDamage`],
//!   [`Effect::PreventDamageFromSource`],
//!   [`Effect::PreventAllCombatDamageThisTurn`]).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. This is a behavior-preserving structural split:
//! the method bodies are moved verbatim from the original match arms.

use crate::core::{CardId, PlayerId, TargetRef};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::DealDamage`]: deal `amount` damage to a single player or
    /// permanent. A `TargetRef::None` means the effect fizzled with no legal
    /// target (CR 608.2b).
    pub(in crate::game::actions) fn execute_deal_damage(&mut self, target: &TargetRef, amount: i32) -> Result<()> {
        match target {
            TargetRef::Player(player_id) => {
                self.deal_damage(*player_id, amount)?;
            }
            TargetRef::Permanent(card_id) => {
                self.deal_damage_to_creature(*card_id, amount)?;
            }
            TargetRef::None => {
                // Spell fizzles - no valid target (CR 608.2b)
                // This happens when triggered damage effects fire with no valid target
                log::debug!("DealDamage fizzles - no target specified");
            }
        }
        Ok(())
    }

    /// [`Effect::DealDamageDivided`] — DivideEvenly$ RoundedDown resolved form
    /// (Fireball): deal `amount_each` to every chosen target. The source is set
    /// via `current_damage_source` by the caller (`resolve_spell_effects`), so
    /// this is a single source dealing simultaneous damage to N targets
    /// (CR 601.2d / 118.5).
    pub(in crate::game::actions) fn execute_deal_damage_divided(
        &mut self,
        targets: &[TargetRef],
        amount_each: i32,
    ) -> Result<()> {
        if amount_each <= 0 {
            log::debug!("DealDamageDivided: amount_each={}, no damage dealt", amount_each);
        } else {
            for target in targets {
                match target {
                    TargetRef::Player(player_id) => {
                        self.deal_damage(*player_id, amount_each)?;
                    }
                    TargetRef::Permanent(card_id) => {
                        self.deal_damage_to_creature(*card_id, amount_each)?;
                    }
                    TargetRef::None => {
                        log::debug!("DealDamageDivided: skipping unresolved target");
                    }
                }
            }
        }
        Ok(())
    }

    /// [`Effect::EachDamage`]: each `damager` on the battlefield deals damage to
    /// the `receiver` (its own power if `use_card_power`, else `fixed_damage`).
    /// Fizzles cleanly if the receiver is unresolved or already gone.
    pub(in crate::game::actions) fn execute_each_damage(
        &mut self,
        damagers: &[CardId],
        receiver: CardId,
        use_card_power: bool,
        fixed_damage: i32,
    ) -> Result<()> {
        // Each damager deals damage to the receiver
        // If receiver is placeholder, the effect wasn't resolved - fizzle
        if receiver.is_placeholder() {
            log::debug!("EachDamage: receiver not resolved, fizzling");
            return Ok(());
        }

        // Check if receiver is still valid
        if !self.battlefield.contains(receiver) {
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
            let damage = if use_card_power {
                // Get damager's current power (includes counters and bonuses)
                self.cards
                    .get(*damager_id)
                    .map(|c| i32::from(c.current_power()))
                    .unwrap_or(0)
            } else {
                fixed_damage
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
                    .get(receiver)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|_| "creature".to_string());

                self.logger.normal(&format!(
                    "{} deals {} damage to {}",
                    damager_name, damage, receiver_name
                ));

                self.deal_damage_to_creature(receiver, damage)?;
            }
        }
        Ok(())
    }

    /// [`Effect::DamageAll`]: deal `amount` damage to every creature matching
    /// `valid_cards`, optionally also to every player (`damage_players`).
    /// Pyroclasm / Earthquake-style sweeps.
    pub(in crate::game::actions) fn execute_damage_all(
        &mut self,
        amount: i32,
        valid_cards: &crate::core::effects::TargetRestriction,
        damage_players: bool,
    ) -> Result<()> {
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
            // Snapshot marked damage for undo BEFORE mutating (mtg-728 sig-2f).
            self.log_damage(card_id);
            let card = self.cards.get_mut(card_id)?;
            card.damage += amount;
            let card_name = card.name.clone();
            let total_damage = card.damage;
            self.logger.gamelog(&format!(
                "{} ({}) takes {} damage (total: {})",
                card_name, card_id, amount, total_damage
            ));
        }

        // Optionally damage all players
        if damage_players {
            let player_ids: Vec<_> = self.players.iter().map(|p| p.id).collect();
            for pid in player_ids {
                let p = self.get_player_mut(pid)?;
                let player_name = p.name.clone();
                p.lose_life(amount);
                let new_life = p.life;
                self.logger
                    .gamelog(&format!("{} takes {} damage (life: {})", player_name, amount, new_life));
            }
        }

        // Check for creatures that took lethal damage
        self.check_lethal_damage()?;
        Ok(())
    }

    /// [`Effect::PreventDamage`]: add a "prevent the next N damage" shield to a
    /// player or permanent for the rest of the turn (CR 615.1).
    pub(in crate::game::actions) fn execute_prevent_damage(&mut self, target: &TargetRef, amount: i32) -> Result<()> {
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
        Ok(())
    }

    /// [`Effect::PreventDamageFromSource`] — Circle of Protection: install a
    /// source-filtered prevention shield protecting `protected` from the chosen
    /// colored `source` for the rest of the turn (CR 615.1, 615.6). The shield
    /// is consumed by the next matching damage event.
    pub(in crate::game::actions) fn execute_prevent_damage_from_source(
        &mut self,
        protected: PlayerId,
        color: crate::core::Color,
        source: CardId,
    ) -> Result<()> {
        if protected.is_placeholder() || source.is_placeholder() {
            log::debug!("PreventDamageFromSource unresolved (placeholder), skipping");
            return Ok(());
        }
        let shield = crate::core::DamagePreventionShield::colored_source_next_event(color, source);
        let source_name = self.cards.get(source).map(|c| c.name.to_string()).unwrap_or_default();
        // Snapshot the shield list for undo BEFORE installing the new
        // shield (mtg-ba6uq #6).
        self.log_source_prevention_shields(protected);
        let player = self.get_player_mut(protected)?;
        player.source_prevention_shields.push(shield);
        let player_name = self
            .get_player(protected)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        self.logger.gamelog(&format!(
            "The next time {} ({}) would deal damage to {} this turn, prevent that damage",
            source_name, source, player_name
        ));
        Ok(())
    }

    /// [`Effect::PreventAllCombatDamageThisTurn`] — Maze of Ith: prevent all
    /// combat damage this creature would deal or receive this turn (CR 615
    /// replacement). Sets `Card::prevent_all_combat_damage_this_turn`, cleared
    /// at cleanup; `assign_combat_damage` checks it before dealing/receiving
    /// combat damage.
    pub(in crate::game::actions) fn execute_prevent_all_combat_damage_this_turn(
        &mut self,
        target: CardId,
    ) -> Result<()> {
        if target.is_placeholder() {
            log::debug!(
                target: "maze_of_ith",
                "PreventAllCombatDamageThisTurn: target is still placeholder, skipping"
            );
            return Ok(());
        }
        let card = self.cards.get_mut(target)?;
        card.prevent_all_combat_damage_this_turn = true;
        let card_name = self
            .cards
            .get(target)
            .map(|c| c.name.as_str())
            .unwrap_or("?")
            .to_string();
        self.logger.gamelog(&format!(
            "Prevent all combat damage that would be dealt to and by {} ({}) this turn",
            card_name, target
        ));
        Ok(())
    }
}

// NOTE on the X-cost damage placeholders and unresolved dynamic variants
// (`DealDamageDynamic`, `DealDamageToTriggeredPlayer`, `DealDamageXPaid`): these
// are resolved into concrete `DealDamage` effects EARLIER (by
// `resolve_effect_target` / `check_triggers_for_controller` / X-cost
// resolution) and should never reach `execute_effect`. The dispatcher handles
// the "reached unresolved" fallback inline (log + fizzle / treat as 0) because
// the behavior is a one-liner and keeping it at the dispatch site documents the
// invariant where the match lives. See `mod.rs::execute_effect`.
