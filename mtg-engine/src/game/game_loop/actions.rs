//! Action query module for GameLoop
//!
//! Provides read-only queries for available player actions (attackers, blockers, spells, abilities).
//! These functions support controller decision-making without modifying game state.

use super::GameLoop;
use crate::core::{CardId, Keyword, PlayerId, StaticAbility};
use crate::game::phase::Step;
use smallvec::SmallVec;

impl<'a> GameLoop<'a> {
    /// Validate that all cards in hand are revealed (network debug mode)
    ///
    /// Called when `debug_validate_reveals` is enabled (gated by `network_debug` flag).
    /// In network mode with hidden info, only validates the local player's cards
    /// (opponent's cards are intentionally hidden per mtg-218).
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

        // Pre-compute the defending player id once (2-player: the other player).
        let defending_player_id = self.game.players.iter().find(|p| p.id != player_id).map(|p| p.id);

        // Island Sanctuary protection (CR 614): if the defending player activated
        // sanctuary this turn, only creatures with flying or islandwalk may attack.
        let defender_has_sanctuary = defending_player_id.is_some_and(|def_id| {
            self.game
                .players
                .iter()
                .find(|p| p.id == def_id)
                .is_some_and(|p| p.island_sanctuary_protected)
        });

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
                        entered_turn == self.game.turn.turn_number
                            && !self.game.has_keyword_with_effects(card_id, Keyword::Haste)
                    } else {
                        false
                    };

                    // Check for defender keyword
                    let has_defender = card.has_defender();

                    // Check conditional CantAttack statics on the creature itself
                    // (e.g. Orgg: "can't attack if defending player controls an
                    // untapped creature with power >= N"). CR 508.1c.
                    let blocked_by_cant_attack_static = card
                        .static_abilities
                        .iter()
                        .any(|sa| self.cant_attack_static_fires(sa, defending_player_id));

                    // Island Sanctuary (CR 614): only creatures with flying or
                    // islandwalk (Landwalk:Island) can attack a protected player.
                    let blocked_by_sanctuary = defender_has_sanctuary
                        && !self.game.has_keyword_with_effects(card_id, Keyword::Flying)
                        && !self.game.has_islandwalk(card_id);

                    // Check global CantAttackOrBlockMatching statics on other
                    // battlefield permanents (e.g. Light of Day: "black creatures
                    // can't attack or block"). CR 508.1c.
                    let globally_cant_attack = self.game.is_attack_prohibited(card);

                    if !has_summoning_sickness
                        && !has_defender
                        && !blocked_by_cant_attack_static
                        && !blocked_by_sanctuary
                        && !globally_cant_attack
                    {
                        creatures.push(card_id);
                    }
                }
            }
        }

        // Sort for deterministic ordering
        creatures.sort();
        creatures
    }

    /// Returns `true` if a `CantAttack`-family static ability blocks the
    /// creature from appearing in the attack-declaration choice menu.
    ///
    /// Called once per potential attacker, per ability, so this must be cheap.
    fn cant_attack_static_fires(&self, ability: &StaticAbility, defender_id: Option<PlayerId>) -> bool {
        match ability {
            StaticAbility::CantAttackIfDefenderHasUntappedPowerGE { min_power, .. } => {
                // Orgg (CR 508.1): "can't attack if defending player controls
                // an untapped creature with power >= min_power."
                let Some(def_id) = defender_id else {
                    return false;
                };
                self.game.battlefield.cards.iter().any(|&bid| {
                    self.game.cards.try_get(bid).is_some_and(|c| {
                        c.controller == def_id
                            && c.is_creature()
                            && !c.tapped
                            && i32::from(c.current_power()) >= *min_power
                    })
                })
            }
            // None of the other self-carried static abilities restrict attacking
            // on their own (global CantAttackOrBlockMatching is handled via
            // is_attack_prohibited() which scans the whole battlefield).
            StaticAbility::ModifyPT { .. }
            | StaticAbility::GrantKeyword { .. }
            | StaticAbility::ReduceCost { .. }
            | StaticAbility::RaiseCost { .. }
            | StaticAbility::GrantAbility { .. }
            | StaticAbility::GainControl { .. }
            | StaticAbility::SacrificeMatchingPresent { .. }
            | StaticAbility::CantBeCast { .. }
            | StaticAbility::CantPlayLand { .. }
            | StaticAbility::CantBlockMatching { .. }
            | StaticAbility::CantAttackOrBlockMatching { .. }
            | StaticAbility::CantBeActivated { .. }
            | StaticAbility::CastWithFlash { .. }
            | StaticAbility::DamageIncrease { .. }
            | StaticAbility::PreventDamageToEnchantedByChosenColor { .. }
            | StaticAbility::ExtraLandPlay { .. }
            | StaticAbility::LifeFloor { .. }
            | StaticAbility::DamageToExileLibrary { .. }
            | StaticAbility::CharacteristicDefiningPt { .. } => false,
        }
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
                    // Global CantAttackOrBlockMatching statics (e.g. Light of Day:
                    // "black creatures can't attack or block"). CR 509.1b.
                    && !self.game.is_block_prohibited(card)
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
        use crate::core::{KeywordArgs, StaticAbility};

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

        // Check for ReduceCost / RaiseCost static abilities from permanents.
        // Polarity rules (CR 601.2f): ReduceCost only helps its own controller;
        // RaiseCost ("hose" effects like Gloom, Karma) applies regardless of
        // who controls the source. See actions/mod.rs::calculate_effective_cost
        // for the canonical implementation — this is the same logic from the
        // get-available-actions / cost-payability path.
        for &bf_card_id in &self.game.battlefield.cards {
            let Some(source_card) = self.game.cards.try_get(bf_card_id) else {
                continue;
            };

            for static_ability in &source_card.static_abilities {
                if let StaticAbility::ReduceCost {
                    valid_card,
                    amount,
                    condition,
                    description,
                } = static_ability
                {
                    // ReduceCost only applies to the source controller's own spells.
                    if source_card.controller != player_id {
                        continue;
                    }

                    if !crate::game::actions::spell_matches_cost_filter(card, valid_card) {
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

                // Also check for RaiseCost (mana-based cost increases). Applies
                // regardless of source controller.
                if let StaticAbility::RaiseCost {
                    valid_card,
                    raised_cost,
                    description,
                } = static_ability
                {
                    use crate::core::RaisedCost;

                    if !crate::game::actions::spell_matches_cost_filter(card, valid_card) {
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
                self.game.cards.try_get(card_id).is_some_and(|c| {
                    c.controller == player_id && crate::game::GameState::card_matches_type_filter_static(c, type_filter)
                })
            })
            .count()
    }

    /// Push castable spells directly to abilities_buffer
    ///
    /// Zero allocation - pushes SpellAbility::CastSpell directly to the buffer
    /// instead of building an intermediate Vec.
    ///
    /// `pub(crate)` so unit tests can directly assert that the right spells
    /// surface in the action buffer (e.g. that an instant from hand is
    /// offered when the stack is non-empty — the response-window scenario).
    pub(crate) fn push_castable_spells(&mut self, player_id: PlayerId) {
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
                    // Cast-prohibition statics (City in a Bottle's CantBeCast /
                    // CantPlayLand on ARN-origin cards). A prohibited spell is
                    // never offered as a legal play. (CR 605: a spell can't be
                    // cast while a continuous effect says it can't.)
                    if self.game.is_play_prohibited(card) {
                        continue;
                    }
                    // Adventure (CR 715): if this card has an Adventure face, the
                    // owner may cast the instant/sorcery half from hand. Offered
                    // independently of the creature half, using the Adventure
                    // face's own type (timing), mana cost, and target requirement.
                    if let Some(adventure) = card.definition.adventure.as_deref() {
                        let adv_is_instant = adventure.types.contains(&crate::core::CardType::Instant);
                        let adv_can_cast_now = if adv_is_instant {
                            true
                        } else {
                            // Sorcery Adventure: sorcery-speed, active player, empty stack.
                            is_sorcery_speed && is_active_player && stack_is_empty
                        };
                        if adv_can_cast_now
                            && self.mana_engine.can_pay_with_pool(&adventure.mana_cost, &mana_pool)
                            && self.adventure_has_legal_targets(card_id, adventure)
                        {
                            self.abilities_buffer.push(SpellAbility::CastAdventure { card_id });
                        }
                    }

                    // Check if card is castable (not a land)
                    if !card.is_land() {
                        // Check timing restrictions
                        // CR 702.8a: Flash allows a permanent to be cast anytime you could cast an instant
                        let has_flash = card.has_keyword(crate::core::Keyword::Flash)
                            || self.game.player_has_cast_with_flash(player_id, card);
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
                                    // Check if there are valid enchantment targets.
                                    // MTG Rule 303.4a / 601.2c: an Aura can only be cast if a
                                    // legal target exists in the relevant zone.
                                    //
                                    // Most Auras enchant battlefield permanents, but some
                                    // (Animate Dead, Dance of the Dead, Spellweaver Volute)
                                    // use `Enchant:Creature.inZoneGraveyard` and target a
                                    // creature card in a graveyard. Strip the `.inZone<X>`
                                    // qualifier from the type check and search zone X for
                                    // those cards.
                                    let enchant_type =
                                        card.keywords.get_args(crate::core::Keyword::Enchant).and_then(|args| {
                                            if let crate::core::KeywordArgs::Enchant { card_type } = args {
                                                Some(card_type.as_str().to_string())
                                            } else {
                                                None
                                            }
                                        });

                                    let (base_type, target_zone): (Option<String>, Option<String>) =
                                        match enchant_type.as_deref() {
                                            Some(s) => {
                                                let lower = s.to_lowercase();
                                                if let Some((bt, zone)) = lower.split_once(".inzone") {
                                                    (Some(bt.to_string()), Some(zone.to_string()))
                                                } else {
                                                    (Some(s.to_string()), None)
                                                }
                                            }
                                            None => (None, None),
                                        };

                                    let type_matches = |target_card: &crate::core::card::Card| -> bool {
                                        match base_type.as_deref() {
                                            Some("creature" | "Creature") | None => target_card.is_creature(),
                                            Some("land" | "Land") => target_card.is_land(),
                                            Some("artifact" | "Artifact") => target_card.is_artifact(),
                                            Some("enchantment" | "Enchantment") => target_card.is_enchantment(),
                                            Some("permanent" | "Permanent") => true,
                                            Some(other) => {
                                                // Creature subtype check
                                                target_card.is_creature()
                                                    && target_card
                                                        .subtypes
                                                        .iter()
                                                        .any(|st| st.as_str().eq_ignore_ascii_case(other))
                                            }
                                        }
                                    };

                                    let has_valid_targets = match target_zone.as_deref() {
                                        Some("graveyard") => {
                                            // Search ALL graveyards (own and opponent's) for
                                            // matching cards (CR 303.4f).
                                            self.game.player_zones.iter().any(|(_, zones)| {
                                                zones.graveyard.cards.iter().any(|&card_id| {
                                                    self.game.cards.try_get(card_id).is_some_and(&type_matches)
                                                })
                                            })
                                        }
                                        _ => self.game.battlefield.cards.iter().any(|&target_id| {
                                            self.game.cards.try_get(target_id).is_some_and(&type_matches)
                                        }),
                                    };

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
                                } else if let Some(graveyard_filter) = Self::spell_requires_graveyard_target(card) {
                                    // For spells like Regrowth that target a card in a graveyard
                                    // (CR 601.2c: can't cast without at least one legal target).
                                    // YouCtrl effects search only the caster's graveyard; unrestricted
                                    // effects (e.g. Debtors' Knell) search all graveyards.
                                    let has_valid_targets = self.game.player_zones.iter().any(|(pid, zones)| {
                                        if graveyard_filter.own_only && *pid != player_id {
                                            return false;
                                        }
                                        zones.graveyard.cards.iter().any(|&gy_id| {
                                            self.game
                                                .cards
                                                .try_get(gy_id)
                                                .is_some_and(|gy_card| graveyard_filter.type_matches(gy_card))
                                        })
                                    });
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

                PersistentEffectKind::MayPlayFromGraveyard {
                    owner,
                    max_power,
                    max_toughness,
                    your_turn_only,
                    add_finality_counter,
                } => {
                    if *owner != player_id {
                        continue;
                    }
                    // Check turn restriction
                    if *your_turn_only && !is_active_player {
                        continue;
                    }

                    // Check all creatures in the player's graveyard
                    let graveyard_cards: smallvec::SmallVec<[CardId; 16]> = self
                        .game
                        .get_player_zones(player_id)
                        .map(|zones| zones.graveyard.cards.iter().copied().collect())
                        .unwrap_or_default();

                    for card_id in graveyard_cards {
                        if let Some(card) = self.game.cards.try_get(card_id) {
                            // Must be a creature
                            if !card.is_creature() {
                                continue;
                            }

                            // Check power/toughness restrictions
                            if let Some(max_p) = max_power {
                                if i32::from(card.current_power()) > *max_p {
                                    continue;
                                }
                            }
                            if let Some(max_t) = max_toughness {
                                if i32::from(card.current_toughness()) > *max_t {
                                    continue;
                                }
                            }

                            // Check timing (creatures are sorcery speed)
                            if !(is_sorcery_speed && stack_is_empty) {
                                continue;
                            }

                            // Check if we can pay the mana cost
                            if self.mana_engine.can_pay_with_pool(&card.mana_cost, &mana_pool) {
                                self.abilities_buffer.push(SpellAbility::CastFromGraveyard {
                                    card_id,
                                    effect_id: effect.id,
                                    add_finality_counter: *add_finality_counter,
                                });
                            }
                        }
                    }
                }

                PersistentEffectKind::CastTargetedSpellFromGraveyard {
                    tracked_card, owner, ..
                } => {
                    if *owner != player_id {
                        continue;
                    }

                    // Verify the card is still in the graveyard
                    let is_in_graveyard = self
                        .game
                        .get_player_zones(player_id)
                        .map(|zones| zones.graveyard.cards.contains(tracked_card))
                        .unwrap_or(false);
                    if !is_in_graveyard {
                        continue;
                    }

                    // Check timing: instants can be cast any time, sorceries need sorcery speed
                    let can_cast_now = if let Some(card) = self.game.cards.try_get(*tracked_card) {
                        if card.is_instant() {
                            true
                        } else {
                            is_sorcery_speed && is_active_player && stack_is_empty
                        }
                    } else {
                        continue;
                    };

                    if !can_cast_now {
                        continue;
                    }

                    // Check mana affordability
                    let mana_cost = self
                        .game
                        .cards
                        .try_get(*tracked_card)
                        .map(|c| c.mana_cost)
                        .unwrap_or_default();
                    if self.mana_engine.can_pay_with_pool(&mana_cost, &mana_pool) {
                        self.abilities_buffer.push(SpellAbility::CastFromGraveyard {
                            card_id: *tracked_card,
                            effect_id: effect.id,
                            add_finality_counter: false, // Chandra -2 doesn't add finality
                        });
                    }
                }

                // Other persistent effect kinds don't grant casting permission
                PersistentEffectKind::Imprint { .. }
                | PersistentEffectKind::Suspend { .. }
                | PersistentEffectKind::CantBeBlocked { .. }
                // ExtraLandPlay is queried via can_play_land_effective(), not here
                | PersistentEffectKind::ExtraLandPlay { .. } => {}
            }
        }
    }

    /// Push castable commander from command zone to abilities_buffer
    ///
    /// In Commander format, the commander can be cast from the command zone
    /// by paying its mana cost plus commander tax ({2} per previous cast).
    /// MTG CR 903.8.
    fn push_castable_from_command(&mut self, player_id: PlayerId) {
        use crate::core::SpellAbility;

        // Update the mana engine for this player
        self.mana_engine.update_mut(self.game, player_id);

        let mana_pool = self
            .game
            .try_get_player(player_id)
            .map(|p| p.mana_pool)
            .unwrap_or_default();

        let is_active_player = self.game.turn.active_player == player_id;
        let is_sorcery_speed = self.game.turn.current_step.is_sorcery_speed();
        let stack_is_empty = self.game.stack.is_empty();

        // Get commander tax from the player
        let commander_tax = self
            .game
            .try_get_player(player_id)
            .map(|p| p.commander_tax())
            .unwrap_or(0);

        if let Some(zones) = self.game.get_player_zones(player_id) {
            for &card_id in &zones.command.cards {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    // Check timing restrictions
                    // Commander follows normal casting timing rules
                    let has_flash = card.has_keyword(crate::core::Keyword::Flash)
                        || self.game.player_has_cast_with_flash(player_id, card);
                    let can_cast_now = if card.is_instant() || has_flash {
                        true
                    } else {
                        // Sorcery-speed casting rules
                        is_sorcery_speed && is_active_player && stack_is_empty
                    };

                    if can_cast_now {
                        // Calculate total cost = base mana cost + commander tax as generic mana
                        let mut total_cost = card.mana_cost;
                        total_cost.generic = total_cost.generic.saturating_add(commander_tax);

                        // Check if we can afford it
                        let can_afford = self.mana_engine.can_pay_with_pool(&total_cost, &mana_pool);

                        if can_afford {
                            self.abilities_buffer
                                .push(SpellAbility::CastFromCommand { card_id, total_cost });
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
    pub(crate) fn push_activatable_abilities(&mut self, player_id: PlayerId) {
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

                // CantBeActivated statics (Cursed Totem): if any battlefield
                // permanent has a CantBeActivated static that matches this card,
                // none of its activated abilities may be activated. CR 602.1.
                if card.is_creature() && self.game.is_activated_ability_prohibited(card) {
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

                    // Summoning sickness: creatures can't use tap-activated abilities
                    // the turn they enter the battlefield, unless they have haste (CR 302.6)
                    if can_activate && ability.cost.includes_tap() && card.is_creature() {
                        if let Some(entered_turn) = card.turn_entered_battlefield {
                            if entered_turn == self.game.turn.turn_number
                                && !self.game.has_keyword_with_effects(card_id, crate::core::Keyword::Haste)
                            {
                                can_activate = false;
                            }
                        }
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
                            // CRITICAL (bug-vinebender-triple-activation): we must only count
                            // sources that can ACTUALLY produce mana right now — i.e. untapped
                            // and (for creatures) not summoning-sick. Otherwise the AI re-offers
                            // the ability after every land has already been tapped to pay it,
                            // leading to multiple "free" activations in a row.
                            let mana_sources = self.mana_engine.all_sources();
                            let mana_source_ids: smallvec::SmallVec<[CardId; 16]> =
                                mana_sources.iter().map(|s| s.card_id).collect();
                            let mana_available = mana_sources
                                .iter()
                                .filter(|s| s.card_id != card_id && !s.is_tapped && !s.has_summoning_sickness)
                                .count() as u8;

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

                    // Check loyalty cost: once-per-turn rule (MTG CR 606.3) and affordability
                    // Uses contains_loyalty_cost() to handle loyalty costs inside Composite
                    if can_activate && ability.cost.contains_loyalty_cost() {
                        if card.loyalty_activated_this_turn {
                            can_activate = false;
                        } else if let Some(amount) = ability.cost.get_sub_loyalty_amount() {
                            // Check affordability for SubLoyalty (works for top-level and Composite)
                            let loyalty = card.get_counter(crate::core::CounterType::Loyalty);
                            if loyalty < amount {
                                can_activate = false;
                            }
                        }
                    }

                    // Check generic counter-removal affordability (Cost::SubCounter, e.g.,
                    // Triskelion's "remove a +1/+1 counter"). Unlike loyalty there is no
                    // once-per-turn restriction — only the counter-presence check.
                    if can_activate {
                        if let Some((amount, counter_type)) = ability.cost.get_sub_counter_requirement() {
                            if card.get_counter(counter_type) < amount {
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

                    // Check "Activate only if ..." restriction (IsPresent$ /
                    // PresentZone$ / PresentCompare$). Library of Alexandria's
                    // draw ability is gated on "exactly seven cards in hand".
                    if can_activate {
                        if let Some(cond) = &ability.activation_condition {
                            let actual = self
                                .game
                                .count_cards_matching_filter(player_id, &cond.filter, cond.zone);
                            if !cond.op.matches(actual, cond.count as usize) {
                                can_activate = false;
                            }
                        }
                    }

                    // Check ActivationPhases$ window (Jade Statue's combat-only
                    // animate, CR 602.5). The current step must fall within
                    // [start, end] in turn order.
                    if can_activate {
                        if let Some(window) = &ability.activation_phases {
                            if !window.contains(self.game.turn.current_step) {
                                can_activate = false;
                            }
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

        // Also enumerate graveyard-activated abilities (ActivationZone$ Graveyard).
        // CR 602.1: an activated ability's activation zone is declared on the card;
        // if it says Graveyard the card activates from its owner's graveyard.
        // This enables e.g. Earthquake Dragon's {2}{G}, Sac a land: return to hand.
        let graveyard_cards: smallvec::SmallVec<[CardId; 8]> = self
            .game
            .get_player_zones(player_id)
            .map(|z| z.graveyard.cards.iter().copied().collect())
            .unwrap_or_default();

        for card_id in graveyard_cards {
            if let Some(card) = self.game.cards.try_get(card_id) {
                // The card must be owned by this player (graveyards are owner-based).
                if card.owner != player_id {
                    continue;
                }

                for (ability_index, ability) in card.activated_abilities.iter().enumerate() {
                    // Only graveyard-zone abilities
                    if ability.activation_zone != crate::zones::Zone::Graveyard {
                        continue;
                    }
                    // Mana abilities are never graveyard-activated in practice, but guard anyway.
                    if ability.is_mana_ability {
                        continue;
                    }

                    let mut can_activate = true;

                    // Mana cost check (same as battlefield path, but card is never a mana source here)
                    if let Some(mana_cost) = ability.cost.get_mana_cost() {
                        if !self.mana_engine.can_pay_with_pool(mana_cost, &mana_pool) {
                            can_activate = false;
                        }
                    }

                    // Sacrifice cost check (e.g., Sac<1/Land> for Earthquake Dragon)
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

                    // Sorcery-speed timing check
                    if can_activate && ability.sorcery_speed {
                        let is_main_phase = self.game.turn.current_step.is_sorcery_speed();
                        let is_your_turn = self.game.turn.active_player == player_id;
                        let stack_empty = self.game.stack.is_empty();
                        if !is_main_phase || !is_your_turn || !stack_empty {
                            can_activate = false;
                        }
                    }

                    // Your-turn-only check
                    if can_activate && ability.your_turn_only && self.game.turn.active_player != player_id {
                        can_activate = false;
                    }

                    // Activation condition (IsPresent$) check
                    if can_activate {
                        if let Some(cond) = &ability.activation_condition {
                            let actual = self
                                .game
                                .count_cards_matching_filter(player_id, &cond.filter, cond.zone);
                            if !cond.op.matches(actual, cond.count as usize) {
                                can_activate = false;
                            }
                        }
                    }

                    // ActivationPhases$ window check (CR 602.5).
                    if can_activate {
                        if let Some(window) = &ability.activation_phases {
                            if !window.contains(self.game.turn.current_step) {
                                can_activate = false;
                            }
                        }
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
            && self.game.can_play_land_effective(player_id)
        {
            // Collect first (the closure below borrows self.game while
            // we also push into self.abilities_buffer).
            let playable: smallvec::SmallVec<[crate::core::CardId; 8]> = Self::lands_in_hand_iter(self.game, player_id)
                .filter(|&land_id| {
                    // Skip lands a CantPlayLand / CantBeCast static
                    // prohibits (City in a Bottle: ARN-origin lands).
                    !self
                        .game
                        .cards
                        .try_get(land_id)
                        .is_some_and(|c| self.game.is_play_prohibited(c))
                })
                .collect();
            for land_id in playable {
                self.abilities_buffer.push(SpellAbility::PlayLand { card_id: land_id });
            }
        }

        // Add castable spells (pushes directly to abilities_buffer)
        self.push_castable_spells(player_id);

        // Add castable spells from exile (Airbend, Suspend, etc.)
        self.push_castable_from_exile(player_id);

        // Add castable commander from command zone (Commander format)
        if self.game.is_commander_game {
            self.push_castable_from_command(player_id);
        }

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
    /// Decide whether an Adventure spell (CR 715) can legally be cast right now
    /// with respect to TARGET availability (CR 601.2c: can't begin casting a
    /// targeting spell with no legal target). Builds a transient Adventure-face
    /// card so the existing `spell_requires_*` predicates and the battlefield
    /// scan can be reused unchanged. The transient card is NOT inserted into the
    /// store — it is only inspected for its parsed effects and cache flags.
    fn adventure_has_legal_targets(&self, card_id: CardId, adventure: &crate::loader::CardDefinition) -> bool {
        let owner = self
            .game
            .cards
            .try_get(card_id)
            .map(|c| c.owner)
            .unwrap_or_else(|| crate::core::PlayerId::new(0));
        let adv_card = adventure.instantiate(card_id, owner);

        if Self::spell_requires_stack_target(&adv_card) {
            // Counter-type Adventure: needs a spell on the stack.
            return !self.game.stack.is_empty();
        }
        if Self::spell_requires_battlefield_target(&adv_card) {
            // Targets a permanent: at least one legal permanent must exist.
            return self.game.battlefield.cards.iter().any(|&target_id| {
                self.game
                    .cards
                    .try_get(target_id)
                    .is_some_and(|tc| Self::adventure_permanent_target_ok(&adv_card, tc, owner))
            });
        }
        // Non-permanent-targeting Adventure (e.g. Stomp deals 2 to "any target",
        // which can always hit a player) — offer it. Players always exist.
        true
    }

    /// Whether `target_card` is a plausible permanent target for the Adventure
    /// spell `adv_card`, using the Adventure face's cached target restrictions.
    /// Conservative: matches the type class (creature/land/permanent) so the
    /// offered cast has at least one legal target; precise validity is enforced
    /// at target-selection time by `get_valid_targets_for_spell`.
    fn adventure_permanent_target_ok(
        adv_card: &crate::core::Card,
        target_card: &crate::core::Card,
        spell_owner: crate::core::PlayerId,
    ) -> bool {
        let cache = &adv_card.definition.cache;
        if cache.spell_targets_creature && !target_card.is_creature() {
            return false;
        }
        if cache.spell_targets_land && !target_card.is_land() {
            return false;
        }
        // Permanent-targeting bounce (Petty Theft) restricts to opponent-controlled
        // nonland permanents; require it be controlled by someone other than the
        // caster when the spell isn't a self-target buff. We keep this loose — the
        // exact predicate is enforced at selection time — but reject obviously
        // illegal own-permanent-only cases for opponent-restricted bounces.
        let _ = spell_owner;
        true
    }

    fn spell_requires_stack_target(card: &crate::core::Card) -> bool {
        use crate::core::Effect;

        // Check if any effect is CounterSpell with a placeholder target
        // Placeholder target (CardId(0)) means the spell needs to choose a target when cast
        card.effects
            .iter()
            .any(|effect| matches!(effect, Effect::CounterSpell { target, .. } if target.is_placeholder()))
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
                // DealDamage / DealDamageDynamic: creature-only spells require a creature target.
                // "Any target" spells (Lightning Bolt) can target players, so they don't require
                // a creature to be present. Creature-only spells (Firebending Lesson) do.
                Effect::DealDamage {
                    target: crate::core::TargetRef::None,
                    ..
                }
                | Effect::DealDamageDynamic {
                    target: crate::core::TargetRef::None,
                    ..
                } if (card.definition.cache.spell_targets_creature
                    || card.definition.cache.spell_targets_planeswalker)
                    && !card.definition.cache.spell_targets_any =>
                {
                    true
                }
                _ => false,
            }
        })
    }

    /// Check if a spell requires a graveyard card target (e.g. Regrowth, Reclaim).
    ///
    /// Returns `Some(GraveyardTargetFilter)` when the spell has a
    /// `ReturnGraveyardCardToHand` or `ReturnGraveyardCardToZone` effect so that
    /// `push_castable_spells` can gate the offer on there being at least one
    /// matching card in the relevant graveyard (CR 601.2c).
    ///
    /// `ReturnGraveyardCardToHand` always searches only the caster's own graveyard
    /// (these spells always use `ValidTgts$ Card.YouCtrl` / `Instant.YouCtrl` etc.).
    /// `ReturnGraveyardCardToZone` can search any graveyard (Goryo's Vengeance,
    /// Debtors' Knell) — we conservatively require *some* graveyard to be non-empty.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn spell_requires_graveyard_target(card: &crate::core::Card) -> Option<GraveyardTargetFilter<'_>> {
        use crate::core::Effect;

        for effect in &card.effects {
            match effect {
                Effect::ReturnGraveyardCardToHand { type_filter, .. } => {
                    return Some(GraveyardTargetFilter {
                        type_filter: type_filter.as_str(),
                        // Forge ValidTgts$ Card.YouCtrl — always the caster's own GY.
                        own_only: true,
                    });
                }
                Effect::ReturnGraveyardCardToZone { type_filter, .. } => {
                    return Some(GraveyardTargetFilter {
                        type_filter: type_filter.as_str(),
                        // May target any player's GY (Goryo's Vengeance, etc.).
                        own_only: false,
                    });
                }
                _ => {}
            }
        }
        None
    }
}

/// Descriptor returned by `spell_requires_graveyard_target` that captures which
/// graveyard(s) to search and what card types count as legal targets.
struct GraveyardTargetFilter<'a> {
    /// Comma-separated type filter (e.g. `"Instant,Sorcery"`, `"Card"`).
    /// Empty means any card type is a legal target.
    type_filter: &'a str,
    /// When `true` only the casting player's own graveyard is searched
    /// (`YouCtrl` effects like Regrowth/Reclaim).  When `false` all
    /// graveyards are eligible (e.g. Debtors' Knell / Goryo's Vengeance).
    own_only: bool,
}

impl GraveyardTargetFilter<'_> {
    fn type_matches(&self, card: &crate::core::Card) -> bool {
        if self.type_filter.is_empty() {
            return true;
        }
        self.type_filter.split(',').any(|t| match t.trim() {
            "Instant" => card.is_instant(),
            "Sorcery" => card.is_sorcery(),
            "Creature" => card.is_creature(),
            "Land" => card.is_land(),
            "Artifact" => card.is_artifact(),
            "Enchantment" => card.is_enchantment(),
            "Card" => true,
            _ => false,
        })
    }
}
