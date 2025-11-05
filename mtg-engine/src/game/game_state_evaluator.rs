//! Game state evaluation for heuristic AI
//!
//! This module provides holistic board evaluation, scoring the overall game state
//! from a player's perspective. This is a faithful port of Java Forge's
//! GameStateEvaluator.java which is used by the simulation-based AI.
//!
//! Reference: forge-java/forge-ai/src/main/java/forge/ai/simulation/GameStateEvaluator.java

use crate::core::{Card, PlayerId};
use crate::game::controller::GameStateView;
use crate::game::heuristic_controller::HeuristicController;

/// Score representing the value of a game state
///
/// Reference: GameStateEvaluator.Score (lines 297-320)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    /// Overall board value from AI's perspective
    /// Positive = good for AI, Negative = good for opponent
    /// MAX_VALUE = AI wins, MIN_VALUE = AI loses
    pub value: i32,

    /// Score excluding summon sick creatures
    /// Used to encourage holding creatures until Main Phase 2
    /// if they don't provide immediate value
    pub summon_sick_value: i32,
}

impl Score {
    /// Create a new score with the same value for both metrics
    pub fn new(value: i32) -> Self {
        Score {
            value,
            summon_sick_value: value,
        }
    }

    /// Create a score with different summon sick value
    pub fn with_summon_sick(value: i32, summon_sick_value: i32) -> Self {
        Score {
            value,
            summon_sick_value,
        }
    }

    /// Score indicating the AI has won
    pub const WIN: Score = Score {
        value: i32::MAX,
        summon_sick_value: i32::MAX,
    };

    /// Score indicating the AI has lost
    pub const LOSS: Score = Score {
        value: i32::MIN,
        summon_sick_value: i32::MIN,
    };
}

/// Game state evaluator for heuristic AI
///
/// Provides holistic board evaluation by scoring:
/// - Life totals
/// - Cards in hand
/// - Battlefield permanents (creatures, lands, etc.)
/// - Mana base quality
///
/// Reference: GameStateEvaluator.java
pub struct GameStateEvaluator {
    /// Creature evaluator (reuse from HeuristicController)
    creature_eval: HeuristicController,
}

impl GameStateEvaluator {
    /// Create a new game state evaluator
    pub fn new(player_id: PlayerId) -> Self {
        GameStateEvaluator {
            creature_eval: HeuristicController::new(player_id),
        }
    }

    /// Evaluate the current game state from the AI's perspective
    ///
    /// Returns a Score where:
    /// - Positive values favor the AI
    /// - Negative values favor opponents
    /// - Score::WIN if AI has won
    /// - Score::LOSS if AI has lost
    ///
    /// Reference: GameStateEvaluator.getScoreForGameState() (lines 86-100)
    pub fn evaluate_game_state(&self, view: &GameStateView, ai_player: PlayerId) -> Score {
        // TODO: Check if game is over and return WIN/LOSS

        // TODO: Simulate upcoming combat to see if it's lethal
        // For now, just evaluate the current board state

        self.evaluate_game_state_impl(view, ai_player)
    }

