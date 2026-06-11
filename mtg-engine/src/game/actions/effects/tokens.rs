//! Token-creation and copy effect-family handlers extracted from the
//! `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that mint tokens or copy stack/permanent objects
//! (CR 111, CR 707):
//! - [`Effect::CreateToken`] — instantiate N tokens from a token script
//!   (CR 111.2),
//! - [`Effect::CopyPermanent`] — make token copies of a permanent (CR 707.2),
//! - [`Effect::CopySpellAbility`] — copy the resolving spell onto the stack
//!   (CR 707.10, Chain Lightning's Parent path).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim,
//! including the undo-log entity-mint bookkeeping (mtg-ba6uq #3) and the
//! shadow-game token dedup that keeps server/client `next_entity_id` in lockstep.

use crate::core::{CardId, PlayerId};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::CreateToken`]: create `amount` tokens from `token_script` under
    /// `controller` (or under every player when `for_each_player`). CR 111.2:
    /// the creator is owner and controller. Each mint is undo-logged
    /// (mtg-ba6uq #3) and revealed to all players for network determinism.
    pub(in crate::game::actions) fn execute_create_token(
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

        if let Some(token_def) = token_def {
            // Determine which players get tokens
            let player_ids: Vec<PlayerId> = if for_each_player {
                // Each player creates tokens (TokenOwner$ Player)
                self.players.iter().map(|p| p.id).collect()
            } else {
                // Only the specified controller
                vec![controller]
            };

            for player_id in player_ids {
                // Use actual token definition
                for _ in 0..amount {
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
        add_types: &[String],
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
}
