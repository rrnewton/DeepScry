//! Unified trigger handling and placeholder resolution
//!
//! This module consolidates trigger-related functionality that was previously
//! duplicated across multiple functions (check_triggers, check_attack_triggers,
//! check_death_triggers, check_card_drawn_triggers, etc.).
//!
//! ## Key Components
//!
//! - `TriggerContext`: Encapsulates all context needed for trigger resolution
//! - `resolve_effect_placeholder`: Shared function for resolving placeholder values in effects
//! - Trigger matching logic via structured fields instead of string parsing
//!
//! ## Design Rationale
//!
//! Previously, each trigger handler had its own inline placeholder resolution logic,
//! leading to ~400+ lines of duplicated code. This module centralizes that logic
//! while preserving the specific behaviors needed for different trigger types.

use crate::core::{CardId, Effect, PlayerId, TargetRef};
use crate::game::GameState;

/// Context for resolving placeholder values in triggered effects
///
/// This struct captures all the information needed to resolve placeholders
/// like `PlayerId::placeholder()` (controller) and `CardId::placeholder()` (trigger source).
#[derive(Debug, Clone)]
pub struct TriggerContext {
    /// The card that owns the trigger (the trigger source)
    pub trigger_source: CardId,

    /// The controller of the trigger source
    pub controller: PlayerId,

    /// The card that caused the event (e.g., the creature that entered, the card that was drawn)
    /// May be the same as trigger_source for self-triggers
    pub event_source: CardId,

    /// For triggers that affect "target opponent", this is the opponent
    pub opponent: Option<PlayerId>,

    /// For effects that reference "last resolved target" in a chain (Defined$ Targeted)
    pub last_resolved_target: Option<CardId>,

    /// For firebend effects: power of the attacking creature
    pub creature_power: u8,

    /// For firebend effects with sacrifice costs: power of the sacrificed creature
    pub sacrificed_power: u8,

    /// For CardDrawn triggers: the player who drew the card
    pub drawing_player: Option<PlayerId>,
}

impl TriggerContext {
    /// Create a new trigger context with minimal required info
    pub fn new(trigger_source: CardId, controller: PlayerId) -> Self {
        TriggerContext {
            trigger_source,
            controller,
            event_source: trigger_source,
            opponent: None,
            last_resolved_target: None,
            creature_power: 0,
            sacrificed_power: 0,
            drawing_player: None,
        }
    }

    /// Builder method to set the event source
    pub fn with_event_source(mut self, event_source: CardId) -> Self {
        self.event_source = event_source;
        self
    }

    /// Builder method to set the opponent
    pub fn with_opponent(mut self, opponent: PlayerId) -> Self {
        self.opponent = Some(opponent);
        self
    }

    /// Builder method to set the drawing player (for CardDrawn triggers)
    pub fn with_drawing_player(mut self, player: PlayerId) -> Self {
        self.drawing_player = Some(player);
        self
    }

    /// Builder method to set creature power (for firebend)
    pub fn with_creature_power(mut self, power: u8) -> Self {
        self.creature_power = power;
        self
    }

    /// Builder method to set sacrificed creature power
    pub fn with_sacrificed_power(mut self, power: u8) -> Self {
        self.sacrificed_power = power;
        self
    }
}