    /// Internal implementation of game state evaluation
    ///
    /// Reference: GameStateEvaluator.getScoreForGameStateImpl() (lines 102-174)
    fn evaluate_game_state_impl(&self, view: &GameStateView, ai_player: PlayerId) -> Score {
        let mut score = 0;

        // Count cards in hand
        // Java: +5 per AI card, -4 per opponent card (lines 108-123)
        let my_hand_size = view.player_hand(ai_player).len() as i32;
        let opponent_hand_size = self.get_opponent_hand_size(view, ai_player);

        score += 5 * my_hand_size - 4 * opponent_hand_size;

        // Life totals
        // Java: +2 per AI life, -2 per opponent life (lines 124-133)
        let my_life = view.life();
        let opponent_life = self.get_opponent_life(view, ai_player);

        score += 2 * my_life;
        score -= 2 * opponent_life;

        // Evaluate mana base quality
        // TODO(mtg-78): Full evalManaBase() port requires deck statistics (AiDeckStatistics)
        // For now, use simplified evaluation that doesn't need deck stats
        let mana_base_value = self.evaluate_mana_base_simplified(view, ai_player);
        score += mana_base_value;

        // Evaluate battlefield permanents
        // Java: Loop through all battlefield cards, evaluate each (lines 148-170)
        let mut summon_sick_score = score;
        let current_turn = view.turn_number();
        let current_step = view.current_step();

        for &card_id in view.battlefield() {
            if let Some(card) = view.get_card(card_id) {
                let value = self.evaluate_card(card);

                // Track summon sickness (vc-3)
                // Reference: GameStateEvaluator.java:153-155
                // If the creature is summon sick and it's before MAIN2, treat it as having 0 value
                // for the summon_sick_score to encourage AI to hold creatures until Main2
                let mut summon_sick_value = value;
                if current_step < crate::game::Step::Main2 && card.is_creature() && card.owner == ai_player {
                    // Check if card entered battlefield this turn (is "sick")
                    if let Some(turn_entered) = card.turn_entered_battlefield {
                        if turn_entered == current_turn {
                            summon_sick_value = 0;
                        }
                    }
                }

                // Cards owned by AI add to score, opponent cards subtract
                if card.owner == ai_player {
                    score += value;
                    summon_sick_score += summon_sick_value;
                } else {
                    score -= value;
                    summon_sick_score -= summon_sick_value;
                }
            }
        }

        Score::with_summon_sick(score, summon_sick_score)
    }

    /// Evaluate mana base quality (simplified version without deck statistics)
    ///
    /// This is a simplified version of Java's evalManaBase() that doesn't require
    /// deck statistics. The full port would need AiDeckStatistics (maxPips, maxCost).
    ///
    /// Reference: GameStateEvaluator.evalManaBase() (lines 176-216)
    fn evaluate_mana_base_simplified(&self, view: &GameStateView, ai_player: PlayerId) -> i32 {
        let mut value = 0;
        let mut total_mana_sources = 0;
        let mut colors_available = [false; 5]; // WUBRG

        // Count mana sources on battlefield
        for &card_id in view.battlefield() {
            if let Some(card) = view.get_card(card_id) {
                // Only count our own mana sources
                if card.owner != ai_player {
                    continue;
                }

                // Check for mana abilities
                for ability in &card.activated_abilities {
                    if !ability.is_mana_ability {
                        continue;
                    }

                    // Check what mana this produces
                    for effect in &ability.effects {
                        if let crate::core::Effect::AddMana { mana, .. } = effect {
                            // Count total mana produced
                            let mana_amount = mana.cmc() as i32;
                            total_mana_sources += mana_amount.max(1); // At least 1 per source

                            // Track colors available (simplified - just check if cost has color)
                            if mana.white > 0 {
                                colors_available[0] = true;
                            }
                            if mana.blue > 0 {
                                colors_available[1] = true;
                            }
                            if mana.black > 0 {
                                colors_available[2] = true;
                            }
                            if mana.red > 0 {
                                colors_available[3] = true;
                            }
                            if mana.green > 0 {
                                colors_available[4] = true;
                            }
                        }
                    }
                }
            }
        }

        // Value mana sources
        // Java awards +100 per mana source up to deck needs, then +5 for excess
        // Without deck stats, we'll use a simple heuristic:
        // - First 6 mana sources: +100 each (typical early/mid game needs)
        // - Next 4 mana sources: +50 each (late game)
        // - Beyond 10: +5 each (excess)
        if total_mana_sources <= 6 {
            value += total_mana_sources * 100;
        } else if total_mana_sources <= 10 {
            value += 600 + (total_mana_sources - 6) * 50;
        } else {
            value += 800 + (total_mana_sources - 10) * 5;
        }

        // Value color fixing
        // Java awards +100 per color pip up to deck needs
        // Without deck stats, award +50 per color available (encouraging fixing)
        let colors_count = colors_available.iter().filter(|&&x| x).count() as i32;
        value += colors_count * 50;

        value
    }

    /// Evaluate a single card on the battlefield
    ///
    /// Reference: GameStateEvaluator.evalCard() (lines 218-238)
    fn evaluate_card(&self, card: &Card) -> i32 {
        if card.is_creature() {
            self.creature_eval.evaluate_creature(card)
        } else if card.is_land() {
            Self::evaluate_land(card)
        } else if card.is_enchantment() {
            // TODO(vc-4): Properly evaluate enchantments
            // Java: Should only provide value based on what they enchant (lines 224-228)
            0
        } else {
            // Other permanents (artifacts, planeswalkers, etc.)
            // Java: 50 + 30 * CMC (lines 232-236)
            let cmc = card.mana_cost.cmc() as i32;
            50 + 30 * cmc
        }
    }

