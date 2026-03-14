//! Action query module for GameLoop
//!
//! Provides read-only queries for available player actions (attackers, blockers, spells, abilities).
//! These functions support controller decision-making without modifying game state.

use super::GameLoop;
use crate::core::{CardId, Keyword, PlayerId};
use crate::game::phase::Step;
use smallvec::SmallVec;

impl<'a> GameLoop<'a> {
    /// Validate that all cards in hand are revealed (network debug mode)
    ///
    /// Called when `debug_validate_reveals` is enabled (gated by `network_debug` flag).
    /// In network mode with hidden info, only validates the local player's cards
    /// (opponent's cards are intentionally hidden per mtg-qtqcr).
    ///
    /// This validation follows the linear transfer of control model: all reveals should
    /// have been processed before we reach a point where we need the card. If a reveal
    /// is missing, it's a protocol bug, not a timing issue.
    ///
    /// # Panics
    /// Panics immediately with detailed error if any unrevealed card is found in the
    /// local player's hand. No retries - missing reveals indicate a protocol bug.
    fn validate_cards_revealed(&self, player_id: PlayerId) {
        // In network mode, only validate the LOCAL player's cards
        // Opponent's hand cards are intentionally not revealed (hidden info architecture)
        if let Some(local_id) = self.local_player_id {
            if player_id != local_id {
                // Skip validation for opponent's turn - their cards are hidden
                return;
            }
        }

        // Check all cards in player's hand are revealed
        let hand_cards: SmallVec<[CardId; 8]> = self
            .game
            .get_player_zones(player_id)
            .map(|zones| zones.hand.cards.iter().copied().collect())
            .unwrap_or_default();

        for card_id in &hand_cards {
            if !self.game.cards.is_revealed(*card_id) {
                panic!(
                    "REVEAL VALIDATION FAILED: Card {:?} in {:?}'s hand is NOT revealed!\n\
                     This indicates a missing CardRevealed message from server.\n\
                     All reveals should be processed before player needs to act.\n\
                     Hand contents: {:?}",
                    card_id, player_id, hand_cards
                );
            }
        }

        // In non-network mode (local_player_id is None), also check public zones
        if self.local_player_id.is_none() {
            for &card_id in &self.game.battlefield.cards {
                if !self.game.cards.is_revealed(card_id) {
                    panic!(
                        "REVEAL VALIDATION FAILED: Card {:?} on battlefield is NOT revealed!\n\
                         Battlefield contents: {:?}",
                        card_id, self.game.battlefield.cards
                    );
                }
            }

            for &card_id in &self.game.stack.cards {
                if !self.game.cards.is_revealed(card_id) {
                    panic!(
                        "REVEAL VALIDATION FAILED: Card {:?} on stack is NOT revealed!\n\
                         Stack contents: {:?}",
                        card_id, self.game.stack.cards
                    );
                }
            }
        }
    }

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

