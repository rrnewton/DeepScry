//! Mana counting, land selection, and mana-source scoring
//!
//! Part of the heuristic AI controller, split out of the former monolithic
//! `heuristic_controller.rs`. See `heuristic_controller/README.md` for the
//! submodule map. This is a pure structural refactor of the Java-Forge AI
//! port — no decision logic changed.

use super::*;

impl HeuristicController {
    /// Count available mana from untapped lands
    ///
    /// This is a simplified count that assumes each untapped land produces 1 mana.
    /// It doesn't account for multi-mana lands or mana dorks, but is sufficient
    /// for early game mana efficiency calculations.
    ///
    /// For accurate mana availability, use the ManaEngine - but for heuristic
    /// purposes this simple count is fast and good enough.
    pub(crate) fn count_available_mana(&self, view: &GameStateView) -> u32 {
        view.battlefield()
            .iter()
            .filter(|&&card_id| {
                if let Some(card) = view.get_card(card_id) {
                    // Count untapped lands we control
                    card.owner == self.player_id && card.is_land() && !view.is_tapped(card_id)
                } else {
                    false
                }
            })
            .count() as u32
    }

    /// Evaluate whether a land should be played
    ///
    /// Reference: AiController.java:1428-1446 (land play decision logic)
    ///
    /// The Java AI uses several checks:
    /// 1. Don't play lands that would deal lethal ETB damage
    /// 2. Sometimes hold land drop for Main 2 (bluffing/deception)
    /// 3. Prioritize playing lands early in the game
    pub(crate) fn should_play_land(&mut self, land_id: CardId, view: &GameStateView) -> bool {
        // If it's not Main Phase 1, always play the land
        let current_step = view.current_step();
        if current_step != crate::game::phase::Step::Main1 {
            return true;
        }

        // Check if it's safe to hold land drop for Main 2 (bluffing/deception)
        if self.is_safe_to_hold_land_for_main2(land_id, view) {
            // Hold the land - don't play it in Main 1
            return false;
        }

        // Otherwise, play the land
        true
    }

    /// Check if it's safe to hold a land drop for Main 2 (bluffing/deception)
    ///
    /// Reference: AiController.isSafeToHoldLandDropForMain2 (lines 1643-1740)
    ///
    /// This is a deception mechanism to hide information from opponents.
    /// The AI sometimes waits until Main 2 to play lands when it doesn't need the mana,
    /// making it harder for opponents to read what's in hand.
    pub(crate) fn is_safe_to_hold_land_for_main2(&mut self, _land_id: CardId, view: &GameStateView) -> bool {
        use rand::Rng;
        // 50% chance to consider holding (matches Java's default HOLD_LAND_DROP_FOR_MAIN2_IF_UNUSED)
        // This prevents the AI from being too predictable
        if !self.rng.gen_bool(0.5) {
            return false;
        }

        // Don't do this on very early turns (too obvious)
        if view.turn_number() <= 2 {
            return false;
        }

        // Get hand
        let hand = view.hand();

        // Count non-land cards in hand
        let nonlands_in_hand: SmallVec<[&Card; 8]> = hand
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| !c.is_land())
            .collect();

        // Calculate minimum CMC of spells in hand (any non-land card)
        let min_cmc = nonlands_in_hand
            .iter()
            .map(|c| u32::from(c.mana_cost.cmc()))
            .min()
            .unwrap_or(0);

        // Calculate available mana (before playing land)
        let current_mana = self.count_available_mana(view);

        // Check if the land would enable casting something
        // Simplified: assume playing the land adds 1 mana
        let mana_with_land = current_mana + 1;
        let can_cast_with_land = min_cmc > 0 && mana_with_land >= min_cmc;
        let cant_cast_now = current_mana < min_cmc;

        // Simplified decision: Hold land if we can't cast anything even with the land drop
        // This is the core bluffing logic - we hold lands when they won't help us cast spells
        // More sophisticated checks (landfall triggers, activated abilities, taplands)
        // can be added later if needed
        if !can_cast_with_land && cant_cast_now {
            // Safe to hold - we won't be able to cast anything anyway
            // Holding hides information from opponents
            return true;
        }

        false
    }

    /// Choose the best land to play from available lands
    ///
    /// Reference: AiController.java:500-724 (chooseBestLandToPlay)
    ///
    /// The Java AI scores lands based on:
    /// 1. Base evaluation score (from GameStateEvaluator.evaluateLand)
    /// 2. +25 points for new basic land types
    /// 3. Color production: (new_colors * 50) / (existing_colors + 1)
    /// 4. Preference for untapped lands when we have spells to cast
    /// 5. Color fixing for one-drops in hand
    ///
    /// For now, simplified version:
    /// - Prefer untapped lands
    /// - Prefer lands that produce colors we need
    /// - TODO: Full scoring algorithm from Java
    pub(crate) fn choose_best_land(&self, _view: &GameStateView, lands: &[CardId]) -> Option<CardId> {
        if lands.is_empty() {
            return None;
        }

        // For now, just return the first land
        // TODO(mtg-XX): Implement full land selection algorithm
        // - Score based on enters-tapped status
        // - Score based on color production vs colors in hand
        // - Score based on new basic land types
        // - Consider mana curve of hand

        Some(lands[0])
    }

    /// Score a mana source by its alternate uses
    ///
    /// Port of Java's ComputerUtilMana.scoreManaProducingCard()
    /// Reference: ComputerUtilMana.java:95-120
    ///
    /// Lower scores = fewer alternate uses = tap first
    /// Higher scores = more valuable for other purposes = preserve
    pub(crate) fn score_mana_source(&self, card: &Card, view: &GameStateView) -> i32 {
        let mut score = 0;

        // Score mana abilities (each mana ability contributes to score)
        // In Java: score += ability.calculateScoreForManaAbility()
        // We'll use a simpler heuristic: count mana produced
        for ability in &card.activated_abilities {
            if ability.is_mana_ability {
                // Simple scoring: +1 per mana type the ability can produce
                for effect in &ability.effects {
                    if let crate::core::Effect::AddMana { mana, .. } = effect {
                        // Count total mana produced
                        score +=
                            i32::from(mana.white + mana.blue + mana.black + mana.red + mana.green + mana.colorless);
                    }
                }
            } else {
                // Non-mana activated abilities add +13 (preserve flexibility)
                // Reference: ComputerUtilMana.java:104-106
                score += 13;
            }
        }

        // Creatures with combat potential add significant score
        // Reference: ComputerUtilMana.java:109-117
        if card.is_creature() {
            // Can attack? (not summoning sick, not tapped)
            let can_attack = self.can_mana_creature_attack(card, view);
            if can_attack {
                score += 13;
            }

            // Can block? (not tapped, has ability to block)
            // Note: Most creatures can block unless they have "can't block"
            // For simplicity, assume all untapped creatures can block
            if !card.tapped {
                score += 13;
            }
        }

        score
    }

    /// Check if a mana creature can attack (simplified check for mana tapping)
    pub(crate) fn can_mana_creature_attack(&self, card: &Card, view: &GameStateView) -> bool {
        // Must be a creature
        if !card.is_creature() {
            return false;
        }

        // Must not be tapped
        if card.tapped {
            return false;
        }

        // Must not have summoning sickness (unless has haste)
        if let Some(turn_entered) = card.turn_entered_battlefield {
            if turn_entered >= view.turn_number() && !card.has_keyword(crate::core::Keyword::Haste) {
                return false;
            }
        }

        // Must not have defender
        if card.has_keyword(crate::core::Keyword::Defender) {
            return false;
        }

        true
    }
}