    /// Evaluate a land card
    ///
    /// Reference: GameStateEvaluator.evaluateLand() (lines 240-285)
    pub fn evaluate_land(card: &Card) -> i32 {
        let mut value = 3;

        // Evaluate mana production
        // Java: +100 per mana produced (net after costs), +3 per color
        let mut max_produced = 0;
        let mut colors_produced = std::collections::HashSet::new();

        for ability in &card.activated_abilities {
            if !ability.is_mana_ability {
                continue;
            }

            // Calculate net mana production (mana generated - mana cost to activate)
            let mut mana_generated = 0;
            let mut mana_cost = 0;

            for effect in &ability.effects {
                if let crate::core::Effect::AddMana { mana, .. } = effect {
                    mana_generated += mana.cmc() as i32;

                    // Track colors produced
                    if mana.white > 0 {
                        colors_produced.insert("W");
                    }
                    if mana.blue > 0 {
                        colors_produced.insert("U");
                    }
                    if mana.black > 0 {
                        colors_produced.insert("B");
                    }
                    if mana.red > 0 {
                        colors_produced.insert("R");
                    }
                    if mana.green > 0 {
                        colors_produced.insert("G");
                    }
                    if mana.colorless > 0 {
                        colors_produced.insert("C");
                    }
                }
            }

            // Check for mana cost in activation
            match &ability.cost {
                crate::core::Cost::Mana(cost) => {
                    mana_cost = cost.cmc() as i32;
                }
                crate::core::Cost::TapAndMana(cost) => {
                    mana_cost = cost.cmc() as i32;
                }
                crate::core::Cost::Composite(costs) => {
                    for c in costs {
                        if let crate::core::Cost::Mana(cost) = c {
                            mana_cost += cost.cmc() as i32;
                        } else if let crate::core::Cost::TapAndMana(cost) = c {
                            mana_cost += cost.cmc() as i32;
                        }
                    }
                }
                _ => {}
            }

            let net_produced = mana_generated.saturating_sub(mana_cost);
            max_produced = max_produced.max(net_produced);
        }

        value += 100 * max_produced;
        value += colors_produced.len() as i32 * 3;

        // Evaluate non-mana abilities
        // Java: manlands (+25), sac abilities (+10), repeatable utility (+50)
        for ability in &card.activated_abilities {
            if ability.is_mana_ability {
                continue;
            }

            // Check if it has a tap cost
            let has_tap_cost = match &ability.cost {
                crate::core::Cost::Tap | crate::core::Cost::TapAndMana(_) => true,
                crate::core::Cost::Composite(costs) => costs
                    .iter()
                    .any(|c| matches!(c, crate::core::Cost::Tap | crate::core::Cost::TapAndMana(_))),
                _ => false,
            };

            // Check if it has a sacrifice cost
            let has_sac_cost = match &ability.cost {
                crate::core::Cost::Sacrifice { .. } | crate::core::Cost::SacrificePattern { .. } => true,
                crate::core::Cost::Composite(costs) => costs.iter().any(|c| {
                    matches!(
                        c,
                        crate::core::Cost::Sacrifice { .. } | crate::core::Cost::SacrificePattern { .. }
                    )
                }),
                _ => false,
            };

            if !has_tap_cost {
                // Probably a manland (can activate without tapping)
                // Rate it higher than a rainbow land
                value += 25;
            } else if has_sac_cost {
                // Sacrifice ability, so not repeatable
                // Less good than a utility land that gets you ahead
                value += 10;
            } else {
                // Repeatable utility land with tap cost
                // Probably gets you ahead on board over time
                value += 50;
            }
        }

        // Add value for static abilities
        // Java: +6 per static ability
        // Note: We don't have a static_abilities field on Card yet,
        // but keywords might provide similar value
        // For now, we'll skip this as it requires more infrastructure

        value
    }