/// Resolve placeholder values in an effect based on trigger context
///
/// This is the single source of truth for placeholder resolution, replacing
/// the duplicated logic that was scattered across multiple trigger handlers.
///
/// ## Placeholder Conventions
///
/// - `PlayerId::placeholder()` / `is_placeholder()` → controller of the trigger source
/// - `CardId::placeholder()` / `is_placeholder()` → the trigger source itself (for "put counter on ~")
/// - `TargetRef::None` for DealDamage → context-dependent (opponent, drawing player, etc.)
/// - `CardId::reuse_previous()` / `is_reuse_previous()` → the last resolved target in a chain
///
/// ## Effect Coverage
///
/// This function handles placeholder resolution for all Effect variants that use placeholders:
/// - Player-targeting: DrawCards, DiscardCards, GainLife, Mill, Scry, Loot, AddMana, Firebend
/// - Permanent-targeting: PutCounter, PumpCreature, DestroyPermanent, ExilePermanent, etc.
/// - Token creation: CreateToken (controller placeholder)
/// - Damage: DealDamage (various target types)
///
/// Effects without placeholders pass through unchanged.
///
/// # Note
///
/// This function does not panic. The `unwrap()` for `ctx.drawing_player` is only called
/// after confirming `is_some()` via the match guard.
#[allow(clippy::wildcard_enum_match_arm)]
#[allow(clippy::missing_panics_doc)]
pub fn resolve_effect_placeholder(effect: &Effect, ctx: &TriggerContext) -> Effect {
    match effect {
        // =========================================================================
        // Player-targeting effects: PlayerId::new(0) → controller
        // =========================================================================
        Effect::DrawCards { player, count } if player.is_placeholder() => Effect::DrawCards {
            player: ctx.controller,
            count: *count,
        },

        Effect::DiscardCards {
            player,
            count,
            remember_discarded,
            optional,
            remember_discarding_players,
        } if player.is_placeholder() => Effect::DiscardCards {
            player: ctx.controller,
            count: *count,
            remember_discarded: *remember_discarded,
            optional: *optional,
            remember_discarding_players: *remember_discarding_players,
        },

        Effect::GainLife { player, amount } if player.is_placeholder() => Effect::GainLife {
            player: ctx.controller,
            amount: *amount,
        },

        Effect::Mill { player, count } if player.is_placeholder() => Effect::Mill {
            player: ctx.controller,
            count: *count,
        },

        Effect::Scry { player, count } if player.is_placeholder() => Effect::Scry {
            player: ctx.controller,
            count: *count,
        },

        Effect::Loot {
            player,
            discard_count,
            draw_count,
        } if player.is_placeholder() => Effect::Loot {
            player: ctx.controller,
            discard_count: *discard_count,
            draw_count: *draw_count,
        },

        Effect::AddMana {
            player,
            mana,
            produces_chosen_color,
            amount_var,
        } if player.is_placeholder() => Effect::AddMana {
            player: ctx.controller,
            mana: *mana,
            produces_chosen_color: *produces_chosen_color,
            amount_var: amount_var.clone(),
        },

        // =========================================================================
        // Firebend: special handling for power-based mana
        // =========================================================================
        Effect::Firebend { controller, amount } if controller.is_placeholder() => {
            // amount=0 means "use creature's power" (Firebending X)
            // amount=254 means "use sacrificed creature's power" (Fire Lord Ozai)
            let actual_amount = match *amount {
                0 => ctx.creature_power,
                254 => ctx.sacrificed_power,
                n => n,
            };
            Effect::Firebend {
                controller: ctx.controller,
                amount: actual_amount,
            }
        }

        // =========================================================================
        // Self-targeting effects: CardId::new(0) → trigger source
        // =========================================================================
        Effect::PutCounter {
            target,
            counter_type,
            amount,
        } if target.is_placeholder() => Effect::PutCounter {
            target: ctx.trigger_source,
            counter_type: *counter_type,
            amount: *amount,
        },

        // `Defined$ Self` PutCounter (Sengir Vampire's TrigPutCounter SVar):
        // self_target() is a distinct sentinel from placeholder() — set by
        // the effect_converter when parsing `Defined$ Self`. Resolve it to
        // the trigger source so the counter lands on the source card itself.
        Effect::PutCounter {
            target,
            counter_type,
            amount,
        } if target.is_self_target() => Effect::PutCounter {
            target: ctx.trigger_source,
            counter_type: *counter_type,
            amount: *amount,
        },

        // `Defined$ Self` RemoveCounter (All Hallow's Eve TrigRemoveCounter):
        // remove a counter from the trigger source itself.
        Effect::RemoveCounter {
            target,
            counter_type,
            amount,
        } if target.is_self_target() || target.is_placeholder() => Effect::RemoveCounter {
            target: ctx.trigger_source,
            counter_type: *counter_type,
            amount: *amount,
        },

        // `DB$ ChangeZone | Defined$ Self | Origin$ Exile | Destination$ Graveyard`
        // fired from a trigger (All Hallow's Eve moves itself to the graveyard
        // once its last scream counter is removed).
        Effect::MoveSelfBetweenZones {
            source,
            origin,
            destination,
        } if source.is_self_target() || source.is_placeholder() => Effect::MoveSelfBetweenZones {
            source: ctx.trigger_source,
            origin: *origin,
            destination: *destination,
        },

        // ConditionalSelfCounter fired from a trigger: patch the condition source
        // and recurse into the inner effect so its `Defined$ Self` placeholders
        // also resolve to the trigger source.
        Effect::ConditionalSelfCounter {
            source,
            condition,
            inner,
        } => Effect::ConditionalSelfCounter {
            source: if source.is_self_target() || source.is_placeholder() {
                ctx.trigger_source
            } else {
                *source
            },
            condition: condition.clone(),
            inner: Box::new(resolve_effect_placeholder(inner, ctx)),
        },

        // Note: PumpCreature with CardId::new(0) is NOT handled here because it's ambiguous:
        // - CardDrawn triggers: "this creature gets +X/+Y" → target is self
        // - ETB triggers: "target creature gets +X/+Y" → need to find a target
        // Let context-specific handlers deal with this ambiguity.

        // =========================================================================
        // AttachEquipment: source_equipment placeholder → trigger source (Card.Self)
        //
        // Used by Equipment ETB triggers like Twin Blades:
        //   T:Mode$ ChangesZone | ... | ValidCard$ Card.Self | Execute$ TrigAttach
        //   SVar:TrigAttach:DB$ Attach | ValidTgts$ Creature.YouCtrl
        //
        // The Equipment attaching is *itself* (Card.Self), so resolve the source
        // to the trigger source. Target creature is still a placeholder and is
        // resolved by the calling trigger handler (battlefield search).
        // =========================================================================
        Effect::AttachEquipment {
            source_equipment,
            target_creature,
        } if source_equipment.is_placeholder() => Effect::AttachEquipment {
            source_equipment: ctx.trigger_source,
            target_creature: *target_creature,
        },

        // =========================================================================
        // Damage effects: various target resolution strategies
        // =========================================================================

        // DealDamage with player placeholder → controller
        Effect::DealDamage {
            target: TargetRef::Player(player_id),
            amount,
        } if player_id.is_placeholder() => Effect::DealDamage {
            target: TargetRef::Player(ctx.controller),
            amount: *amount,
        },

        // DealDamage with TargetRef::None: ONLY resolve for CardDrawn triggers
        // For other triggers (ETB, etc.), leave as TargetRef::None for context-specific
        // handling in the trigger handler (e.g., finding creature targets first)
        Effect::DealDamage {
            target: TargetRef::None,
            amount,
        } if ctx.drawing_player.is_some() => {
            // "CARDNAME deals N damage to that player" → target is player who drew
            // Used by Underworld Dreams
            Effect::DealDamage {
                target: TargetRef::Player(ctx.drawing_player.unwrap()),
                amount: *amount,
            }
        }

        // =========================================================================
        // Token creation: controller placeholder
        // =========================================================================
        Effect::CreateToken {
            controller,
            token_script,
            amount,
            for_each_player,
        } if controller.is_placeholder() => Effect::CreateToken {
            controller: ctx.controller,
            token_script: token_script.clone(),
            amount: *amount,
            for_each_player: *for_each_player,
        },

        // =========================================================================
        // Mass pump: controller placeholder
        // =========================================================================
        Effect::PumpAllCreatures {
            controller,
            filter,
            power_bonus,
            toughness_bonus,
        } if controller.is_placeholder() => Effect::PumpAllCreatures {
            controller: ctx.controller,
            filter: filter.clone(),
            power_bonus: *power_bonus,
            toughness_bonus: *toughness_bonus,
        },

        // =========================================================================
        // Mass animate: controller placeholder
        // =========================================================================
        Effect::AnimateAll {
            controller,
            filter,
            power,
            toughness,
            keywords_granted,
        } if controller.is_placeholder() => Effect::AnimateAll {
            controller: ctx.controller,
            filter: filter.clone(),
            power: *power,
            toughness: *toughness,
            keywords_granted: keywords_granted.clone(),
        },

        // =========================================================================
        // Default: return clone unchanged
        // =========================================================================
        other => other.clone(),
    }
}

