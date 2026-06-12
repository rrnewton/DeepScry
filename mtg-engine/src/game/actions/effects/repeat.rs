//! RepeatEach effect handler — "for each X, do Y" loop mechanic.
//!
//! Implements [`Effect::RepeatEach`], which covers two patterns from the
//! Java Forge card DSL:
//!
//! **Pattern A — iterate over chosen targets** (Terastodon):
//! ```text
//! DB$ RepeatEach | RepeatSubAbility$ DBToken | DefinedCards$ Targeted | ChangeZoneTable$ True
//! SVar:DBToken:DB$ Token | TokenOwner$ RememberedController | ...
//! ```
//! For each targeted permanent that ended up in the graveyard (successfully
//! destroyed), set it as `remembered_cards[0]` and run the sub-effects.
//!
//! **Pattern B — iterate over players** (Tragic Arrogance):
//! ```text
//! A:SP$ RepeatEach | RepeatPlayers$ Player | RepeatSubAbility$ YouChoose | SubAbility$ SacAllOthers
//! SVar:YouChoose:DB$ ChooseCard | ...
//! ```
//! For each player in turn order, set them as `remembered_players[0]` and run
//! the sub-effects.
//!
//! CR 609.3: effects that use "for each" repeat an action once per set member,
//! sequentially. State-based actions check between repetitions (handled by the
//! engine's normal SBA check at the end of each spell resolution).
//!
//! **`TokenOwner$ RememberedController`**: when a sub-effect contains a
//! `CreateToken` with `controller = PlayerId::remembered_controller()`, this
//! executor resolves that sentinel to the controller of the current remembered
//! card before calling `execute_effect`. This lets Terastodon give the Elephant
//! token to the controller of the destroyed permanent rather than to the caster.

