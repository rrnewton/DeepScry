//! Control-change and permanent-state effect handlers extracted from the
//! `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that change who controls a permanent, make two creatures
//! fight, or grant a short-lived permanent-state shield/flag:
//! - [`Effect::GainControl`] — steal control (CR 720, Control Magic / Aladdin),
//! - [`Effect::Fight`] — two creatures deal damage to each other (CR 701.12),
//! - [`Effect::GrantCantBeBlocked`] — until-EOT can't-be-blocked,
//! - [`Effect::Regenerate`] — add a regeneration shield (CR 701.15a),
//! - [`Effect::AttachEquipment`] — attach Equipment to a creature.
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::{effects::ControlDuration, CardId, PlayerId};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::GainControl`]: change `target`'s controller to `new_controller`
    /// (optionally untapping it), with the given `duration`. A
    /// `WhileControlSource` grant records `(source, grantee)` so
    /// `recompute_source_control` reverts it when the grantee loses the source
    /// (Aladdin). Logged for undo.
    pub(in crate::game::actions) fn execute_gain_control(
        &mut self,
        target: CardId,
        new_controller: PlayerId,
        untap: bool,
        duration: &ControlDuration,
        source: Option<CardId>,
    ) -> Result<()> {
        // Skip if target is still placeholder
        if target.is_placeholder() {
            return Ok(());
        }
        // Skip if target is not on battlefield
        if !self.battlefield.contains(target) {
            log::debug!(target: "gain_control", "GainControl fizzled: target {} not on battlefield", target.as_u32());
            return Ok(());
        }

        let prior_log_size = self.logger.log_count();
        let (old_controller, target_name) = {
            let card = self.cards.get(target)?;
            (card.controller, card.name.to_string())
        };
        let new_ctrl_name = self
            .get_player(new_controller)
            .map(|p| p.name.clone())
            .unwrap_or_else(|_| format!("P{}", new_controller.as_u32()).into());

        // Change controller, and for a source-duration grant record the
        // (source, grantee) so `recompute_source_control` reverts it when
        // the grantee stops controlling the source (Aladdin).
        {
            let card = self.cards.get_mut(target)?;
            card.controller = new_controller;
            match duration {
                ControlDuration::WhileControlSource => {
                    if let Some(src) = source {
                        card.control_grant = Some((src, new_controller));
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
                card_id: target,
                old_controller,
                new_controller,
            },
            prior_log_size,
        );

        // Optionally untap the stolen permanent
        if untap {
            self.untap_permanent(target)?;
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
        Ok(())
    }

    /// [`Effect::Fight`] (CR 701.12): `fighter` and `target` each deal damage
    /// equal to their power to the other (only when power > 0). Fizzles unless
    /// both are on the battlefield.
    pub(in crate::game::actions) fn execute_fight(&mut self, fighter: CardId, target: CardId) -> Result<()> {
        // CR 701.12: Fight - each creature deals damage equal to its power to the other
        if fighter.is_placeholder() || target.is_placeholder() {
            return Ok(());
        }
        // Both creatures must be on the battlefield
        if !self.battlefield.contains(fighter) || !self.battlefield.contains(target) {
            log::debug!(target: "fight", "Fight fizzled: fighter or target not on battlefield");
            return Ok(());
        }
        // Get power values before dealing damage
        let fighter_power = self.get_effective_power(fighter).unwrap_or_else(|_| {
            self.cards
                .get(fighter)
                .map(|c| i32::from(c.current_power()))
                .unwrap_or(0)
        });
        let target_power = self.get_effective_power(target).unwrap_or_else(|_| {
            self.cards
                .get(target)
                .map(|c| i32::from(c.current_power()))
                .unwrap_or(0)
        });

        let fighter_name = self.cards.get(fighter).map(|c| c.name.to_string()).unwrap_or_default();
        let target_name = self.cards.get(target).map(|c| c.name.to_string()).unwrap_or_default();

        // CR 701.12a: Each creature deals damage equal to its power to the other
        // Only deal damage if power > 0
        if fighter_power > 0 {
            self.deal_damage_to_creature(target, fighter_power)?;
        }
        if target_power > 0 {
            self.deal_damage_to_creature(fighter, target_power)?;
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
        Ok(())
    }

    /// [`Effect::GrantCantBeBlocked`]: install an until-EOT can't-be-blocked
    /// persistent effect on the target creature.
    pub(in crate::game::actions) fn execute_grant_cant_be_blocked(&mut self, target: CardId) -> Result<()> {
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
        let card_name = self.cards.get(target).map(|c| c.name.as_str()).unwrap_or("Unknown");

        // Get the effect controller (the player who activated the ability)
        let controller = self.turn.active_player;

        // Create a PersistentEffect granting "can't be blocked"
        use crate::core::{CleanupCondition, PersistentEffectKind};

        self.persistent_effects.add(
            PersistentEffectKind::CantBeBlocked { creature: target },
            target,     // source_card - the targeted creature
            controller, // controller - the active player
            CleanupCondition::EndOfTurn,
        );

        // Log the effect
        self.logger
            .gamelog(&format!("{} can't be blocked this turn", card_name));
        Ok(())
    }

    /// [`Effect::Regenerate`]: add a regeneration shield to the target (CR
    /// 701.15a — "the next time it would be destroyed this turn, instead remove
    /// all damage, tap it, and remove it from combat").
    pub(in crate::game::actions) fn execute_regenerate(&mut self, target: CardId) -> Result<()> {
        // Regenerate: Add a regeneration shield to target permanent (CR 701.15a)
        // "The next time [permanent] would be destroyed this turn, instead
        // remove all damage marked on it, tap it, and remove it from combat."
        if target.is_placeholder() {
            return Ok(());
        }
        if !self.battlefield.contains(target) {
            return Ok(());
        }
        let card = self.cards.get_mut(target)?;
        card.regeneration_shields = card.regeneration_shields.saturating_add(1);
        let card_name = self.cards.get(target).map(|c| c.name.as_str()).unwrap_or("Unknown");
        self.logger
            .gamelog(&format!("{} ({}) gains a regeneration shield", card_name, target));
        Ok(())
    }

    /// [`Effect::AttachEquipment`]: attach `source_equipment` to
    /// `target_creature` (fizzles if the target left the battlefield).
    pub(in crate::game::actions) fn execute_attach_equipment(
        &mut self,
        source_equipment: CardId,
        target_creature: CardId,
    ) -> Result<()> {
        // Attach Equipment to target creature
        // Skip if target is not on battlefield (fizzle)
        if !self.battlefield.contains(target_creature) {
            // Ability fizzles - target not on battlefield
            return Ok(());
        }

        // Call the attach_equipment method
        self.attach_equipment(source_equipment, target_creature)?;
        Ok(())
    }
}
