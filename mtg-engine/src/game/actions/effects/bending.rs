//! "Bending" effect-family handlers (Avatar: The Last Airbender set mechanics)
//! extracted from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the three custom Avatar-set bending effects:
//! - [`Effect::Airbend`] — exile a permanent, grant its owner permission to
//!   cast it for {2} (CR 701.65b-style mass-exile-and-may-play).
//! - [`Effect::Earthbend`] — animate a land into a 0/0 haste creature with N
//!   +1/+1 counters, returning to the battlefield when it dies/is exiled.
//! - [`Effect::Firebend`] — add N red combat-mana to the controller's pool
//!   (lasts until end of combat).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim,
//! including the undo-log bookkeeping (mtg-610 keyword grants, mtg-614 base-stat
//! overrides, mtg-ba6uq combat-mana snapshot) that keeps rewind+replay
//! bit-identical.

use crate::core::{CardId, PlayerId};
use crate::game::GameState;
use crate::zones::Zone;
use crate::Result;

impl GameState {
    /// [`Effect::Airbend`]: exile the target and grant its owner permission to
    /// cast it for {2} until it leaves exile or is cast (CR 701.65b). Fizzles on
    /// an unresolved/placeholder target.
    pub(in crate::game::actions) fn execute_airbend(&mut self, target: CardId) -> Result<()> {
        // Skip if target is still placeholder (0) - no valid targets found
        if target.is_placeholder() {
            // Ability fizzles - no valid targets
            return Ok(());
        }

        // Get card info before exile
        let (owner, card_name) = {
            let card = self.cards.get(target)?;
            (card.owner, card.name.clone())
        };

        // Move card from battlefield to exile
        self.move_card(target, Zone::Battlefield, Zone::Exile, owner)?;

        // Create a PersistentEffect granting MayPlay from exile for {2}
        use crate::core::{CleanupCondition, ManaCost, PersistentEffectKind};

        let cleanup = CleanupCondition::Any(vec![
            CleanupCondition::TrackedCardLeavesZone {
                card: target,
                zone: Zone::Exile,
            },
            CleanupCondition::TrackedCardIsCast { card: target },
        ]);

        self.persistent_effects.add(
            PersistentEffectKind::MayPlayFromExile {
                tracked_card: target,
                alternative_cost: ManaCost::from_string("2"), // {2} alternative cost
                owner,
            },
            target, // source_card - the airbended card itself is the source
            owner,  // controller - the owner controls this permission
            cleanup,
        );

        // Log the airbend
        self.logger.gamelog(&format!(
            "{} is airbended (exiled, owner may cast for {{2}})",
            card_name
        ));
        Ok(())
    }

    /// [`Effect::Earthbend`]: animate the target land into a 0/0 haste creature
    /// (it stays a land) with `num_counters` +1/+1 counters, plus a delayed
    /// trigger to return it to the battlefield tapped when it dies or is exiled.
    /// Errors if the target is not a land; fizzles on a placeholder target.
    pub(in crate::game::actions) fn execute_earthbend(&mut self, target: CardId, num_counters: u8) -> Result<()> {
        // Skip if target is still placeholder (0) - no valid targets found
        if target.is_placeholder() {
            // Ability fizzles - no valid targets
            return Ok(());
        }

        // Validate the target is a land before earthbending.
        let card_name = {
            let card = self.cards.get_mut(target)?;

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
        self.earthbend_animate_creature_haste_logged(target);

        // Set temp base power/toughness to 0/0 (animate effect) via the
        // logged helper so the override is reversible by the undo log
        // (mtg-614 hole (c)).
        self.set_temp_base_stats_logged(target, Some(0), Some(0));

        // Add +1/+1 counters
        use crate::core::CounterType;
        self.add_counters(target, CounterType::P1P1, num_counters)?;

        // Get controller for the delayed trigger
        let controller = self.turn.active_player;

        // Register delayed trigger: when this land dies or is exiled, return it to battlefield tapped
        use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerCondition};
        use smallvec::smallvec;

        let trigger = DelayedTrigger::new(
            crate::core::DelayedTriggerId::new(0), // ID will be assigned by store
            target,                                // tracked_card
            target,                                // source_card (the land itself)
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
        Ok(())
    }

    /// [`Effect::Firebend`]: add `amount` red mana to the controller's COMBAT
    /// mana pool (cleared at end of combat, not at end of step). Snapshots the
    /// combat pool for undo first (mtg-ba6uq #7).
    pub(in crate::game::actions) fn execute_firebend(&mut self, controller: PlayerId, amount: u8) -> Result<()> {
        // Get player name before mutable borrow for logging
        let player_name = self
            .get_player(controller)
            .map(|p| p.name.clone())
            .unwrap_or_else(|_| "Unknown".into());

        // Snapshot the combat mana pool for undo BEFORE adding (mtg-ba6uq #7).
        self.log_combat_mana_pool(controller);
        // Add red mana to combat mana pool (lazy initialization)
        let player = self.get_player_mut(controller)?;
        for _ in 0..amount {
            player.add_combat_mana(crate::core::Color::Red);
        }

        // Log the firebend
        self.logger.gamelog(&format!(
            "{} adds {} {{R}} (combat mana, lasts until end of combat)",
            player_name, amount
        ));
        Ok(())
    }
}