    /// Calculate effective mana cost after applying cost reduction effects like Affinity.
    ///
    /// Affinity for X reduces generic mana cost by 1 for each permanent of type X you control.
    /// Example: "Affinity for Allies" on a 2G spell with 3 Allies in play = G (0 generic + G)
    ///
    /// # Parameters
    /// - `card`: The card being cast
    /// - `player_id`: The player casting the spell
    ///
    /// # Returns
    /// The effective mana cost after applying all cost reductions
    fn calculate_effective_cost(&self, card: &crate::core::Card, player_id: PlayerId) -> crate::core::ManaCost {
        use crate::core::{CostReductionTarget, KeywordArgs, StaticAbility};

        let mut effective_cost = card.mana_cost;

        // Check for Affinity keyword
        // Affinity for X: This spell costs {1} less for each X you control
        if let Some(KeywordArgs::Affinity { card_type }) = card.keywords.get_args(Keyword::Affinity) {
            // Count permanents of the specified type controlled by the player
            let count = self
                .game
                .battlefield
                .cards
                .iter()
                .filter(|&&card_id| {
                    self.game
                        .cards
                        .try_get(card_id)
                        .is_some_and(|c| c.controller == player_id && c.subtypes.contains(card_type))
                })
                .count() as u8;

            // Reduce generic cost (minimum 0)
            effective_cost.generic = effective_cost.generic.saturating_sub(count);

            if count > 0 {
                log::debug!(
                    "Affinity for {:?}: {} permanents controlled, reducing generic cost by {} (was {}, now {})",
                    card_type,
                    count,
                    count,
                    card.mana_cost.generic,
                    effective_cost.generic
                );
            }
        }

        // Check for ReduceCost static abilities from controlled permanents
        // Example: Gran-Gran reduces non-creature spell costs by {1} with enough Lessons in graveyard
        for &bf_card_id in &self.game.battlefield.cards {
            let Some(source_card) = self.game.cards.try_get(bf_card_id) else {
                continue;
            };

            // Only consider permanents controlled by the player casting the spell
            if source_card.controller != player_id {
                continue;
            }

            for static_ability in &source_card.static_abilities {
                if let StaticAbility::ReduceCost {
                    valid_card,
                    amount,
                    condition,
                    description,
                } = static_ability
                {
                    // Check if the spell being cast matches the valid_card filter
                    let spell_matches = match valid_card {
                        CostReductionTarget::AllSpells => true,
                        CostReductionTarget::NonCreature => !card.is_creature(),
                        CostReductionTarget::Creature => card.is_creature(),
                        CostReductionTarget::Subtype(subtype) => card.subtypes.contains(subtype),
                    };

                    if !spell_matches {
                        continue;
                    }

                    // Check if the condition is met (if any)
                    let condition_met = if let Some(cond) = condition {
                        // Count cards matching is_present filter in the specified zone
                        self.count_cards_matching_filter(player_id, &cond.is_present, cond.present_zone)
                            >= cond.min_count as usize
                    } else {
                        true // No condition means always active
                    };

                    if condition_met {
                        let old_generic = effective_cost.generic;
                        effective_cost.generic = effective_cost.generic.saturating_sub(*amount);

                        if old_generic != effective_cost.generic {
                            log::debug!(
                                "ReduceCost from {}: {} (reducing generic by {}, was {}, now {})",
                                source_card.name,
                                description,
                                amount,
                                old_generic,
                                effective_cost.generic
                            );
                        }
                    }
                }

                // Also check for RaiseCost (mana-based cost increases)
                if let StaticAbility::RaiseCost {
                    valid_card,
                    raised_cost,
                    description,
                } = static_ability
                {
                    use crate::core::RaisedCost;

                    // Check if the spell being cast matches the valid_card filter
                    let spell_matches = match valid_card {
                        CostReductionTarget::AllSpells => true,
                        CostReductionTarget::NonCreature => !card.is_creature(),
                        CostReductionTarget::Creature => card.is_creature(),
                        CostReductionTarget::Subtype(subtype) => card.subtypes.contains(subtype),
                    };

                    if !spell_matches {
                        continue;
                    }

                    // Handle mana-based cost increase
                    if let RaisedCost::Mana(amount) = raised_cost {
                        let old_generic = effective_cost.generic;
                        effective_cost.generic = effective_cost.generic.saturating_add(*amount);

                        if old_generic != effective_cost.generic {
                            log::debug!(
                                "RaiseCost from {}: {} (increasing generic by {}, was {}, now {})",
                                source_card.name,
                                description,
                                amount,
                                old_generic,
                                effective_cost.generic
                            );
                        }
                    }
                    // Note: Sacrifice-based RaiseCost is handled separately during spell casting
                    // as it requires prompting for sacrifice choices, not just mana adjustment
                }
            }
        }

        effective_cost
    }