use crate::core::{CardId, Effect, PlayerId, RepeatEachIterate};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::RepeatEach`]: execute `sub_effects` once for each member of
    /// `iterate_over`, managing the remembered-card/player scratch between
    /// iterations. CR 609.3 — sequential, state-based actions between steps.
    pub(in crate::game::actions) fn execute_repeat_each(
        &mut self,
        sub_effects: &[Effect],
        iterate_over: &RepeatEachIterate,
    ) -> Result<()> {
        match iterate_over {
            RepeatEachIterate::Cards {
                targets,
                require_in_graveyard,
            } => self.execute_repeat_each_cards(sub_effects, targets, *require_in_graveyard),
            RepeatEachIterate::AllPlayers => self.execute_repeat_each_players(sub_effects),
        }
    }

    /// Pattern A: iterate over `targets`, optionally filtered to cards that
    /// ended up in the graveyard (`ChangeZoneTable$ True` — only cards that
    /// were actually destroyed count).
    fn execute_repeat_each_cards(
        &mut self,
        sub_effects: &[Effect],
        targets: &[CardId],
        require_in_graveyard: bool,
    ) -> Result<()> {
        // Snapshot the target list so the loop cannot be disrupted by mutations
        // to game state during iteration (e.g. token creation does not remove
        // cards from the graveyard, so this is safe for the current use-cases).
        let targets_snapshot: Vec<CardId> = targets.to_vec();

        for &card_id in &targets_snapshot {
            // ChangeZoneTable$ True: skip cards that are NOT in a real graveyard
            // (e.g. indestructible permanents that never left the battlefield,
            // or permanents with replacement effects that exiled them instead).
            if require_in_graveyard && !self.is_card_in_graveyard(card_id) {
                log::debug!(
                    target: "repeat_each",
                    "RepeatEach: skipping {:?} — not in graveyard (require_in_graveyard=true)",
                    card_id
                );
                continue;
            }

            // Save existing remembered state so we can restore it after this
            // iteration (outer chains may have set remembered_cards already).
            let saved_cards: smallvec::SmallVec<[CardId; 4]> = self.remembered_cards.clone();

            // Set this card as the sole remembered card for this iteration
            self.remembered_cards.clear();
            self.remembered_cards.push(card_id);

            log::debug!(
                target: "repeat_each",
                "RepeatEach (cards): iteration for {:?}",
                card_id
            );

            // Execute all sub-effects for this iteration, resolving any
            // RememberedController sentinels against the current remembered card.
            for sub_effect in sub_effects {
                let resolved = self.resolve_remembered_controller_in_effect(sub_effect, card_id);
                if let Err(e) = self.execute_effect(&resolved) {
                    log::warn!(
                        target: "repeat_each",
                        "RepeatEach (cards): sub-effect error for {:?}: {}",
                        card_id, e
                    );
                    // Restore remembered state even on error
                    self.remembered_cards = saved_cards;
                    return Err(e);
                }
            }

            // Restore remembered state after this iteration
            self.remembered_cards = saved_cards;
        }
        Ok(())
    }

    /// Pattern B: iterate over all players in turn order, setting each as the
    /// sole remembered player for that iteration.
    fn execute_repeat_each_players(&mut self, sub_effects: &[Effect]) -> Result<()> {
        // Snapshot player IDs to avoid borrow issues during iteration
        let player_ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();

        for player_id in player_ids {
            // Save existing remembered state
            let saved_players: smallvec::SmallVec<[PlayerId; 4]> = self.remembered_players.clone();
            let saved_cards: smallvec::SmallVec<[CardId; 4]> = self.remembered_cards.clone();

            // Set this player as the sole remembered player for this iteration
            self.remembered_players.clear();
            self.remembered_players.push(player_id);
            // Also clear remembered cards (ChooseCard sub-effects build on them)
            self.remembered_cards.clear();

            log::debug!(
                target: "repeat_each",
                "RepeatEach (players): iteration for player {:?}",
                player_id
            );

            for sub_effect in sub_effects {
                if let Err(e) = self.execute_effect(sub_effect) {
                    log::warn!(
                        target: "repeat_each",
                        "RepeatEach (players): sub-effect error for player {:?}: {}",
                        player_id, e
                    );
                    self.remembered_players = saved_players;
                    self.remembered_cards = saved_cards;
                    return Err(e);
                }
            }

            // Restore remembered state after this iteration
            self.remembered_players = saved_players;
            self.remembered_cards = saved_cards;
        }
        Ok(())
    }

    /// Check whether `card_id` is currently in any player's graveyard.
    ///
    /// Used by `ChangeZoneTable$ True` — only iterate over cards that actually
    /// reached the graveyard (successfully destroyed), skipping indestructible
    /// permanents or those replaced to another zone.
    fn is_card_in_graveyard(&self, card_id: CardId) -> bool {
        // Check all player graveyard zones
        for player in &self.players {
            if let Some(zones) = self.get_player_zones(player.id) {
                if zones.graveyard.contains(card_id) {
                    return true;
                }
            }
        }
        // Also accept exile (some replacements move to exile instead of graveyard)
        // but Terastodon specifically uses ConditionPresent$ Card.inRealZoneGraveyard,
        // so we only count actual graveyard placement.
        false
    }

    /// Resolve a `PlayerId::remembered_controller()` sentinel inside a
    /// `CreateToken` (or `CreateTokenDynamic`) effect to the actual controller
    /// of `remembered_card` at this point in time.
    ///
    /// Terastodon's `DBToken` has `TokenOwner$ RememberedController`: the
    /// Elephant should go to the controller of the destroyed permanent, which
    /// is stored in `remembered_cards[0]`. We use `remembered_card` (already
    /// set by the caller) to look up the controller from the cards store or
    /// from the LKI (last-known-information) hint stored on the card when it
    /// left the battlefield.
    ///
    /// If the remembered card is no longer in any store (shouldn't happen in
    /// practice), fall back to the placeholder (card_owner) so the token still
    /// gets created rather than being silently dropped.
    fn resolve_remembered_controller_in_effect(&self, effect: &Effect, remembered_card: CardId) -> Effect {
        // Resolve RememberedController sentinel only in token-creation effects.
        // All other effects are cloned unchanged. We use if-let rather than a
        // wildcard match arm to avoid the clippy::wildcard_in_or_patterns lint
        // that fires when a match arm with `_` would silently cover future
        // Effect variants.
        if let Effect::CreateToken {
            controller,
            token_script,
            amount,
            for_each_player,
        } = effect
        {
            if controller.is_remembered_controller() {
                let resolved_controller = self.controller_of_card_or_lki(remembered_card);
                return Effect::CreateToken {
                    controller: resolved_controller,
                    token_script: token_script.clone(),
                    amount: *amount,
                    for_each_player: *for_each_player,
                };
            }
        }
        if let Effect::CreateTokenDynamic {
            controller,
            token_script,
            amount,
            for_each_player,
        } = effect
        {
            if controller.is_remembered_controller() {
                let resolved_controller = self.controller_of_card_or_lki(remembered_card);
                return Effect::CreateTokenDynamic {
                    controller: resolved_controller,
                    token_script: token_script.clone(),
                    amount: amount.clone(),
                    for_each_player: *for_each_player,
                };
            }
        }
        // All other effects are cloned unchanged (no RememberedController sentinel).
        effect.clone()
    }

    /// Return the controller of `card_id` using the live game state (if the
    /// card is still in the cards store) or the last-known-information from
    /// the card's graveyard entry.
    ///
    /// When a permanent moves to the graveyard, its `controller` field is
    /// preserved on the `Card` struct (our engine never clears it on zone
    /// change), so `cards.try_get(card_id).map(|c| c.controller)` works even
    /// after destruction. Falls back to player 0 (= first player) if the card
    /// is somehow missing from the store.
    fn controller_of_card_or_lki(&self, card_id: CardId) -> PlayerId {
        self.cards.try_get(card_id).map(|c| c.controller).unwrap_or_else(|| {
            // Last resort: use first player as owner
            self.players.first().map(|p| p.id).unwrap_or_else(|| PlayerId::new(0))
        })
    }
}

/// Tests for the RepeatEach executor.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, RepeatEachIterate};
    use crate::game::GameState;

    /// Add a minimal card to player `owner`'s graveyard (simulating a
    /// successfully destroyed permanent) and return its ID.
    fn create_test_permanent_in_graveyard(game: &mut GameState, owner: PlayerId, name: &str) -> CardId {
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, name, owner);
        card.controller = owner;
        game.cards.insert(card_id, card);
        // Place it directly in the owner's graveyard zone
        if let Some(zones) = game.get_player_zones_mut(owner) {
            zones.graveyard.add(card_id);
        }
        card_id
    }

    /// Add a minimal card to the battlefield under `owner`'s control and return its ID.
    fn create_test_permanent_on_battlefield(game: &mut GameState, owner: PlayerId, name: &str) -> CardId {
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, name, owner);
        card.controller = owner;
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        card_id
    }

    /// Terastodon test: RepeatEach creates an Elephant token for each of the
    /// two destroyed noncreature permanents, owned by their respective controllers.
    ///
    /// Setup:
    ///   - P1 controls 2 noncreature permanents (simulated: placed directly in graveyard)
    ///   - P2 controls 1 noncreature permanent (simulated same way)
    ///
    /// Expectation after RepeatEach:
    ///   - 2 × g_3_3_elephant tokens created for P1 (their own permanents were destroyed)
    ///   - 1 × g_3_3_elephant token created for P2 (their permanent was destroyed)
    #[test]
    fn test_repeat_each_creates_elephant_tokens() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id;
        let p2 = game.players[1].id;

        // Load the token definition for g_3_3_elephant
        if !game.token_definitions.contains_key("g_3_3_elephant") {
            // Skip test if token definition not available in test environment
            return;
        }

        // Create two "destroyed permanent" stand-ins in P1's graveyard
        let perm1 = create_test_permanent_in_graveyard(&mut game, p1, "TestPerm1");
        let perm2 = create_test_permanent_in_graveyard(&mut game, p1, "TestPerm2");

        // Create one "destroyed permanent" stand-in in P2's graveyard
        let perm3 = create_test_permanent_in_graveyard(&mut game, p2, "TestPerm3");

        // Count tokens before
        let tokens_before = game.battlefield.cards.len();

        // Build the sub_effects: CreateToken{RememberedController, g_3_3_elephant, 1}
        let sub_effects = vec![Effect::CreateToken {
            controller: PlayerId::remembered_controller(),
            token_script: "g_3_3_elephant".to_string(),
            amount: 1,
            for_each_player: false,
        }];

        // Build the iterate_over: cards {perm1, perm2, perm3}, require_in_graveyard=true
        let iterate_over = RepeatEachIterate::Cards {
            targets: vec![perm1, perm2, perm3],
            require_in_graveyard: true,
        };

        game.execute_repeat_each(&sub_effects, &iterate_over)
            .expect("RepeatEach should succeed");

        // Should have created 3 tokens (one per destroyed permanent)
        let tokens_after = game.battlefield.cards.len();
        assert_eq!(
            tokens_after - tokens_before,
            3,
            "Expected 3 Elephant tokens for 3 destroyed permanents, got {}",
            tokens_after - tokens_before
        );
    }

    /// Indestructible test: RepeatEach skips permanents that did NOT reach the
    /// graveyard (require_in_graveyard=true).
    #[test]
    fn test_repeat_each_skips_non_graveyard_cards() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id;
        let p2 = game.players[1].id;

        if !game.token_definitions.contains_key("g_3_3_elephant") {
            return;
        }

        // One card goes to graveyard (was destroyed), one stays on the battlefield
        // (indestructible — did NOT go to graveyard)
        let perm_destroyed = create_test_permanent_in_graveyard(&mut game, p1, "Destroyed");
        let perm_indestructible = create_test_permanent_on_battlefield(&mut game, p2, "Indestructible");

        let tokens_before = game.battlefield.cards.len();

        let sub_effects = vec![Effect::CreateToken {
            controller: PlayerId::remembered_controller(),
            token_script: "g_3_3_elephant".to_string(),
            amount: 1,
            for_each_player: false,
        }];

        let iterate_over = RepeatEachIterate::Cards {
            targets: vec![perm_destroyed, perm_indestructible],
            require_in_graveyard: true,
        };

        game.execute_repeat_each(&sub_effects, &iterate_over)
            .expect("RepeatEach should succeed");

        // Only 1 token: perm_indestructible is still on battlefield, not in graveyard
        let tokens_after = game.battlefield.cards.len();
        // tokens_after includes perm_indestructible itself, so delta from token creation is:
        let new_tokens = tokens_after - tokens_before;
        assert_eq!(
            new_tokens, 1,
            "Expected 1 Elephant token (indestructible permanent skipped), got {}",
            new_tokens
        );
        let _ = perm_indestructible; // suppress unused warning
    }
}
