//! Action query module for GameLoop
//!
//! Provides read-only queries for available player actions (attackers, blockers, spells, abilities).
//! These functions support controller decision-making without modifying game state.

use super::GameLoop;
use crate::core::{CardId, Keyword, PlayerId};
use crate::game::phase::Step;
use smallvec::SmallVec;

impl<'a> GameLoop<'a> {
    /// Get creatures that can attack for a player (v2 interface)
    ///
    /// Results are sorted by card ID to ensure deterministic ordering for snapshot/resume.
    /// Returns SmallVec to avoid heap allocation for typical creature counts (up to 8).
    pub(super) fn get_available_attacker_creatures(&self, player_id: PlayerId) -> SmallVec<[CardId; 8]> {
        let mut creatures: SmallVec<[CardId; 8]> = SmallVec::new();

        for &card_id in &self.game.battlefield.cards {
            // Using try_get() to avoid Result drop overhead in hot path
            if let Some(card) = self.game.cards.try_get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_attacking(card_id)
                {
                    // Check for summoning sickness
                    // Creatures can't attack the turn they entered unless they have haste
                    let has_summoning_sickness = if let Some(entered_turn) = card.turn_entered_battlefield {
                        entered_turn == self.game.turn.turn_number && !card.has_keyword(Keyword::Haste)
                    } else {
                        false
                    };

                    // Check for defender keyword
                    let has_defender = card.has_defender();

                    if !has_summoning_sickness && !has_defender {
                        creatures.push(card_id);
                    }
                }
            }
        }