    /// Count cards matching a filter string in a specified zone
    ///
    /// Used for checking ReduceCost conditions like "IsPresent$ Lesson.YouOwn | PresentZone$ Graveyard"
    fn count_cards_matching_filter(&self, player_id: PlayerId, filter: &str, zone: crate::zones::Zone) -> usize {
        use crate::zones::Zone;

        // Parse filter: "Lesson.YouOwn" -> type="Lesson", ownership="YouOwn"
        let parts: Vec<&str> = filter.split('.').collect();
        let type_filter = parts.first().copied().unwrap_or("");
        let ownership = parts.get(1).copied().unwrap_or("YouOwn");

        // Get the appropriate zone's cards
        let zone_cards: &[CardId] = match zone {
            Zone::Graveyard => {
                if let Some(zones) = self.game.get_player_zones(player_id) {
                    zones.graveyard.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Hand => {
                if let Some(zones) = self.game.get_player_zones(player_id) {
                    zones.hand.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Battlefield => self.game.battlefield.cards.as_slice(),
            Zone::Exile => {
                if let Some(zones) = self.game.get_player_zones(player_id) {
                    zones.exile.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Library => {
                if let Some(zones) = self.game.get_player_zones(player_id) {
                    zones.library.cards.as_slice()
                } else {
                    return 0;
                }
            }
            Zone::Stack => {
                // Stack items are StackEntry, not directly cards
                return 0;
            }
            Zone::Command => {
                // Command zone (for Commander format) not typically checked
                return 0;
            }
        };

        zone_cards
            .iter()
            .filter(|&&cid| {
                let Some(c) = self.game.cards.try_get(cid) else {
                    return false;
                };

                // Check ownership filter
                let ownership_ok = match ownership {
                    "YouOwn" => c.owner == player_id,
                    "OppOwn" => c.owner != player_id,
                    "YouCtrl" => c.controller == player_id,
                    "OppCtrl" => c.controller != player_id,
                    _ => true,
                };

                if !ownership_ok {
                    return false;
                }

                // Check type filter (subtype match)
                if type_filter.is_empty() {
                    return true;
                }

                // Check if card has the specified subtype
                let subtype = crate::core::Subtype::new(type_filter);
                c.subtypes.contains(&subtype)
            })
            .count()
    }

    /// Check if a card's additional sacrifice costs can be paid
    ///
    /// Returns true if the player has enough permanents to sacrifice for any
    /// RaiseCost::Sacrifice static abilities on the card.
    fn can_pay_sacrifice_costs(&self, card: &crate::core::Card, player_id: PlayerId) -> bool {
        use crate::core::{RaisedCost, RaisedCostAmount, StaticAbility};

        for static_ability in &card.static_abilities {
            if let StaticAbility::RaiseCost {
                raised_cost: RaisedCost::Sacrifice { amount, valid_type },
                ..
            } = static_ability
            {
                // Calculate required sacrifice amount
                let required_amount = match amount {
                    RaisedCostAmount::Fixed(n) => *n as usize,
                    RaisedCostAmount::Variable(svar_name) => {
                        // Evaluate the SVar to get the required amount
                        // For Tectonic Split: SVar:X:Count$Valid Land.YouCtrl/HalfUp
                        self.evaluate_sacrifice_svar(svar_name, &card.svars, player_id, valid_type)
                    }
                };

                // Count available permanents of the required type
                let available = self.count_permanents_by_type(player_id, valid_type);

                if available < required_amount {
                    return false;
                }
            }
        }
        true
    }

    /// Evaluate an SVar for sacrifice cost amount
    ///
    /// Handles patterns like "Count$Valid Land.YouCtrl/HalfUp"
    fn evaluate_sacrifice_svar(
        &self,
        svar_name: &str,
        svars: &std::collections::HashMap<String, String>,
        player_id: PlayerId,
        _valid_type: &str, // Kept for future use (more complex filters)
    ) -> usize {
        // Look up the SVar value
        let Some(svar_value) = svars.get(svar_name) else {
            log::warn!("RaiseCost SVar '{}' not found", svar_name);
            return 0;
        };

        // Parse "Count$Valid Land.YouCtrl/HalfUp" or similar
        if let Some(count_expr) = svar_value.strip_prefix("Count$Valid ") {
            // Split by "/" to get modifier (HalfUp, etc.)
            let parts: Vec<&str> = count_expr.split('/').collect();
            let type_filter = parts.first().copied().unwrap_or("");
            let modifier = parts.get(1).copied().unwrap_or("");

            // Extract the type from filter (e.g., "Land.YouCtrl" -> "Land")
            let filter_type = type_filter.split('.').next().unwrap_or(type_filter);

            // Count matching permanents (use the filter type, not valid_type)
            let count = self.count_permanents_by_type(player_id, filter_type);

            // Apply modifier
            match modifier {
                "HalfUp" => count.div_ceil(2), // Round up
                "Half" => count / 2,           // Round down
                _ => count,
            }
        } else {
            // Try to parse as a simple number
            svar_value.parse().unwrap_or(0)
        }
    }

    /// Count permanents of a specific type controlled by a player
    fn count_permanents_by_type(&self, player_id: PlayerId, type_filter: &str) -> usize {
        self.game
            .battlefield
            .cards
            .iter()
            .filter(|&&card_id| {
                self.game
                    .cards
                    .try_get(card_id)
                    .is_some_and(|c| c.controller == player_id && Self::card_matches_type_filter(c, type_filter))
            })
            .count()
    }

    /// Check if a card matches a type filter string
    fn card_matches_type_filter(card: &crate::core::Card, type_filter: &str) -> bool {
        match type_filter {
            "Land" => card.is_land(),
            "Creature" => card.is_creature(),
            "Artifact" => card.is_artifact(),
            "Enchantment" => card.is_enchantment(),
            "Permanent" => true, // Any permanent matches
            _ => {
                // Try matching as a subtype
                let subtype = crate::core::Subtype::new(type_filter);
                card.subtypes.contains(&subtype)
            }
        }
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
        // OPTIMIZATION: Using try_get_player() to avoid MtgError allocation on failure path
        let mana_pool = self
            .game
            .try_get_player(player_id)
            .map(|p| p.mana_pool)
            .unwrap_or_default();

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
                        // CR 702.8a: Flash allows a permanent to be cast anytime you could cast an instant
                        let has_flash = card.has_keyword(crate::core::Keyword::Flash);
                        let can_cast_now = if card.is_instant() || has_flash {
                            // Instants and cards with Flash can be cast anytime with priority
                            true
                        } else {
                            // Creatures and sorceries require:
                            // - Sorcery speed (Main1 or Main2)
                            // - Active player
                            // - Stack is empty
                            is_sorcery_speed && is_active_player && stack_is_empty
                        };

                        if can_cast_now {
                            // Calculate effective cost (applies Affinity and other cost reductions)
                            let effective_cost = self.calculate_effective_cost(card, player_id);

                            // Check if we can pay for this spell's effective mana cost
                            // Use can_pay_with_pool to consider floating mana from rituals like Dark Ritual
                            // Also check if we can pay any additional sacrifice costs (RaiseCost)
                            let can_afford_mana = self.mana_engine.can_pay_with_pool(&effective_cost, &mana_pool);
                            let can_afford_sacrifice = self.can_pay_sacrifice_costs(card, player_id);

                            if can_afford_mana && can_afford_sacrifice {
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
                                } else if Self::spell_requires_battlefield_target(card) {
                                    // For spells like Disenchant, Terror, Swords to Plowshares
                                    // MTG Rule 601.2c: You can't begin casting a spell that targets
                                    // unless there's at least one legal target
                                    let has_valid_targets = self
                                        .game
                                        .get_valid_targets_for_spell(card_id)
                                        .map(|targets| !targets.is_empty())
                                        .unwrap_or(false);
                                    if has_valid_targets {
                                        self.abilities_buffer.push(SpellAbility::CastSpell { card_id });
                                    }
                                } else {
                                    // For non-targeting spells, check if we understand them
                                    // Instants/sorceries with empty effects likely have unimplemented
                                    // abilities (like Charm) - don't offer them
                                    // Permanents (creatures, artifacts, enchantments) are still valid
                                    // to cast even with no immediate effects (they enter battlefield)
                                    let is_permanent = card.is_creature()
                                        || card.is_artifact()
                                        || card.is_enchantment()
                                        || card.is_planeswalker();
                                    if is_permanent || !card.effects.is_empty() {
                                        self.abilities_buffer.push(SpellAbility::CastSpell { card_id });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Push castable spells from exile (via MayPlayFromExile persistent effects)
    ///
    /// Checks persistent effects like Airbend that grant permission to cast
    /// exiled cards for an alternative cost.
    fn push_castable_from_exile(&mut self, player_id: PlayerId) {
        use crate::core::{ManaCost, PersistentEffectKind, SpellAbility};

        // Update the mana engine for this player
        self.mana_engine.update_mut(self.game, player_id);

        // Get the player's mana pool for checking floating mana
        // OPTIMIZATION: Using try_get_player() to avoid MtgError allocation on failure path
        let mana_pool = self
            .game
            .try_get_player(player_id)
            .map(|p| p.mana_pool)
            .unwrap_or_default();

        // Check if this is the active player (only active player can cast sorceries)
        let is_active_player = self.game.turn.active_player == player_id;

        // Check if it's sorcery speed (Main1 or Main2)
        let is_sorcery_speed = self.game.turn.current_step.is_sorcery_speed();

        // Check if stack is empty (required for sorcery-speed spells)
        let stack_is_empty = self.game.stack.is_empty();

        // Iterate through all persistent effects looking for MayPlayFromExile
        for effect in self.game.persistent_effects.all() {
            match &effect.kind {
                PersistentEffectKind::MayPlayFromExile {
                    tracked_card,
                    alternative_cost,
                    owner,
                } => {
                    // Only the card's owner can cast it
                    if *owner != player_id {
                        continue;
                    }

                    // Get the card from exile
                    if let Some(card) = self.game.cards.try_get(*tracked_card) {
                        // Verify card is actually in exile
                        // (effect cleanup should handle this, but double-check)
                        if !self.game.is_card_in_exile(*tracked_card) {
                            continue;
                        }

                        // Check timing restrictions
                        let can_cast_now = if card.is_instant() {
                            true
                        } else {
                            is_sorcery_speed && is_active_player && stack_is_empty
                        };

                        if can_cast_now {
                            // Check if we can pay the alternative cost
                            if self.mana_engine.can_pay_with_pool(alternative_cost, &mana_pool) {
                                self.abilities_buffer.push(SpellAbility::CastFromExile {
                                    card_id: *tracked_card,
                                    alternative_cost: *alternative_cost,
                                    effect_id: effect.id,
                                });
                            }
                        }
                    }
                }

                PersistentEffectKind::MayPlayOneWithoutManaCost {
                    tracked_cards,
                    beneficiary,
                } => {
                    // Only the beneficiary can cast these cards
                    if *beneficiary != player_id {
                        continue;
                    }

                    // For each tracked card, check if we can cast it
                    for &tracked_card in tracked_cards {
                        // Get the card from exile
                        if let Some(card) = self.game.cards.try_get(tracked_card) {
                            // Verify card is actually in exile
                            if !self.game.is_card_in_exile(tracked_card) {
                                continue;
                            }

                            // Check timing restrictions
                            let can_cast_now = if card.is_instant() {
                                true
                            } else {
                                is_sorcery_speed && is_active_player && stack_is_empty
                            };

                            if can_cast_now {
                                // No mana cost needed - cast for free!
                                self.abilities_buffer.push(SpellAbility::CastFromExile {
                                    card_id: tracked_card,
                                    alternative_cost: ManaCost::new(), // Zero cost
                                    effect_id: effect.id,
                                });
                            }
                        }
                    }
                }

                // Other persistent effect kinds don't grant casting permission
                PersistentEffectKind::Imprint { .. }
                | PersistentEffectKind::Suspend { .. }
                | PersistentEffectKind::CantBeBlocked { .. } => {}
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
        // OPTIMIZATION: Using try_get_player() to avoid MtgError allocation on failure path
        let mana_pool = self
            .game
            .try_get_player(player_id)
            .map(|p| p.mana_pool)
            .unwrap_or_default();

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
                    // When the ability includes a tap cost and the source card is a mana source,
                    // we can't use that card's mana ability to pay the cost (it's already being tapped).
                    if let Some(mana_cost) = ability.cost.get_mana_cost() {
                        let can_pay = if ability.cost.includes_tap() && card.definition.cache.is_mana_source {
                            // Filter out this card from mana sources and check affordability
                            use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};
                            let all_sources = self.mana_engine.all_sources();
                            let filtered: smallvec::SmallVec<[_; 8]> =
                                all_sources.iter().filter(|s| s.card_id != card_id).cloned().collect();
                            let resolver = GreedyManaResolver::new();
                            // First check if pool alone can pay
                            if mana_pool.can_pay(mana_cost) {
                                true
                            } else {
                                // Check if pool + filtered sources can pay
                                resolver.can_pay(mana_cost, &filtered)
                            }
                        } else {
                            self.mana_engine.can_pay_with_pool(mana_cost, &mana_pool)
                        };
                        if !can_pay {
                            can_activate = false;
                        }
                    }

                    // Check Waterbend cost (Avatar set mechanic - like Convoke)
                    // Waterbend N requires N total payment via mana OR tapping creatures/artifacts
                    // Similar to Convoke: "you can tap your artifacts and creatures to help. Each one pays for {1}."
                    if can_activate {
                        if let Some(waterbend_amount) = ability.cost.get_waterbend_amount() {
                            // Get mana sources (lands and creatures/artifacts with mana abilities)
                            let mana_sources = self.mana_engine.all_sources();
                            let mana_source_ids: smallvec::SmallVec<[CardId; 16]> =
                                mana_sources.iter().map(|s| s.card_id).collect();
                            let mana_available = mana_sources.iter().filter(|s| s.card_id != card_id).count() as u8;

                            // Count untapped creatures/artifacts that are NOT mana sources
                            // (mana sources are already counted above - avoid double-counting)
                            let tappable_for_waterbend = self
                                .game
                                .battlefield
                                .cards
                                .iter()
                                .filter(|&&cid| {
                                    if cid == card_id {
                                        return false; // Can't tap the source to pay its own cost
                                    }
                                    // Skip if it's already counted as a mana source
                                    if mana_source_ids.contains(&cid) {
                                        return false;
                                    }
                                    if let Some(c) = self.game.cards.try_get(cid) {
                                        !c.tapped && c.controller == player_id && (c.is_creature() || c.is_artifact())
                                    } else {
                                        false
                                    }
                                })
                                .count() as u8;

                            // Total payment capacity = floating mana + mana from sources + tappable creatures/artifacts
                            let total_available = mana_pool.total() + mana_available + tappable_for_waterbend;
                            if total_available < waterbend_amount {
                                can_activate = false;
                            }
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

                    // Check sacrifice cost (e.g., "Sac<1/Saproling>")
                    // Without this check, abilities with unpayable sacrifice costs
                    // would appear as available, causing infinite loops when AI tries
                    // to activate them repeatedly.
                    if can_activate {
                        if let Some((sac_count, sac_pattern)) = ability.cost.get_sacrifice_pattern() {
                            if !self
                                .game
                                .can_pay_sacrifice_pattern(sac_pattern, sac_count, card_id, player_id)
                            {
                                can_activate = false;
                            }
                        }
                    }

                    // TODO: Check other cost types (discard, etc.)
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

                    // Check your-turn-only restriction (PlayerTurn$ True)
                    // Less restrictive than sorcery speed - only requires it to be your turn
                    if can_activate && ability.your_turn_only {
                        let is_your_turn = card.controller == self.game.turn.active_player;
                        if !is_your_turn {
                            can_activate = false;
                        }
                    }

                    // Check exhaust restriction (Exhaust$ True)
                    // Exhaust abilities can only be activated once per game
                    if can_activate && ability.exhaust && card.exhausted_abilities.contains(&ability_index) {
                        can_activate = false;
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

    /// Push cycling abilities for cards in hand
    ///
    /// Cycling abilities are activated from hand (not battlefield).
    /// MTG CR 702.29: "Cycling is an activated ability that functions only
    /// while the card with cycling is in a player's hand."
    ///
    /// Checks for both regular Cycling and Typecycling (Mountaincycling, etc.)
    fn push_cycling_abilities(&mut self, player_id: PlayerId) {
        use crate::core::{Keyword, KeywordArgs, SpellAbility};

        // Update mana engine for cost checking
        self.mana_engine.update_mut(self.game, player_id);
        // OPTIMIZATION: Using try_get_player() to avoid MtgError allocation on failure path
        let mana_pool = self
            .game
            .try_get_player(player_id)
            .map(|p| p.mana_pool)
            .unwrap_or_default();

        // Get cards in hand
        let hand = self
            .game
            .player_zones
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, zones)| &zones.hand);

        let Some(hand) = hand else {
            return;
        };

        // Check each card in hand for cycling abilities
        for &card_id in &hand.cards {
            if let Some(card) = self.game.cards.try_get(card_id) {
                // Check for regular Cycling keyword
                if card.keywords.contains(Keyword::Cycling) {
                    if let Some(KeywordArgs::Cycling { cost }) = card.keywords.get_args(Keyword::Cycling) {
                        // Check if we can pay the cycling cost
                        if self.mana_engine.can_pay_with_pool(cost, &mana_pool) {
                            self.abilities_buffer.push(SpellAbility::Cycle {
                                card_id,
                                cost: *cost,
                                search_type: None, // Regular cycling draws a card
                            });
                        }
                    }
                }

                // Check for Typecycling (Mountaincycling, Swampcycling, etc.)
                if card.keywords.contains(Keyword::Typecycling) {
                    if let Some(KeywordArgs::Typecycling { cost, card_type }) =
                        card.keywords.get_args(Keyword::Typecycling)
                    {
                        // Check if we can pay the typecycling cost
                        if self.mana_engine.can_pay_with_pool(cost, &mana_pool) {
                            self.abilities_buffer.push(SpellAbility::Cycle {
                                card_id,
                                cost: *cost,
                                search_type: Some(card_type.clone()), // Search for this land type
                            });
                        }
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

        // Fail-fast validation for network debugging: ensure all cards are revealed
        if self.debug_validate_reveals {
            self.validate_cards_revealed(player_id);
        }

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

        // Add castable spells from exile (Airbend, Suspend, etc.)
        self.push_castable_from_exile(player_id);

        // Add activated abilities (pushes directly to abilities_buffer)
        self.push_activatable_abilities(player_id);

        // Add cycling abilities from hand (Cycling, Mountaincycling, etc.)
        self.push_cycling_abilities(player_id);

        // Sort by card ID to ensure deterministic ordering
        // This is critical for snapshot/resume: if two runs have the same cards available
        // but in different hand order, we need to present them in the same order so that
        // index-based choice replay (FixedScriptController) selects the same logical card
        self.abilities_buffer.sort_unstable_by_key(SpellAbility::card_id);

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
            .any(|effect| matches!(effect, Effect::CounterSpell { target } if target.is_placeholder()))
    }

    /// Check if a spell requires a battlefield target (e.g., Disenchant, Terror)
    ///
    /// Returns true if the spell has effects that require targeting a permanent.
    /// Per MTG Rule 601.2c: You can't begin casting a spell that targets unless
    /// there's a legal target.
    ///
    /// Note: Wildcard is intentional - Effect has 24+ variants; we check for specific
    /// targeting effect types (Destroy, Pump, Tap, Exile, Copy with placeholder targets).
    #[allow(clippy::wildcard_enum_match_arm)]
    fn spell_requires_battlefield_target(card: &crate::core::Card) -> bool {
        use crate::core::Effect;

        card.effects.iter().any(|effect| {
            match effect {
                // DestroyPermanent with placeholder target (CardId(0))
                Effect::DestroyPermanent { target, .. } if target.is_placeholder() => true,
                // PumpCreature with placeholder target
                Effect::PumpCreature { target, .. } if target.is_placeholder() => true,
                // TapPermanent with placeholder target
                Effect::TapPermanent { target } if target.is_placeholder() => true,
                // UntapPermanent with placeholder target
                Effect::UntapPermanent { target } if target.is_placeholder() => true,
                // ExilePermanent with placeholder target
                Effect::ExilePermanent { target } if target.is_placeholder() => true,
                // CopyPermanent with placeholder target
                Effect::CopyPermanent { target, .. } if target.is_placeholder() => true,
                _ => false,
            }
        })
    }
}