    /// Get total hand size for all opponents
    fn get_opponent_hand_size(&self, view: &GameStateView, _ai_player: PlayerId) -> i32 {
        // Sum hand sizes of all opponents (supports multiplayer)
        view.opponents()
            .map(|opp_id| view.player_hand(opp_id).len() as i32)
            .sum()
    }

    /// Get total life for all opponents
    fn get_opponent_life(&self, view: &GameStateView, _ai_player: PlayerId) -> i32 {
        // Use GameStateView's opponent_life() method (supports multiplayer)
        view.opponent_life()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_creation() {
        let score = Score::new(100);
        assert_eq!(score.value, 100);
        assert_eq!(score.summon_sick_value, 100);

        let score2 = Score::with_summon_sick(150, 100);
        assert_eq!(score2.value, 150);
        assert_eq!(score2.summon_sick_value, 100);
    }

    #[test]
    fn test_win_loss_scores() {
        assert_eq!(Score::WIN.value, i32::MAX);
        assert_eq!(Score::LOSS.value, i32::MIN);
    }

    #[test]
    fn test_mana_base_evaluation() {
        use crate::game::controller::GameStateView;
        use crate::game::state::GameState;

        // Create a simple game with two players
        let game = GameState::new_two_player("AI".to_string(), "Opponent".to_string(), 20);
        let player_id = game.players[0].id;

        // Create evaluator and view
        let evaluator = GameStateEvaluator::new(player_id);
        let view = GameStateView::new(&game, player_id);

        // Evaluate mana base (empty battlefield should give 0)
        let mana_value = evaluator.evaluate_mana_base_simplified(&view, player_id);

        // Empty battlefield should have 0 mana base value
        assert_eq!(mana_value, 0);
    }

    #[test]
    fn test_land_evaluation() {
        use crate::core::{ActivatedAbility, Card, CardId, CardType, Cost, Effect, ManaCost, PlayerId};

        let player_id = PlayerId::new(0);

        // Test basic land (Forest: T: Add G)
        let mut forest = Card::new(CardId::new(1), "Forest", player_id);
        forest.types.push(CardType::Land);

        let mut green_mana = ManaCost::new();
        green_mana.green = 1;
        let mana_ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: green_mana,
            }],
            "T: Add G".to_string(),
            true, // is_mana_ability
        );
        forest.activated_abilities.push(mana_ability);

        let forest_value = GameStateEvaluator::evaluate_land(&forest);
        // Base 3 + 100 for 1 mana + 3 for 1 color = 106
        assert_eq!(forest_value, 106);

        // Test dual land (Command Tower: T: Add W or U)
        let mut dual_land = Card::new(CardId::new(2), "Command Tower", player_id);
        dual_land.types.push(CardType::Land);

        let mut white_mana = ManaCost::new();
        white_mana.white = 1;
        let white_ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: white_mana,
            }],
            "T: Add W".to_string(),
            true,
        );

        let mut blue_mana = ManaCost::new();
        blue_mana.blue = 1;
        let blue_ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: blue_mana,
            }],
            "T: Add U".to_string(),
            true,
        );

        dual_land.activated_abilities.push(white_ability);
        dual_land.activated_abilities.push(blue_ability);

        let dual_value = GameStateEvaluator::evaluate_land(&dual_land);
        // Base 3 + 100 for 1 mana + 6 for 2 colors = 109
        assert_eq!(dual_value, 109);

        // Test utility land with tap ability
        let mut utility_land = Card::new(CardId::new(3), "Utility Land", player_id);
        utility_land.types.push(CardType::Land);

        // Add mana ability
        let mut colorless = ManaCost::new();
        colorless.colorless = 1;
        let mana_ab = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: colorless,
            }],
            "T: Add C".to_string(),
            true,
        );
        utility_land.activated_abilities.push(mana_ab);

        // Add utility ability (tap to draw a card)
        let utility_ab = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DrawCards {
                player: player_id,
                count: 1,
            }],
            "T: Draw a card".to_string(),
            false, // not mana ability
        );
        utility_land.activated_abilities.push(utility_ab);

        let utility_value = GameStateEvaluator::evaluate_land(&utility_land);
        // Base 3 + 100 for 1 mana + 3 for 1 color + 50 for utility = 156
        assert_eq!(utility_value, 156);
    }
}