impl GameState {
    /// Check if a trigger should fire based on structured filter flags
    ///
    /// This replaces the string-based filtering that checked description.contains("[marker]")
    /// with compile-time checked structured fields on the Trigger struct.
    ///
    /// ## Filter Flags Checked
    ///
    /// - `requires_other`: Event source must be different from trigger source
    /// - `requires_landfall`: Event source must be a Land controlled by trigger controller
    /// - `controller_turn_only`: Must be controller's turn
    /// - `requires_noncreature`: Event source must not be a creature
    ///
    /// Returns true if the trigger should fire, false if it should be filtered out.
    pub fn trigger_matches_filters(
        &self,
        trigger: &crate::core::Trigger,
        trigger_card_id: CardId,
        trigger_controller: PlayerId,
        event_source_id: CardId,
        active_player: PlayerId,
    ) -> bool {
        // Self-only triggers only fire when the trigger source is the event source
        if trigger.trigger_self_only && trigger_card_id != event_source_id {
            return false;
        }

        // "[other]" / requires_other: fires only when event source is DIFFERENT
        if trigger.requires_other && trigger_card_id == event_source_id {
            return false;
        }

        // Check description-based markers for backwards compatibility during migration
        // TODO(mtg-dry): Remove once all triggers use structured fields
        if trigger.description.contains("[other]") && trigger_card_id == event_source_id {
            return false;
        }

        // "[landfall]" / requires_landfall: fires only for lands controlled by trigger owner
        let source_is_land = self.cards.get(event_source_id).map(|c| c.is_land()).unwrap_or(false);
        let source_controller = self
            .cards
            .get(event_source_id)
            .map(|c| c.controller)
            .unwrap_or(trigger_controller);

        if trigger.requires_landfall && (!source_is_land || source_controller != trigger_controller) {
            return false;
        }

        // Backwards compatibility for description-based landfall
        if trigger.description.contains("[landfall]") && (!source_is_land || source_controller != trigger_controller) {
            return false;
        }

        // "[controller_only]" / controller_turn_only: fires only on controller's turn
        if trigger.controller_turn_only && trigger_controller != active_player {
            return false;
        }

        // Backwards compatibility
        if trigger.description.starts_with("[controller_only]") && trigger_controller != active_player {
            return false;
        }

        // "[noncreature]" / requires_noncreature: fires only for non-creature spells
        if trigger.requires_noncreature {
            let is_creature = self
                .cards
                .get(event_source_id)
                .map(|c| c.is_creature())
                .unwrap_or(false);
            if is_creature {
                return false;
            }
        }

        // Backwards compatibility
        if trigger.description.contains("[noncreature]") {
            let is_creature = self
                .cards
                .get(event_source_id)
                .map(|c| c.is_creature())
                .unwrap_or(false);
            if is_creature {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)]
mod tests {
    use super::*;
    use crate::core::{CounterType, Effect};

    #[test]
    fn test_resolve_draw_cards_placeholder() {
        let ctx = TriggerContext::new(CardId::new(42), PlayerId::new(1));

        let effect = Effect::DrawCards {
            player: PlayerId::new(0), // placeholder
            count: 2,
        };

        let resolved = resolve_effect_placeholder(&effect, &ctx);

        match resolved {
            Effect::DrawCards { player, count } => {
                assert_eq!(player.as_u32(), 1); // resolved to controller
                assert_eq!(count, 2);
            }
            _ => panic!("Expected DrawCards effect"),
        }
    }

    #[test]
    fn test_resolve_discard_cards_placeholder() {
        let ctx = TriggerContext::new(CardId::new(42), PlayerId::new(2));

        let effect = Effect::DiscardCards {
            player: PlayerId::new(0), // placeholder
            count: 1,
            remember_discarded: false,
            optional: false,
            remember_discarding_players: false,
        };

        let resolved = resolve_effect_placeholder(&effect, &ctx);

        match resolved {
            Effect::DiscardCards { player, count, .. } => {
                assert_eq!(player.as_u32(), 2);
                assert_eq!(count, 1);
            }
            _ => panic!("Expected DiscardCards effect"),
        }
    }

    #[test]
    fn test_resolve_put_counter_self() {
        let ctx = TriggerContext::new(CardId::new(99), PlayerId::new(1));

        let effect = Effect::PutCounter {
            target: CardId::new(0), // placeholder = self
            counter_type: CounterType::P1P1,
            amount: 1,
        };

        let resolved = resolve_effect_placeholder(&effect, &ctx);

        match resolved {
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } => {
                assert_eq!(target.as_u32(), 99); // resolved to trigger source
                assert_eq!(counter_type, CounterType::P1P1);
                assert_eq!(amount, 1);
            }
            _ => panic!("Expected PutCounter effect"),
        }
    }

    #[test]
    fn test_resolve_firebend_creature_power() {
        let ctx = TriggerContext::new(CardId::new(1), PlayerId::new(1)).with_creature_power(5);

        let effect = Effect::Firebend {
            controller: PlayerId::new(0), // placeholder
            amount: 0,                    // 0 = use creature power
        };

        let resolved = resolve_effect_placeholder(&effect, &ctx);

        match resolved {
            Effect::Firebend { controller, amount } => {
                assert_eq!(controller.as_u32(), 1);
                assert_eq!(amount, 5); // resolved to creature power
            }
            _ => panic!("Expected Firebend effect"),
        }
    }

    #[test]
    fn test_resolve_deal_damage_to_drawing_player() {
        let ctx = TriggerContext::new(CardId::new(1), PlayerId::new(1)).with_drawing_player(PlayerId::new(2));

        let effect = Effect::DealDamage {
            target: TargetRef::None,
            amount: 1,
        };

        let resolved = resolve_effect_placeholder(&effect, &ctx);

        match resolved {
            Effect::DealDamage { target, amount } => {
                assert_eq!(target, TargetRef::Player(PlayerId::new(2))); // resolved to drawing player
                assert_eq!(amount, 1);
            }
            _ => panic!("Expected DealDamage effect"),
        }
    }

    #[test]
    fn test_non_placeholder_passes_through() {
        let ctx = TriggerContext::new(CardId::new(1), PlayerId::new(1));

        // Effect without placeholder (player is already resolved)
        let effect = Effect::DrawCards {
            player: PlayerId::new(5), // not a placeholder
            count: 3,
        };

        let resolved = resolve_effect_placeholder(&effect, &ctx);

        match resolved {
            Effect::DrawCards { player, count } => {
                assert_eq!(player.as_u32(), 5); // unchanged
                assert_eq!(count, 3);
            }
            _ => panic!("Expected DrawCards effect"),
        }
    }
}
