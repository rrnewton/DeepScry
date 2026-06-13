//! Life-total and mana-pool effect-family handlers extracted from the
//! `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that change a player's life total or empty their mana
//! pool:
//! - [`Effect::GainLife`] / [`Effect::GainLifeDynamic`] (CR 119.3),
//! - [`Effect::LoseLife`] (CR 119.3),
//! - [`Effect::SetLife`] (CR 119.5),
//! - [`Effect::DrainMana`] — "lose all unspent mana" (Power Sink rider).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::{DynamicAmount, PlayerId};
use crate::game::log_event::LogEvent;
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::GainLife`]: the player gains `amount` life (CR 119.3). A
    /// zero-amount gain still resolves but is not logged (avoids noisy
    /// "gains 0 life" output). Records a `ModifyLife` undo entry.
    pub(in crate::game::actions) fn execute_gain_life(&mut self, player: PlayerId, amount: i32) -> Result<()> {
        // Capture log size before life gain
        let prior_log_size = self.logger.log_count();

        let p = self.get_player_mut(player)?;
        let player_name = p.name.clone();
        p.gain_life(amount);
        let new_life = p.life;

        // Emit a gamelog line so reproducers can verify the life gain
        // (CR 119.3). A zero-life gain still resolved but changed nothing;
        // skip the line in that case to avoid noisy "gains 0 life" output.
        if amount > 0 {
            self.logger
                .gamelog(&format!("{} gains {} life (life: {})", player_name, amount, new_life));
            self.logger.push_event(LogEvent::LifeChanged {
                player,
                delta: amount,
                new_total: new_life,
            });
        }

        // Log the life gain
        self.undo_log.log(
            crate::undo::GameAction::ModifyLife {
                player_id: player,
                delta: amount,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::GainLifeDynamic`]: gain life equal to a value computed from
    /// public, last-known game state at resolution time (CR 608.2g) — e.g.
    /// Swords to Plowshares' "gain life equal to its power" where the creature
    /// may already have been exiled earlier in this same resolution.
    pub(in crate::game::actions) fn execute_gain_life_dynamic(
        &mut self,
        player: PlayerId,
        amount: &DynamicAmount,
        reference: crate::core::CardId,
    ) -> Result<()> {
        // Resolve the dynamic amount from public, last-known game state
        // at resolution time (CR 608.2g). The referenced card may already
        // have left the battlefield earlier in this resolution (e.g.
        // Swords exiled the creature) — its retained characteristics in
        // the entity store are its last-known information.
        let resolved_amount = self.resolve_dynamic_amount(amount, reference, player);
        let prior_log_size = self.logger.log_count();

        let p = self.get_player_mut(player)?;
        let player_name = p.name.clone();
        p.gain_life(resolved_amount);
        let new_life = p.life;

        self.logger.gamelog(&format!(
            "{} gains {} life (life: {})",
            player_name, resolved_amount, new_life
        ));
        if resolved_amount > 0 {
            self.logger.push_event(LogEvent::LifeChanged {
                player,
                delta: resolved_amount,
                new_total: new_life,
            });
        }

        self.undo_log.log(
            crate::undo::GameAction::ModifyLife {
                player_id: player,
                delta: resolved_amount,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::LoseLife`]: the player loses `amount` life (CR 119.3).
    /// Records a `ModifyLife` undo entry with a negated delta.
    pub(in crate::game::actions) fn execute_lose_life(&mut self, player: PlayerId, amount: i32) -> Result<()> {
        let prior_log_size = self.logger.log_count();

        let p = self.get_player_mut(player)?;
        let player_name = p.name.clone();
        p.lose_life(amount);
        let new_life = p.life;

        self.logger
            .gamelog(&format!("{} loses {} life (life: {})", player_name, amount, new_life));
        self.logger.push_event(LogEvent::LifeChanged {
            player,
            delta: -amount,
            new_total: new_life,
        });

        self.undo_log.log(
            crate::undo::GameAction::ModifyLife {
                player_id: player,
                delta: -amount,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::SetLife`]: set a player's life total to a specific amount
    /// (CR 119.5 — the player gains or loses the necessary amount of life).
    pub(in crate::game::actions) fn execute_set_life(&mut self, player: PlayerId, amount: i32) -> Result<()> {
        // CR 119.5: "If an effect sets a player's life total, the player gains
        // or loses the necessary amount of life"
        let prior_log_size = self.logger.log_count();
        let p = self.get_player_mut(player)?;
        let player_name = p.name.clone();
        let old_life = p.life;
        let delta = amount - old_life;
        p.life = amount;
        self.logger.gamelog(&format!(
            "{}'s life total is set to {} (was {})",
            player_name, amount, old_life
        ));
        // Record as a ModifyLife delta so rewind/replay restores the old life
        // total correctly (matches how GainLife and LoseLife are logged).
        self.undo_log.log(
            crate::undo::GameAction::ModifyLife {
                player_id: player,
                delta,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::DrainMana`] — "lose all unspent mana" (Power Sink). CR 500.4
    /// empties pools automatically at step/phase end; this rider forces it
    /// immediately so the player can't spend mana floated for the countered
    /// spell.
    pub(in crate::game::actions) fn execute_drain_mana(&mut self, player: PlayerId) -> Result<()> {
        let (player_name, amount) = self
            .get_player(player)
            .map(|p| (p.name.to_string(), p.mana_pool.total()))
            .unwrap_or_else(|_| (format!("Player {}", player.as_u32()), 0));
        if let Some(p) = self.players.iter_mut().find(|p| p.id == player) {
            p.empty_mana_pool();
        }
        self.logger
            .gamelog(&format!("{} loses all unspent mana ({} drained)", player_name, amount));
        Ok(())
    }
}