        // Sort for deterministic ordering
        creatures.sort();
        creatures
    }

    /// Get creatures that can block for a player (v2 interface)
    ///
    /// Results are sorted by card ID to ensure deterministic ordering for snapshot/resume.
    /// Returns SmallVec to avoid heap allocation for typical creature counts (up to 8).
    pub(super) fn get_available_blocker_creatures(&self, player_id: PlayerId) -> SmallVec<[CardId; 8]> {
        let mut creatures: SmallVec<[CardId; 8]> = SmallVec::new();

        for &card_id in &self.game.battlefield.cards {
            // Using try_get() to avoid Result drop overhead in hot path
            if let Some(card) = self.game.cards.try_get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_blocking(card_id)
                {
                    creatures.push(card_id);
                }
            }
        }

        // Sort for deterministic ordering
        creatures.sort();
        creatures
    }

    /// Get currently attacking creatures (v2 interface)
    /// Returns SmallVec to avoid heap allocation for typical attacker counts (up to 8)
    pub(super) fn get_current_attackers(&self) -> SmallVec<[CardId; 8]> {
        self.game.combat.get_attackers()
    }

    /// Get lands in player's hand (v2 interface)
    ///
    /// Returns an iterator over land CardIds in the player's hand.
    /// Zero allocation - filters the hand directly.
    ///
    /// Note: This is a static method taking `&GameState` to allow separate
    /// borrows when the caller needs to mutate other fields (like abilities_buffer).
    fn lands_in_hand_iter<'g>(
        game: &'g crate::game::GameState,
        player_id: PlayerId,
    ) -> impl Iterator<Item = CardId> + 'g {
        let cards = &game.cards;
        game.get_player_zones(player_id)
            .into_iter()
            .flat_map(|zones| zones.hand.cards.iter().copied())
            .filter(move |&card_id| cards.try_get(card_id).is_some_and(|card| card.is_land()))
    }

    /// Push castable spells directly to abilities_buffer
    ///
    /// Zero allocation - pushes SpellAbility::CastSpell directly to the buffer
    /// instead of building an intermediate Vec.
    fn push_castable_spells(&mut self, player_id: PlayerId) {
        use crate::core::SpellAbility;

        // Update the mana engine for this player
        self.mana_engine.update_mut(self.game, player_id);

        // Get the player's mana pool for checking floating mana (from Dark Ritual, etc.)
        let mana_pool = self.game.get_player(player_id).map(|p| p.mana_pool).unwrap_or_default();

        // Check if this is the active player (only active player can cast sorceries)
        let is_active_player = self.game.turn.active_player == player_id;

        // Check if it's sorcery speed (Main1 or Main2)
        let is_sorcery_speed = self.game.turn.current_step.is_sorcery_speed();

        // Check if stack is empty (required for sorcery-speed spells)
        // MTG Rules 307.5: Sorceries and creatures can only be cast when stack is empty
        let stack_is_empty = self.game.stack.is_empty();

        if let Some(zones) = self.game.get_player_zones(player_id) {
            for &card_id in &zones.hand.cards {
                // Using try_get() to avoid Result drop overhead in hot path
                if let Some(card) = self.game.cards.try_get(card_id) {
                    // Check if card is castable (not a land)
                    if !card.is_land() {
                        // Check timing restrictions
                        let can_cast_now = if card.is_instant() {
                            // Instants can be cast anytime with priority
                            true
                        } else {
                            // Creatures and sorceries require:
                            // - Sorcery speed (Main1 or Main2)
                            // - Active player
                            // - Stack is empty
                            is_sorcery_speed && is_active_player && stack_is_empty
                        };

                        if can_cast_now {
                            // Check if we can pay for this spell's mana cost
                            // Use can_pay_with_pool to consider floating mana from rituals like Dark Ritual
                            if self.mana_engine.can_pay_with_pool(&card.mana_cost, &mana_pool) {
                                // For Aura spells, check if there are valid targets
                                // MTG Rule 303.4a: You can only cast an Aura spell if there's a legal object or player it could enchant
                                if card.is_aura() {
                                    // Check if there are valid enchantment targets on the battlefield
                                    let has_valid_targets = self.game.battlefield.cards.iter().any(|&target_id| {
                                        // Using try_get() to avoid Result drop overhead in hot path
                                        self.game.cards.try_get(target_id).is_some_and(|target_card| {
                                            // Paralyze enchants creatures, so check for creatures
                                            // TODO: Parse enchant restrictions from card data (e.g., "Enchant creature")
                                            // For now, assume Auras enchant creatures
                                            target_card.is_creature()
                                        })
                                    });

                                    if has_valid_targets {
                                        self.abilities_buffer.push(SpellAbility::CastSpell { card_id });
                                    }
                                } else if Self::spell_requires_stack_target(card) {
                                    // For counterspells and similar effects, check if stack has valid targets
                                    // MTG Rule 608.2b: If a spell/ability targets, it's countered if all targets are illegal
                                    if !self.game.stack.is_empty() {
                                        self.abilities_buffer.push(SpellAbility::CastSpell { card_id });
                                    }
                                } else {
                                    self.abilities_buffer.push(SpellAbility::CastSpell { card_id });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Push activatable abilities directly to abilities_buffer
    ///
    /// Zero allocation - pushes SpellAbility::ActivateAbility directly to the buffer
    /// instead of building an intermediate Vec.
    fn push_activatable_abilities(&mut self, player_id: PlayerId) {
        use crate::core::SpellAbility;

        // Update the mana engine for this player
        self.mana_engine.update_mut(self.game, player_id);

        // Get the player's mana pool for checking floating mana (from Dark Ritual, etc.)
        let mana_pool = self.game.get_player(player_id).map(|p| p.mana_pool).unwrap_or_default();

        // Check all permanents controlled by this player
        for &card_id in &self.game.battlefield.cards {
            // Using try_get() to avoid Result drop overhead in hot path
            if let Some(card) = self.game.cards.try_get(card_id) {
                // Only check permanents controlled by this player
                if card.controller != player_id {
                    continue;
                }

                // Check each activated ability on this card
                for (ability_index, ability) in card.activated_abilities.iter().enumerate() {
                    // Skip mana abilities for now (they'll be handled specially)
                    if ability.is_mana_ability {
                        continue;
                    }

                    // Check if cost can be paid
                    let mut can_activate = true;

                    // Check tap cost
                    if ability.cost.includes_tap() && card.tapped {
                        can_activate = false;
                    }

                    // Check mana cost - use can_pay_with_pool to consider floating mana
                    if let Some(mana_cost) = ability.cost.get_mana_cost() {
                        if !self.mana_engine.can_pay_with_pool(mana_cost, &mana_pool) {
                            can_activate = false;
                        }
                    }

                    // Check life cost
                    if let Some(life_cost) = ability.cost.get_life_cost() {
                        if let Ok(player) = self.game.get_player(player_id) {
                            if player.life <= life_cost {
                                // Can't pay life cost (would go to 0 or below)
                                can_activate = false;
                            }
                        } else {
                            can_activate = false;
                        }
                    }

                    // TODO: Check other cost types (sacrifice, discard, etc.)
                    // TODO: Check activation limits

                    // Check sorcery-speed timing restrictions (CR 602.5d, CR 307.5)
                    // Sorcery-speed abilities require: main phase, your turn, stack empty
                    if ability.sorcery_speed {
                        let is_main_phase = self.game.turn.current_step.is_sorcery_speed();
                        let is_your_turn = card.controller == self.game.turn.active_player;
                        let stack_empty = self.game.stack.is_empty();

                        if !is_main_phase || !is_your_turn || !stack_empty {
                            can_activate = false;
                        }
                    }

                    // TODO(mtg-70): Check if ability has valid targets
                    // For targeting abilities, check that there's at least one valid target
                    //
                    // OPT: Check requires_target FIRST to skip expensive get_valid_targets_for_ability
                    // call for non-targeting abilities (firebreathing, regeneration, etc.)
                    if can_activate {
                        // Use cached value to avoid allocation
                        let requires_targets = ability.cache.requires_target;

                        if requires_targets {
                            // Only call get_valid_targets_for_ability for targeting abilities
                            let valid_targets = self
                                .game
                                .get_valid_targets_for_ability(card_id, ability_index)
                                .unwrap_or_else(|_| SmallVec::new());

                            if valid_targets.is_empty() {
                                // Ability requires targets but none are available
                                can_activate = false;
                            }
                        }
                        // Non-targeting abilities (requires_targets = false) skip the expensive check
                    }

                    if can_activate {
                        self.abilities_buffer
                            .push(SpellAbility::ActivateAbility { card_id, ability_index });
                    }
                }
            }
        }
    }

    /// Get all available spell abilities for a player
    ///
    /// This matches Java Forge's approach where lands, spells, and activated
    /// abilities are all represented as SpellAbility objects that can be
    /// chosen from a unified list.
    ///
    /// Returns a list of all abilities the player can currently play:
    /// - Land plays (if player can play lands and it's a main phase)
    /// - Castable spells (if player has mana and targeting is valid)
    /// - Activated abilities (TODO: not yet implemented)
    ///
    /// IMPORTANT: Results are sorted by card ID to ensure deterministic ordering.
    /// This is critical for snapshot/resume determinism where choice indices
    /// must map to the same logical cards across runs.
    /// Get available spell abilities for a player
    ///
    /// Returns a borrowed slice of the internal buffer. The buffer is reused across calls
    /// to avoid repeated heap allocations. Callers should not hold the reference across
    /// game state mutations.
    ///
    /// **Optimization note**: Previously returned `Vec<SpellAbility>` via `mem::take`, which
    /// required a new allocation for each call. Now returns `&[SpellAbility]` and reuses
    /// the buffer, eliminating ~2.5% of total allocations per DHAT profiling.
    pub(super) fn get_available_spell_abilities(&mut self, player_id: PlayerId) -> &[crate::core::SpellAbility] {
        use crate::core::SpellAbility;

        // Clear and reuse the buffer (retains capacity for next call)
        self.abilities_buffer.clear();

        // Check if stack is empty (required for sorcery-speed actions)
        let stack_is_empty = self.game.stack.is_empty();

        // Add playable lands (only in main phases when player can play lands AND stack is empty)
        // MTG Rules 307.4: Can only play land when stack is empty and you have priority during your main phase
        if stack_is_empty
            && matches!(self.game.turn.current_step, Step::Main1 | Step::Main2)
            && self.game.turn.active_player == player_id
        {
            if let Ok(player) = self.game.get_player(player_id) {
                if player.can_play_land() {
                    for land_id in Self::lands_in_hand_iter(self.game, player_id) {
                        self.abilities_buffer.push(SpellAbility::PlayLand { card_id: land_id });
                    }
                }
            }
        }

        // Add castable spells (pushes directly to abilities_buffer)
        self.push_castable_spells(player_id);

        // Add activated abilities (pushes directly to abilities_buffer)
        self.push_activatable_abilities(player_id);

        // Sort by card ID to ensure deterministic ordering
        // This is critical for snapshot/resume: if two runs have the same cards available
        // but in different hand order, we need to present them in the same order so that
        // index-based choice replay (FixedScriptController) selects the same logical card
        self.abilities_buffer.sort_by_key(|ability| match ability {
            SpellAbility::PlayLand { card_id } => *card_id,
            SpellAbility::CastSpell { card_id } => *card_id,
            SpellAbility::ActivateAbility { card_id, .. } => *card_id,
        });

        // Return a borrowed slice - buffer is reused across calls
        &self.abilities_buffer
    }

    /// Check if a spell requires a target on the stack (e.g., Counterspell)
    ///
    /// Returns true if the spell has effects that target spells on the stack,
    /// meaning it can only be cast when there's a spell to target.
    fn spell_requires_stack_target(card: &crate::core::Card) -> bool {
        use crate::core::Effect;

        // Check if any effect is CounterSpell with a placeholder target
        // Placeholder target (CardId(0)) means the spell needs to choose a target when cast
        card.effects
            .iter()
            .any(|effect| matches!(effect, Effect::CounterSpell { target } if target.as_u32() == 0))
    }
}
