//! Miscellaneous resource / turn-structure / control-flow effect handlers
//! extracted from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the small, self-contained effects that did not warrant a family of
//! their own:
//! - **mana:** [`Effect::AddMana`] (Dark Ritual / Su-Chi-style mana into the
//!   pool), [`Effect::ChooseColor`],
//! - **turn structure:** [`Effect::AddTurn`] (CR 500.7 extra turns),
//!   [`Effect::AddPhase`] (extra combat phases),
//! - **scratch / control-flow:** [`Effect::ClearRemembered`],
//!   [`Effect::Clone`] (routing-guard fallback), [`Effect::Unimplemented`],
//!   [`Effect::NoOp`].
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::{CardId, Color, ManaCost, PlayerId, TargetType};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::AddMana`]: add the components of `mana` to the player's pool.
    /// Spell/triggered-ability path (Dark Ritual, Su-Chi); mana ABILITIES go
    /// through `tap_for_mana_for_cost` instead, where the source's chosen color
    /// and variable amount are available — so `produces_chosen_color` /
    /// `amount_var` reaching here is unexpected and warns.
    pub(in crate::game::actions) fn execute_add_mana(
        &mut self,
        player: PlayerId,
        mana: &ManaCost,
        produces_chosen_color: bool,
        amount_var: Option<&str>,
    ) -> Result<()> {
        // Capture log size before mana addition
        let prior_log_size = self.logger.log_count();

        // Add mana to player's mana pool
        // Note: For mana abilities, produces_chosen_color is handled in tap_for_mana_for_cost
        // where we have access to the source card's chosen_color.
        // This path is mainly for spell effects (Dark Ritual) and triggered abilities (Su-Chi).
        // Note: amount_var (for variable mana like Raucous Audience) is resolved in ManaEngine
        // during tap_for_mana_for_cost, not here.
        if produces_chosen_color {
            // This shouldn't happen in practice since mana abilities go through tap_for_mana_for_cost
            // but log a warning if it does
            self.logger
                .normal("Warning: produces_chosen_color in execute_effect - source card unknown");
        }
        // `remembered*N` — Metalworker-style: add `mana * remembered_amount * N`.
        // Encoded by effect_converter when it detects `Remembered$Amount[/Twice]`
        // in the SVar referenced by `Amount$`.
        let effective_mana = if let Some(var) = amount_var {
            if let Some(mult_str) = var.strip_prefix("remembered*") {
                let mult: u32 = mult_str.parse().unwrap_or(1);
                let remembered = self.remembered_amount.unwrap_or(0);
                let total = remembered * mult;
                mana.multiply(total as u8)
            } else {
                // Unknown variable: warn and fall through to base mana (1 unit).
                self.logger
                    .normal(&format!("Warning: unresolved amount_var '{var}' in execute_add_mana"));
                *mana
            }
        } else {
            *mana
        };
        let p = self.get_player_mut(player)?;

        // Add each component of the mana cost to the pool
        for _ in 0..effective_mana.white {
            p.mana_pool.add_color(Color::White);
        }
        for _ in 0..effective_mana.blue {
            p.mana_pool.add_color(Color::Blue);
        }
        for _ in 0..effective_mana.black {
            p.mana_pool.add_color(Color::Black);
        }
        for _ in 0..effective_mana.red {
            p.mana_pool.add_color(Color::Red);
        }
        for _ in 0..effective_mana.green {
            p.mana_pool.add_color(Color::Green);
        }
        for _ in 0..effective_mana.colorless {
            p.mana_pool.add_color(Color::Colorless);
        }

        // Log the mana addition
        self.undo_log.log(
            crate::undo::GameAction::AddMana {
                player_id: player,
                mana: effective_mana,
            },
            prior_log_size,
        );
        Ok(())
    }

    /// [`Effect::ChooseColor`]: pick a color (AI heuristic: the most prominent
    /// color in the player's deck) and store it on the `source` card.
    pub(in crate::game::actions) fn execute_choose_color(&mut self, player: PlayerId, source: CardId) -> Result<()> {
        // Choose a color using AI heuristic (pick most prominent color in deck)
        let chosen = self.pick_prominent_color(player, &[]);

        // Store the chosen color on the source card
        if let Ok(card) = self.cards.get_mut(source) {
            let card_name = card.name.clone();
            card.chosen_color = Some(chosen);
            let player_name = self
                .get_player(player)
                .map(|p| p.name.to_string())
                .unwrap_or_else(|_| format!("Player {}", player.as_u32()));
            self.logger
                .normal(&format!("{} chooses color: {:?} ({})", player_name, chosen, card_name));
        } else {
            log::warn!("ChooseColor: source card {} not found", source.as_u32());
        }
        Ok(())
    }

    /// [`Effect::AddTurn`]: queue `num_turns` extra turns for the player
    /// (CR 500.7 — Time Walk etc.). Pushes onto `extra_turns` (the queue the
    /// turn-rotation code actually drains) and logs each for undo (mtg-551 /
    /// mtg-559 / mtg-610).
    pub(in crate::game::actions) fn execute_add_turn(&mut self, player: PlayerId, num_turns: u8) -> Result<()> {
        // Take extra turns (CR 500.7) - Time Walk, Temporal Manipulation, etc.
        // Add extra turns to the GameState extra-turn queue (consumed in
        // GameState::advance_step at end of turn, CR 500.7). NOTE: this
        // must push to `self.extra_turns` (the VecDeque actually drained
        // by the turn-rotation code), NOT `self.turn.extra_turns` (a
        // dead, write-only field) — otherwise the extra turn was queued
        // somewhere nothing reads, and never taken (mtg-551).
        for _ in 0..num_turns {
            let prior_log_size = self.logger.log_count();
            self.extra_turns.push_back(player);
            // Log for undo so a rewind+replay across the AddTurn
            // resolution doesn't leave a stale queued extra turn
            // (mtg-559/mtg-610).
            self.undo_log
                .log(crate::undo::GameAction::PushExtraTurn { player }, prior_log_size);
        }
        let player_name = self
            .get_player(player)
            .map(|p| p.name.as_str().to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        self.logger.gamelog(&format!(
            "{} takes {} extra turn(s) after this one",
            player_name, num_turns
        ));
        Ok(())
    }

    /// [`Effect::AddPhase`]: add `count` extra combat phase(s) after the current
    /// step (Relentless Assault-style).
    pub(in crate::game::actions) fn execute_add_phase(&mut self, count: u8) -> Result<()> {
        // Add extra combat phase(s) after the current step
        for _ in 0..count {
            self.extra_combat_phases += 1;
        }
        self.logger
            .gamelog(&format!("AddPhase: {} additional combat phase(s) this turn", count));
        Ok(())
    }

    /// [`Effect::ClearRemembered`]: clear the remembered-card / remembered-player
    /// / remembered-amount scratch. Any numeric value (Mana Drain) was already
    /// captured onto its delayed trigger by the preceding CreateDelayedTrigger,
    /// so clearing here is safe. The amount clear is logged for undo.
    pub(in crate::game::actions) fn execute_clear_remembered(&mut self) -> Result<()> {
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
        Ok(())
    }

    /// [`Effect::Clone`] routing-guard fallback. Clone requires a controller
    /// decision ("you may" + which permanent to copy) and is resolved by the
    /// interactive path in `priority.rs::resolve_clone_effect`. Reaching
    /// execute_effect means that interception was bypassed — warn so the routing
    /// gap is visible rather than silently entering as a vanilla permanent.
    pub(in crate::game::actions) fn execute_clone_fallback(&mut self) -> Result<()> {
        log::warn!(
            target: "actions",
            "Effect::Clone reached execute_effect without controller interception; \
             permanent will not copy. This is a routing bug — Clone must go through \
             the interactive spell-resolution hook."
        );
        Ok(())
    }

    /// [`Effect::SkipUntapStep`]: set the `skip_untap_next_turn` flag on
    /// `player`, causing their next untap step to be skipped (CR 502.1).
    ///
    /// Yosei, the Morning Star die trigger:
    ///   `DB$ SkipPhase | ValidTgts$ Player | Step$ Untap`
    ///
    /// The flag is consumed (and cleared) at the start of the next
    /// `untap_step` for that player.
    pub(in crate::game::actions) fn execute_skip_untap_step(&mut self, player: PlayerId) -> Result<()> {
        let Some(p) = self.players.iter_mut().find(|p| p.id == player) else {
            return Ok(()); // Player has already lost; ignore gracefully.
        };
        let old_value = p.skip_untap_next_turn;
        p.skip_untap_next_turn = true;

        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetSkipUntapNextTurn {
                player_id: player,
                old_value,
                new_value: true,
            },
            prior_log_size,
        );

        let player_num = player.as_u32() + 1;
        self.logger
            .gamelog(&format!("P{} will skip their next untap step", player_num));
        Ok(())
    }

    /// [`Effect::Unimplemented`]: an effect API not yet modeled — log a warning
    /// and a gamelog line, then resolve as a no-op (so the gap is visible).
    pub(in crate::game::actions) fn execute_unimplemented(&mut self, api_type: &str) -> Result<()> {
        // Log a warning instead of silently doing nothing
        log::warn!(target: "actions", "Unimplemented effect '{}' resolved as no-op", api_type);
        self.logger.gamelog(&format!(
            "WARNING: Effect '{}' is not yet implemented - resolving as no-op",
            api_type
        ));
        Ok(())
    }

    /// [`Effect::NoOp`]: an *intentional* no-op (e.g. StoreSVar, whose value is
    /// modeled directly elsewhere). Silent — no warning, no gamelog.
    pub(in crate::game::actions) fn execute_noop(&self, api_type: &str) -> Result<()> {
        log::debug!(target: "actions", "NoOp effect '{}' (intentional)", api_type);
        Ok(())
    }

    /// [`Effect::ExtraLandPlay`]: grant `player` permission to play `amount`
    /// additional lands this turn.
    ///
    /// Creates a `PersistentEffectKind::ExtraLandPlay` with
    /// `CleanupCondition::EndOfTurn`.  The permanent form (Oracle of Mul Daya,
    /// Exploration enchantment, …) is handled via `StaticAbility::ExtraLandPlay`
    /// on the battlefield card and does NOT create a persistent effect.
    pub(in crate::game::actions) fn execute_extra_land_play(&mut self, player: PlayerId, amount: u8) -> Result<()> {
        use crate::core::{CleanupCondition, PersistentEffectKind};

        if amount == 0 {
            return Ok(());
        }

        let player_name = self
            .get_player(player)
            .map(|p| p.name.as_str().to_string())
            .unwrap_or_else(|_| format!("Player {}", player.as_u32()));

        // Use a sentinel source (card 0 = no specific source card)
        let source = crate::core::CardId::new(0);

        self.persistent_effects.add(
            PersistentEffectKind::ExtraLandPlay { player, amount },
            source,
            player,
            CleanupCondition::EndOfTurn,
        );

        let plural = if amount == 1 { "" } else { "s" };
        self.logger.gamelog(&format!(
            "{} may play {} additional land{} this turn.",
            player_name, amount, plural
        ));

        Ok(())
    }

    /// [`Effect::ChooseAndRememberOneOfEach`]: for each `TargetType` in `types`,
    /// choose one permanent of that type controlled by the current loop player
    /// (`remembered_players[0]`) and push the chosen card onto
    /// `game.remembered_cards`.
    ///
    /// Called from Tragic Arrogance's `YouChoose` SVar:
    ///   `DB$ ChooseCard | ChooseEach$ Artifact & Creature & Enchantment & Planeswalker
    ///    | ControlledByPlayer$ Remembered | RememberChosen$ True`
    ///
    /// The caster (YOU) picks for the current loop player.  AI heuristic:
    /// - For the caster's OWN permanents: keep the highest-mana-value one of
    ///   each type (save the best).
    /// - For an OPPONENT's permanents: keep the lowest-mana-value one of each
    ///   type (so the opponent's best get sacrificed).
    ///
    /// If no permanent of a given type is controlled by the loop player, that
    /// type is silently skipped (MTG rules: you can only choose from what exists).
    ///
    /// CR 701.17: "sacrifice a permanent" is used by the companion SacAllOthers;
    /// this step only REMEMBERS the chosen permanents.
    pub(in crate::game::actions) fn execute_choose_and_remember_one_of_each(
        &mut self,
        types: &[TargetType],
    ) -> Result<()> {
        // Who is the current loop player? (set by RepeatEach | RepeatPlayers$ Player)
        let loop_player = self.remembered_players.first().copied();
        let Some(loop_player_id) = loop_player else {
            log::warn!(
                target: "actions",
                "ChooseAndRememberOneOfEach: no remembered player to choose for; skipping"
            );
            return Ok(());
        };

        // Is the loop player the caster?  For this effect, "caster" = spell
        // controller.  We approximate as: is the loop player player 0 in
        // turn order?  More precisely we compare against `active_player` at
        // time of resolution, but we don't always have that.  Instead we use
        // the sign of the choice heuristic:
        //   - loop player == spell controller → keep best (high MV)
        //   - loop player != spell controller → keep worst (low MV, sacrifice best)
        //
        // Technically the card says the CASTER always decides (Defined$ You),
        // but the AI approximation is: the caster prefers to keep their own
        // best and sacrifice the opponent's best.  We identify "caster" as
        // whichever remembered_player is current (RepeatEach always runs the
        // active spell-controller as first or later); for simplicity we check
        // whether `loop_player_id` is player index 0 in `self.players`.
        let loop_player_is_first = self.players.first().is_some_and(|p| p.id == loop_player_id);

        for target_type in types {
            // Collect all permanents of this type controlled by loop_player_id
            let candidates: Vec<(CardId, u8)> = self
                .battlefield
                .cards
                .iter()
                .copied()
                .filter_map(|cid| {
                    let card = self.cards.try_get(cid)?;
                    // Must be controlled by the loop player
                    if card.controller != loop_player_id {
                        return None;
                    }
                    // Must match the target type
                    let type_restriction = crate::core::TargetRestriction::from_types([*target_type]);
                    if !type_restriction.matches(card) {
                        return None;
                    }
                    Some((cid, card.mana_cost.cmc()))
                })
                .collect();

            if candidates.is_empty() {
                // No permanent of this type for this player — skip silently (CR: you
                // choose from among permanents that player controls; if none exist,
                // there is nothing to choose and nothing survives for that type).
                log::debug!(
                    target: "actions",
                    "ChooseAndRememberOneOfEach: player {:?} controls no {:?}; skipping type",
                    loop_player_id, target_type
                );
                continue;
            }

            // Pick the one to KEEP (remember = save from sacrifice).
            // If this is the loop player's own permanents (loop_player_is_first):
            //   keep highest CMC (save the best)
            // If this is an opponent's permanents (!loop_player_is_first):
            //   keep lowest CMC (sacrifice the best)
            let chosen_id = if loop_player_is_first {
                // Keep best (highest CMC)
                candidates.iter().max_by_key(|(_, cmc)| *cmc).map(|(cid, _)| *cid)
            } else {
                // Keep worst (lowest CMC) so the best gets sacrificed
                candidates.iter().min_by_key(|(_, cmc)| *cmc).map(|(cid, _)| *cid)
            };

            if let Some(cid) = chosen_id {
                self.remembered_cards.push(cid);
                let card_name = self
                    .cards
                    .try_get(cid)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                log::debug!(
                    target: "actions",
                    "ChooseAndRememberOneOfEach: chose {:?} ({}) for type {:?} (player {:?})",
                    cid, card_name, target_type, loop_player_id
                );
                self.logger.gamelog(&format!(
                    "Player {:?} chooses to keep {} ({:?}) as their {:?}",
                    loop_player_id, card_name, cid, target_type
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, CardId, CardType, TargetType};
    use crate::game::GameState;

    /// Helper: create a permanent of the given `card_type` on the battlefield
    /// under `owner`'s control, with the given CMC set on its mana cost.
    fn create_typed_permanent(
        game: &mut GameState,
        owner: PlayerId,
        name: &str,
        card_type: CardType,
        cmc: u8,
    ) -> CardId {
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, name, owner);
        card.controller = owner;
        card.add_type(card_type);
        // Set a mana cost whose CMC matches `cmc`. Using generic mana is the
        // simplest way — ManaCost::from_generic(n).
        // Build a mana cost string with `cmc` generic mana
        let cost_str = cmc.to_string();
        card.mana_cost = crate::core::ManaCost::from_string(&cost_str);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        card_id
    }

    /// Tragic Arrogance scenario: P1 (loop player = "self") controls one
    /// Creature and one Artifact; P2 is the caster but is not the loop player.
    ///
    /// When `loop_player_is_first` is true (P1 == players[0]):
    ///   the executor should keep the highest-CMC permanent of each type.
    #[test]
    fn test_choose_and_remember_keeps_best_for_self() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id; // loop player
        let _p2 = game.players[1].id;

        // P1 controls two creatures: Grizzly Bears (CMC 2) and Serra Angel (CMC 5)
        let bears = create_typed_permanent(&mut game, p1, "Grizzly Bears", CardType::Creature, 2);
        let angel = create_typed_permanent(&mut game, p1, "Serra Angel", CardType::Creature, 5);
        // P1 controls two artifacts: Sol Ring (CMC 1) and Wurmcoil Engine (CMC 6)
        let sol_ring = create_typed_permanent(&mut game, p1, "Sol Ring", CardType::Artifact, 1);
        let wurmcoil = create_typed_permanent(&mut game, p1, "Wurmcoil Engine", CardType::Artifact, 6);

        // Set up: loop player = P1 (stored in remembered_players[0])
        game.remembered_players.clear();
        game.remembered_players.push(p1);
        game.remembered_cards.clear();

        let types = vec![TargetType::Creature, TargetType::Artifact];
        game.execute_choose_and_remember_one_of_each(&types)
            .expect("execute_choose_and_remember_one_of_each should succeed");

        // Should remember exactly 2 cards: best creature (Serra Angel, CMC 5)
        // and best artifact (Wurmcoil Engine, CMC 6).
        assert_eq!(
            game.remembered_cards.len(),
            2,
            "Expected 2 remembered cards, got {}: {:?}",
            game.remembered_cards.len(),
            game.remembered_cards
        );
        assert!(
            game.remembered_cards.contains(&angel),
            "Serra Angel (CMC 5, best creature) should be kept"
        );
        assert!(
            game.remembered_cards.contains(&wurmcoil),
            "Wurmcoil Engine (CMC 6, best artifact) should be kept"
        );
        // Weaker cards must NOT be in remembered (they will be sacrificed)
        assert!(
            !game.remembered_cards.contains(&bears),
            "Grizzly Bears should NOT be kept (lower CMC)"
        );
        assert!(
            !game.remembered_cards.contains(&sol_ring),
            "Sol Ring should NOT be kept (lower CMC)"
        );
    }

    /// When the loop player is NOT players[0] (i.e., an opponent's turn in the
    /// RepeatEach loop), the executor should keep the WEAKEST permanent of each
    /// type (so the opponent's best permanents get sacrificed).
    #[test]
    fn test_choose_and_remember_keeps_worst_for_opponent() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id; // caster (first player)
        let p2 = game.players[1].id; // loop player (opponent)

        // P2 controls two creatures: Grizzly Bears (CMC 2) and Serra Angel (CMC 5)
        let bears = create_typed_permanent(&mut game, p2, "Grizzly Bears", CardType::Creature, 2);
        let angel = create_typed_permanent(&mut game, p2, "Serra Angel", CardType::Creature, 5);

        // Set loop player = P2 (NOT the first player → opponent branch)
        game.remembered_players.clear();
        game.remembered_players.push(p2);
        game.remembered_cards.clear();

        let types = vec![TargetType::Creature];
        game.execute_choose_and_remember_one_of_each(&types)
            .expect("execute_choose_and_remember_one_of_each should succeed");

        assert_eq!(
            game.remembered_cards.len(),
            1,
            "Expected 1 remembered card, got {}: {:?}",
            game.remembered_cards.len(),
            game.remembered_cards
        );
        // Keep the WORST (lowest CMC) so the opponent's best gets sacrificed
        assert!(
            game.remembered_cards.contains(&bears),
            "Grizzly Bears (CMC 2, worst creature) should be kept for opponent"
        );
        assert!(
            !game.remembered_cards.contains(&angel),
            "Serra Angel (CMC 5, best creature) should NOT be kept (gets sacrificed)"
        );
        let _ = p1;
    }

    /// If the loop player controls no permanent of a given type, that type is
    /// silently skipped (no panic, no remembered card for that type).
    #[test]
    fn test_choose_and_remember_skips_missing_types() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id;
        let _p2 = game.players[1].id;

        // P1 controls only a Creature — no Artifact
        let _bear = create_typed_permanent(&mut game, p1, "Grizzly Bears", CardType::Creature, 2);

        game.remembered_players.clear();
        game.remembered_players.push(p1);
        game.remembered_cards.clear();

        // Ask for both Creature and Artifact
        let types = vec![TargetType::Creature, TargetType::Artifact];
        game.execute_choose_and_remember_one_of_each(&types)
            .expect("execute_choose_and_remember_one_of_each should succeed even with missing type");

        // Only 1 card remembered (the creature); Artifact type silently skipped
        assert_eq!(
            game.remembered_cards.len(),
            1,
            "Expected 1 remembered card (creature kept, no artifact to skip), got {}",
            game.remembered_cards.len()
        );
    }

    /// `SacrificeAll` with `!IsRemembered` should sacrifice only permanents NOT
    /// in `remembered_cards`.  This is the "SacAllOthers" step of Tragic
    /// Arrogance: `ValidCards$ Permanent.nonLand+!IsRemembered`.
    #[test]
    fn test_sacrifice_all_excludes_remembered() {
        use crate::core::TargetRestriction;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id;

        // P1 controls: a Creature (to keep) and an Artifact (to sacrifice)
        let creature = create_typed_permanent(&mut game, p1, "Serra Angel", CardType::Creature, 5);
        let artifact = create_typed_permanent(&mut game, p1, "Sol Ring", CardType::Artifact, 1);

        // Mark the creature as "remembered" (chosen to survive)
        game.remembered_cards.clear();
        game.remembered_cards.push(creature);

        // Execute SacrificeAll with !IsRemembered restriction
        let restriction = TargetRestriction::parse("Permanent.!IsRemembered");
        game.execute_sacrifice_all(&restriction)
            .expect("execute_sacrifice_all should succeed");

        // The artifact should be sacrificed (moved to graveyard); the creature should survive
        assert!(
            !game.battlefield.contains(artifact),
            "Artifact should have been sacrificed (not in remembered)"
        );
        assert!(
            game.battlefield.contains(creature),
            "Creature should survive (it was in remembered)"
        );
    }
}
