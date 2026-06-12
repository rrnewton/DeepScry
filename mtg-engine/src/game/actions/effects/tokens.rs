//! Token-creation, emblem-creation, and copy effect-family handlers extracted
//! from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that mint tokens, emblems, or copy stack/permanent objects
//! (CR 111, CR 113, CR 707):
//! - [`Effect::CreateToken`] — instantiate N tokens from a token script (CR 111.2),
//! - [`Effect::CreateEmblem`] — mint a synthetic Card in the command zone (CR 113.2),
//! - [`Effect::CopyPermanent`] — make token copies of a permanent (CR 707.2),
//! - [`Effect::CopySpellAbility`] — copy the resolving spell onto the stack
//!   (CR 707.10, Chain Lightning's Parent path).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim,
//! including the undo-log entity-mint bookkeeping (mtg-ba6uq #3) and the
//! shadow-game token dedup that keeps server/client `next_entity_id` in lockstep.

use crate::core::{CardId, PlayerId, StaticAbility, Trigger};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::CreateToken`]: create `amount` tokens from `token_script` under
    /// `controller` (or under every player when `for_each_player`). CR 111.2:
    /// the creator is owner and controller. Each mint is undo-logged
    /// (mtg-ba6uq #3) and revealed to all players for network determinism.
    ///
    /// `token_script` may be a comma-separated list of distinct token script
    /// names (e.g. Wurmcoil Engine: `"c_3_3_a_phyrexian_wurm_deathtouch,
    /// c_3_3_a_phyrexian_wurm_lifelink"`). When multiple names are present,
    /// one token of each distinct kind is created per repetition of `amount`.
    ///
    /// Applies `StaticAbility::TokenCreationBonus` replacement effects (CR 614):
    /// if any battlefield permanent controlled by a token recipient carries this
    /// static, additional tokens are minted for that player after the originals.
    /// The bonus minting is non-recursive (CR 614.5d: a replacement effect cannot
    /// apply to itself).
    pub(in crate::game::actions) fn execute_create_token(
        &mut self,
        controller: PlayerId,
        token_script: &str,
        amount: u8,
        for_each_player: bool,
    ) -> Result<()> {
        // TokenScript$ may name multiple distinct tokens via comma separation
        // (CR 111.2 — each listed token is minted independently). Split and
        // delegate so every script name goes through the single-script path.
        let scripts: Vec<&str> = token_script.split(',').map(str::trim).collect();
        if scripts.len() > 1 {
            for script in scripts {
                self.execute_create_token_single(controller, script, amount, for_each_player)?;
            }
            return Ok(());
        }
        self.execute_create_token_single(controller, token_script, amount, for_each_player)
    }

    /// Inner helper: create `amount` tokens from a single (non-comma) `token_script`.
    fn execute_create_token_single(
        &mut self,
        controller: PlayerId,
        token_script: &str,
        amount: u8,
        for_each_player: bool,
    ) -> Result<()> {
        // Create token(s) on the battlefield
        // MTG Rules 111.2: The player who creates a token is its owner and controller

        // Look up token definition from cache (loaded during game initialization)
        // For native builds, tokens are loaded from tokenscripts/ directory.
        // For WASM builds, tokens are bundled with deck data.
        let token_def = self.token_definitions.get(token_script).cloned();

        // Determine which players get tokens.
        let player_ids: Vec<PlayerId> = if for_each_player {
            self.players.iter().map(|p| p.id).collect()
        } else {
            vec![controller]
        };

        // Collect TokenCreationBonus statics for each receiving player (read-only
        // scan before mutations so there's no borrow conflict). Each tuple is
        // (recipient, bonus_script, bonus_amount). CR 614.5d: we don't
        // recurse — the bonus minting itself does NOT re-trigger this logic.
        let bonuses: Vec<(PlayerId, String, u8)> = {
            use crate::core::StaticAbility;
            let mut result = Vec::new();
            for &card_id in &self.battlefield.cards {
                if let Ok(card) = self.cards.get(card_id) {
                    let ctrl = card.controller;
                    if !player_ids.contains(&ctrl) {
                        continue;
                    }
                    for sa in &card.static_abilities {
                        if let StaticAbility::TokenCreationBonus {
                            token_script: bonus_script,
                            amount: bonus_amt,
                            ..
                        } = sa
                        {
                            result.push((ctrl, bonus_script.clone(), *bonus_amt));
                        }
                    }
                }
            }
            result
        };

        if let Some(token_def) = token_def {
            for &player_id in &player_ids {
                for _ in 0..amount {
                    self.mint_single_token(&token_def, player_id)?;
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

        // Apply TokenCreationBonus replacements (non-recursive, CR 614.5d).
        // One bonus event fires per (player, source) pair regardless of how many
        // original tokens were created (the bonus is a flat additional amount,
        // not per-original-token).
        for (player_id, bonus_script, bonus_amount) in &bonuses {
            let bonus_def = self.token_definitions.get(bonus_script.as_str()).cloned();
            if let Some(def) = bonus_def {
                for _ in 0..*bonus_amount {
                    self.mint_single_token(&def, *player_id)?;
                }
            } else {
                log::warn!(
                    "TokenCreationBonus: token definition '{}' not found — skipping",
                    bonus_script
                );
            }
        }

        Ok(())
    }

    /// Instantiate a single token from `def` under `player_id`'s control and
    /// place it on the battlefield. Handles undo-log, network reveal, and the
    /// shadow-game dedup (pre-added `CardRevealed(TokenCreated)` tokens).
    ///
    /// This is the shared mint primitive used by both the original token
    /// creation and the `TokenCreationBonus` replacement path (non-recursive).
    fn mint_single_token(&mut self, def: &crate::loader::CardDefinition, player_id: PlayerId) -> Result<()> {
        let token_id = self.next_card_id();

        // Shadow game dedup: in shadow games, tokens for opponent actions are
        // pre-added via CardRevealed(TokenCreated) before this effect runs.
        // CardRevealed uses insert_if_vacant (doesn't advance next_entity_id),
        // so next_card_id() here returns the SAME id that was pre-added.
        // We must skip to avoid the EntityStore write-once panic.
        // For locally-created tokens (own actions in native shadow game),
        // cards.contains() is false so we proceed normally.
        if self.is_shadow_game && self.cards.contains(token_id) {
            if !self.battlefield.contains(token_id) {
                self.battlefield.add(token_id);
            }
            return Ok(());
        }

        let mut token = def.instantiate(token_id, player_id);
        token.is_token = true;
        token.controller = player_id;

        let token_name = token.name.to_string();
        self.cards.insert(token_id, token);

        // Log the entity mint so a rewind can remove the token AND roll
        // `next_entity_id` back (mtg-ba6uq #3). Logged BEFORE the
        // reveal/battlefield placement so the LIFO undo reverses those first.
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

        self.battlefield.add(token_id);

        log::debug!(
            target: "token",
            "Created token {} (id={}) under player {}'s control",
            token_name,
            token_id.as_u32(),
            player_id.as_u32()
        );

        self.logger.gamelog(&format!(
            "Created {} under {}'s control",
            token_name,
            self.get_player(player_id)?.name
        ));

        Ok(())
    }

    /// [`Effect::CreateTokenWithStoredPt`]: create one token whose power/toughness
    /// equals `source_card.stored_int` (Phyrexian Processor: life paid on ETB).
    ///
    /// The token script (`b_x_x_phyrexian_minion`) has `PT:*/*`; we override its
    /// base power and toughness from the stored amount. If `stored_int` is `None`
    /// (shouldn't happen in normal play), the token is created as a 0/0 — matching
    /// the pre-fix behaviour so the engine doesn't crash.
    pub(in crate::game::actions) fn execute_create_token_with_stored_pt(
        &mut self,
        source_card: crate::core::CardId,
        controller: crate::core::PlayerId,
        token_script: &str,
    ) -> Result<()> {
        // Read the stored amount from the source card (Phyrexian Processor).
        let stored = self.cards.try_get(source_card).and_then(|c| c.stored_int);
        let pt = stored.unwrap_or(0) as i8;
        if stored.is_none() {
            log::warn!(
                target: "token",
                "CreateTokenWithStoredPt: source card {:?} has no stored_int; creating 0/0 token",
                source_card
            );
        }

        let token_def = self.token_definitions.get(token_script).cloned();
        if let Some(token_def) = token_def {
            let token_id = self.next_card_id();

            if self.is_shadow_game && self.cards.contains(token_id) {
                if !self.battlefield.contains(token_id) {
                    self.battlefield.add(token_id);
                }
                return Ok(());
            }

            let mut token = token_def.instantiate(token_id, controller);
            token.is_token = true;
            token.controller = controller;
            // Override the printed `*/*` with the life-paid amount (CR 208.2a
            // defines base P/T; layer 7b CDAs don't apply to tokens, so the
            // stored value goes in as the printed base stat).
            token.set_base_power(Some(pt));
            token.set_base_toughness(Some(pt));

            let token_name = token.name.to_string();
            self.cards.insert(token_id, token);

            let mint_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::CreateEntity { card_id: token_id },
                mint_log_size,
            );

            let prior_log_size = self.logger.log_count();
            self.maybe_reveal_to_all(token_id, prior_log_size);
            self.battlefield.add(token_id);

            log::debug!(target: "token",
                "Created {}/{} {} token (id={}) under player {}'s control from stored_int={}",
                pt, pt, token_name, token_id.as_u32(), controller.as_u32(), stored.unwrap_or(0)
            );
            self.logger.gamelog(&format!(
                "Created {}/{} {} under {}'s control",
                pt,
                pt,
                token_name,
                self.get_player(controller)?.name
            ));
        } else {
            log::warn!(
                "Token definition not found: '{}' - skipping CreateTokenWithStoredPt",
                token_script
            );
        }
        Ok(())
    }

    /// [`Effect::CopyPermanent`]: create `num_copies` token copies of `target`
    /// under `controller`, applying optional set-power/set-toughness/add-types
    /// modifications (CR 707.2). Fizzles if the target left the battlefield.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::actions) fn execute_copy_permanent(
        &mut self,
        target: CardId,
        controller: PlayerId,
        set_power: Option<i32>,
        set_toughness: Option<i32>,
        add_subtypes: &[crate::core::Subtype],
        num_copies: u8,
    ) -> Result<()> {
        // Create token copies of the target permanent
        // MTG Rules 707.2: A copy of a permanent has the same characteristics
        // as the original, except for any modifications specified

        // Verify target is still on battlefield
        if !self.battlefield.contains(target) {
            // Target was removed - spell fizzles
            log::debug!(target: "actions", "CopyPermanent target no longer on battlefield");
            return Ok(());
        }

        let original = self.cards.get(target)?;
        let original_name = original.name.clone();
        let original_base_power = original.base_power();
        let original_base_toughness = original.base_toughness();

        for _ in 0..num_copies {
            let token_id = self.next_card_id();

            // Clone the original card to get all characteristics
            let original = self.cards.get(target)?;
            let mut token = original.clone();

            // Update identity for the new token
            token.id = token_id;
            token.owner = controller;
            token.controller = controller;

            // Reset state for new permanent
            token.tapped = false;
            token.turn_entered_battlefield = None; // Will be set when it enters battlefield
            token.counters.clear();
            token.damage = 0;
            token.attached_to = None;
            // CR 111.9: a copy created as a token IS a token permanent.
            token.is_token = true;

            // Apply modifications

            // SetPower$ N - override power
            if let Some(power) = set_power {
                // Power is i8 in Card but i32 in Effect, clamp to i8 range
                token.set_base_power(Some(power as i8));
            }

            // SetToughness$ N - override toughness
            if let Some(toughness) = set_toughness {
                token.set_base_toughness(Some(toughness as i8));
            }

            // AddTypes$ Type1 & Type2 - add creature/permanent subtypes (CR 205.3)
            for subtype in add_subtypes {
                if !token.subtypes.contains(subtype) {
                    token.subtypes.push(subtype.clone());
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
            let modification_desc = if set_power.is_some() || set_toughness.is_some() || !add_subtypes.is_empty() {
                let p = set_power.map(|x| x as i8).or(original_base_power).unwrap_or(0);
                let t = set_toughness.map(|x| x as i8).or(original_base_toughness).unwrap_or(0);
                let types_str = if add_subtypes.is_empty() {
                    String::new()
                } else {
                    let names: Vec<&str> = add_subtypes.iter().map(|s| s.as_str()).collect();
                    format!(" {}", names.join(" "))
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
                self.get_player(controller)?.name
            ));
        }
        Ok(())
    }

    /// [`Effect::CopySpellAbility`]: copy a spell/ability onto the stack
    /// (CR 707.10). Only the `Parent` self-copy path (Chain Lightning) is
    /// implemented here; `TargetedSpell` (Twincast/Fork) and
    /// `TriggeredSpellAbility` (delayed-trigger path) are safe no-ops so the
    /// dispatcher never falls through to an infinite self-copy loop.
    pub(in crate::game::actions) fn execute_copy_spell_ability(
        &mut self,
        may_choose_targets: bool,
        defined_source: &crate::core::effects::CopySpellSource,
        controller: Option<&str>,
    ) -> Result<()> {
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
                let new_targets = if may_choose_targets && copy_controller != original_owner {
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
        Ok(())
    }

    /// [`Effect::CreateEmblem`]: mint a synthetic Card in `controller`'s command
    /// zone and populate it with the given static abilities and/or triggers.
    ///
    /// CR 113.2: Emblems are objects with abilities that are placed in the command
    /// zone.  They have no owner or controller in the rules (CR 113.4), but we
    /// track the creating player as both owner and controller for scoping purposes
    /// ("creatures YOU control", etc.).
    ///
    /// The command zone is already scanned by:
    /// - `continuous_effects.rs::calculate_modifypt_effects` / `calculate_granted_keywords`
    ///   (for static ModifyPT and GrantKeyword abilities), and
    /// - `steps.rs::fire_phase_triggers` (for phase triggers with TriggerZones$ Command).
    ///
    /// So once the emblem card is placed in the command zone, the existing machinery
    /// handles it with no further special-casing.
    ///
    /// Undo-log note: emblems created by ultimate abilities are permanent — they last
    /// for the rest of the game. We still undo-log the entity creation so that
    /// snapshot/resume and network-rewind can reconstruct them correctly.
    pub(in crate::game::actions) fn execute_create_emblem(
        &mut self,
        controller: PlayerId,
        emblem_name: &str,
        static_abilities: &[StaticAbility],
        triggers: &[Trigger],
    ) -> Result<()> {
        use crate::core::CardName;

        let emblem_id = self.next_card_id();

        // In shadow games, emblems for opponent actions are pre-added via
        // CardRevealed before this effect runs (same pattern as tokens).
        if self.is_shadow_game && self.cards.contains(emblem_id) {
            let already_in_command = self
                .get_player_zones(controller)
                .is_some_and(|z| z.command.cards.contains(&emblem_id));
            if !already_in_command {
                if let Some(zones) = self.get_player_zones_mut(controller) {
                    zones.command.add(emblem_id);
                }
            }
            return Ok(());
        }

        // Mint a minimal synthetic Card for the emblem
        let mut emblem_card = crate::core::Card::new(emblem_id, CardName::from(emblem_name), controller);
        emblem_card.controller = controller;
        emblem_card.static_abilities = static_abilities.to_vec();
        emblem_card.triggers = triggers.to_vec();
        // Mark as non-token so it persists through zone-change cleanup
        emblem_card.is_token = false;

        let name_for_log = emblem_card.name.to_string();
        let controller_name = self
            .players
            .iter()
            .find(|p| p.id == controller)
            .map(|p| p.name.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Log the entity mint so rewind can remove it
        let mint_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::CreateEntity { card_id: emblem_id },
            mint_log_size,
        );

        // Reveal to all players for network determinism
        let prior_log_size = self.logger.log_count();
        self.cards.insert(emblem_id, emblem_card);
        self.maybe_reveal_to_all(emblem_id, prior_log_size);

        // Place in the controller's command zone
        if let Some(zones) = self.get_player_zones_mut(controller) {
            zones.command.add(emblem_id);
        }

        log::debug!(target: "emblem",
            "Created emblem '{}' (id={}) for player {}",
            name_for_log, emblem_id.as_u32(), controller.as_u32()
        );
        self.logger
            .gamelog(&format!("{} gets an emblem: {}", controller_name, name_for_log));

        Ok(())
    }
}
