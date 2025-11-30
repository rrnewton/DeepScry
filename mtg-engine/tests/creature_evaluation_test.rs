//! Tests for creature evaluation that compare Rust implementation to Java Forge AI
//!
//! These tests verify that the Rust Heuristic AI's creature evaluation scores
//! match (within tolerance) the scores from Java Forge's CreatureEvaluator.
//!
//! Reference: forge-java/forge-ai/src/main/java/forge/ai/CreatureEvaluator.java

use mtg_forge_rs::core::{Card, CardId, CardType, Keyword, PlayerId};
use mtg_forge_rs::game::{controller::GameStateView, state::GameState, HeuristicController};

/// Helper to create a basic creature card for testing with optional keywords
fn create_test_setup_with_keywords(
    name: &str,
    power: i8,
    toughness: i8,
    cmc: u8,
    keywords: Vec<Keyword>,
) -> (GameState, CardId, PlayerId) {
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;

    // Create the creature card
    let card_id = CardId::new(100); // Use a consistent ID for testing
    let mut card = Card::new(card_id, name, player_id);
    card.add_type(CardType::Creature);
    card.set_power(Some(power));
    card.set_toughness(Some(toughness));
    // Set mana cost to generic mana for simplicity
    let mut mana_cost = mtg_forge_rs::core::ManaCost::new();
    mana_cost.generic = cmc;
    card.mana_cost = mana_cost;

    // Add keywords
    for keyword in keywords {
        card.keywords.insert(keyword);
    }

    // Add the card to the game
    game.cards.insert(card_id, card);

    (game, card_id, player_id)
}

/// Helper to create a basic creature card for testing without keywords
fn create_test_setup(name: &str, power: i8, toughness: i8, cmc: u8) -> (GameState, CardId, PlayerId) {
    create_test_setup_with_keywords(name, power, toughness, cmc, vec![])
}

#[test]
fn test_grizzly_bears_evaluation() {
    // Grizzly Bears: 2/2 vanilla creature for 1G (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (2): +10 (2 * 5)
    // Total: 80 + 20 + 30 + 20 + 10 = 160

    let (game, card_id, player_id) = create_test_setup("Grizzly Bears", 2, 2, 2);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 160, "Grizzly Bears should score 160");
}

#[test]
fn test_serra_angel_evaluation() {
    // Serra Angel: 4/4 Flying, Vigilance for 3WW (CMC 5)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (4): +60 (4 * 15)
    // - Toughness (4): +40 (4 * 10)
    // - CMC (5): +25 (5 * 5)
    // - Flying: +40 (power * 10 = 4 * 10)
    // - Vigilance: +60 ((power * 5) + (toughness * 5) = (4*5) + (4*5) = 20 + 20 = 40)
    // Total: 80 + 20 + 60 + 40 + 25 + 40 + 40 = 305

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Serra Angel", 4, 4, 5, vec![Keyword::Flying, Keyword::Vigilance]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Expected: 80 + 20 + 60 + 40 + 25 + 40 + 60 = 325
    // Flying: power * 10 = 4 * 10 = 40
    // Vigilance: (power * 5) + (toughness * 5) = 20 + 20 = 40
    assert_eq!(score, 305, "Serra Angel should score 305");
}

#[test]
fn test_shivan_dragon_evaluation() {
    // Shivan Dragon: 5/5 Flying for 4RR (CMC 6)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (5): +75 (5 * 15)
    // - Toughness (5): +50 (5 * 10)
    // - CMC (6): +30 (6 * 5)
    // - Flying: +50 (power * 10 = 5 * 10)
    // Total: 80 + 20 + 75 + 50 + 30 + 50 = 305

    let (game, card_id, player_id) = create_test_setup_with_keywords("Shivan Dragon", 5, 5, 6, vec![Keyword::Flying]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 305, "Shivan Dragon should score 305");
}

#[test]
fn test_llanowar_elves_evaluation() {
    // Llanowar Elves: 1/1 for G (CMC 1)
    // Has mana ability (adds G) - worth +10
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (1): +15 (1 * 15)
    // - Toughness (1): +10 (1 * 10)
    // - CMC (1): +5 (1 * 5)
    // - Mana ability: +10
    // Total: 80 + 20 + 15 + 10 + 5 + 10 = 140

    let (game, card_id, player_id) = create_test_setup("Llanowar Elves", 1, 1, 1);
    // TODO: Add mana ability support
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // For now, without mana ability tracking: 130
    // With mana ability: 140
    assert_eq!(score, 130, "Llanowar Elves should score 130 (140 with mana ability)");
}

#[test]
fn test_prodigal_sorcerer_evaluation() {
    // Prodigal Sorcerer ("Tim"): 1/1 with activated ability: {T}: Deal 1 damage
    // for 2U (CMC 3)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (1): +15 (1 * 15)
    // - Toughness (1): +10 (1 * 10)
    // - CMC (3): +15 (3 * 5)
    // - Activated ability: +10
    // Total: 80 + 20 + 15 + 10 + 15 + 10 = 150

    let (game, card_id, player_id) = create_test_setup("Prodigal Sorcerer", 1, 1, 3);
    // TODO: Add activated ability support to evaluation
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // For now, without activated ability tracking: 140
    // With activated ability: 150
    assert_eq!(
        score, 140,
        "Prodigal Sorcerer should score 140 (150 with activated ability)"
    );
}

#[test]
fn test_royal_assassin_evaluation() {
    // Royal Assassin: 1/1 with {T}: Destroy target tapped creature
    // for 1BB (CMC 3)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (1): +15 (1 * 15)
    // - Toughness (1): +10 (1 * 10)
    // - CMC (3): +15 (3 * 5)
    // - Activated ability: +10
    // Total: 80 + 20 + 15 + 10 + 15 + 10 = 150

    let (game, card_id, player_id) = create_test_setup("Royal Assassin", 1, 1, 3);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(
        score, 140,
        "Royal Assassin should score 140 (150 with activated ability)"
    );
}

#[test]
fn test_wall_of_omens_evaluation() {
    // Wall of Omens: 0/4 Defender with ETB: Draw a card
    // for 1W (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (0): +0 (0 * 15)
    // - Toughness (4): +40 (4 * 10)
    // - CMC (2): +10 (2 * 5)
    // - Defender: -(0 * 9 + 40) = -40
    // Total: 80 + 20 + 0 + 40 + 10 - 40 = 110

    let (game, card_id, player_id) = create_test_setup_with_keywords("Wall of Omens", 0, 4, 2, vec![Keyword::Defender]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 110, "Wall of Omens should score 110");
}

#[test]
fn test_double_strike_creature() {
    // Boros Swiftblade: 1/2 Double Strike for WR (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (1): +15 (1 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (2): +10 (2 * 5)
    // - Double Strike: +25 (10 + (power * 15) = 10 + (1 * 15) = 10 + 15)
    // Total: 80 + 20 + 15 + 20 + 10 + 25 = 170

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Boros Swiftblade", 1, 2, 2, vec![Keyword::DoubleStrike]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 170, "Boros Swiftblade should score 170");
}

#[test]
fn test_first_strike_creature() {
    // Elite Vanguard with First Strike: 2/1 First Strike for W (CMC 1)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (1): +10 (1 * 10)
    // - CMC (1): +5 (1 * 5)
    // - First Strike: +20 (10 + (power * 5) = 10 + (2 * 5) = 10 + 10)
    // Total: 80 + 20 + 30 + 10 + 5 + 20 = 165

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Elite Vanguard", 2, 1, 1, vec![Keyword::FirstStrike]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 165, "Elite Vanguard with First Strike should score 165");
}

#[test]
fn test_deathtouch_creature() {
    // Typhoid Rats: 1/1 Deathtouch for B (CMC 1)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (1): +15 (1 * 15)
    // - Toughness (1): +10 (1 * 10)
    // - CMC (1): +5 (1 * 5)
    // - Deathtouch: +25
    // Total: 80 + 20 + 15 + 10 + 5 + 25 = 155

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Typhoid Rats", 1, 1, 1, vec![Keyword::Deathtouch]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 155, "Typhoid Rats should score 155");
}

#[test]
fn test_lifelink_creature() {
    // Ajani's Pridemate: 2/2 Lifelink for 1W (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (2): +10 (2 * 5)
    // - Lifelink: +20 (power * 10 = 2 * 10)
    // Total: 80 + 20 + 30 + 20 + 10 + 20 = 180

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Ajani's Pridemate", 2, 2, 2, vec![Keyword::Lifelink]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 180, "Ajani's Pridemate should score 180");
}

#[test]
fn test_trample_creature() {
    // Kalonian Tusker: 3/3 Trample for GG (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (3): +45 (3 * 15)
    // - Toughness (3): +30 (3 * 10)
    // - CMC (2): +10 (2 * 5)
    // - Trample: +10 ((power - 1) * 5 = (3 - 1) * 5 = 2 * 5)
    // Total: 80 + 20 + 45 + 30 + 10 + 10 = 195

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Kalonian Tusker", 3, 3, 2, vec![Keyword::Trample]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 195, "Kalonian Tusker should score 195");
}

#[test]
fn test_menace_creature() {
    // Bloodcrazed Goblin: 2/2 Menace for 1R (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (2): +10 (2 * 5)
    // - Menace: +8 (power * 4 = 2 * 4)
    // Total: 80 + 20 + 30 + 20 + 10 + 8 = 168

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Bloodcrazed Goblin", 2, 2, 2, vec![Keyword::Menace]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 168, "Bloodcrazed Goblin should score 168");
}

#[test]
fn test_reach_creature() {
    // Giant Spider: 2/4 Reach for 3G (CMC 4)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (4): +40 (4 * 10)
    // - CMC (4): +20 (4 * 5)
    // - Reach: +5 (doesn't have flying)
    // Total: 80 + 20 + 30 + 40 + 20 + 5 = 195

    let (game, card_id, player_id) = create_test_setup_with_keywords("Giant Spider", 2, 4, 4, vec![Keyword::Reach]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 195, "Giant Spider should score 195");
}

#[test]
fn test_hexproof_creature() {
    // Slippery Bogle: 1/1 Hexproof for G/U (CMC 1)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (1): +15 (1 * 15)
    // - Toughness (1): +10 (1 * 10)
    // - CMC (1): +5 (1 * 5)
    // - Hexproof: +35
    // Total: 80 + 20 + 15 + 10 + 5 + 35 = 165

    let (game, card_id, player_id) =
        create_test_setup_with_keywords("Slippery Bogle", 1, 1, 1, vec![Keyword::Hexproof]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 165, "Slippery Bogle should score 165");
}

#[test]
fn test_indestructible_creature() {
    // Darksteel Colossus: 11/11 Indestructible, Trample for 11 (CMC 11)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (11): +165 (11 * 15)
    // - Toughness (11): +110 (11 * 10)
    // - CMC (11): +55 (11 * 5)
    // - Trample: +50 ((power - 1) * 5 = (11 - 1) * 5 = 10 * 5)
    // - Indestructible: +70
    // Total: 80 + 20 + 165 + 110 + 55 + 50 + 70 = 550

    let (game, card_id, player_id) = create_test_setup_with_keywords(
        "Darksteel Colossus",
        11,
        11,
        11,
        vec![Keyword::Indestructible, Keyword::Trample],
    );
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 550, "Darksteel Colossus should score 550");
}

#[test]
fn test_shroud_creature() {
    // Troll Ascetic: 3/2 Shroud, Regenerate for 1GG (CMC 3)
    // (Ignoring regenerate for now)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (3): +45 (3 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (3): +15 (3 * 5)
    // - Shroud: +30
    // Total: 80 + 20 + 45 + 20 + 15 + 30 = 210

    let (game, card_id, player_id) = create_test_setup_with_keywords("Troll Ascetic", 3, 2, 3, vec![Keyword::Shroud]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    assert_eq!(score, 210, "Troll Ascetic should score 210");
}

// =========================================================================
// UPKEEP COST PENALTY TESTS
// Reference: CreatureEvaluator.java:235-276
// These tests verify that creatures with recurring costs are properly penalized
// =========================================================================

use mtg_forge_rs::core::KeywordArgs;

/// Helper for creating creatures with complex keywords (those requiring arguments)
fn create_test_setup_with_complex_keywords(
    name: &str,
    power: i8,
    toughness: i8,
    cmc: u8,
    complex_keywords: Vec<KeywordArgs>,
) -> (GameState, CardId, PlayerId) {
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;

    // Create the creature card
    let card_id = CardId::new(100);
    let mut card = Card::new(card_id, name, player_id);
    card.types.push(CardType::Creature);
    card.set_power(Some(power));
    card.set_toughness(Some(toughness));
    let mut mana_cost = mtg_forge_rs::core::ManaCost::new();
    mana_cost.generic = cmc;
    card.mana_cost = mana_cost;

    // Add complex keywords using insert_complex
    for keyword_args in complex_keywords {
        card.keywords.insert_complex(keyword_args);
    }

    game.cards.insert(card_id, card);
    (game, card_id, player_id)
}

#[test]
fn test_cumulative_upkeep_penalty() {
    // Test a creature with Cumulative Upkeep (severe penalty -30)
    // E.g., Krovikan Horror: 2/2 with Cumulative Upkeep: Pay 1 life
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (4): +20 (4 * 5)
    // - Cumulative Upkeep: -30
    // Total: 80 + 20 + 30 + 20 + 20 - 30 = 140

    let (game, card_id, player_id) = create_test_setup_with_complex_keywords(
        "Krovikan Horror",
        2,
        2,
        4,
        vec![KeywordArgs::CumulativeUpkeep {
            cost: mtg_forge_rs::core::ManaCost::new(),
        }],
    );
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Without cumulative upkeep: 170, with penalty: 140
    assert_eq!(score, 140, "Creature with Cumulative Upkeep should have -30 penalty");
}

#[test]
fn test_echo_penalty() {
    // Test a creature with Echo (moderate penalty -10)
    // E.g., Albino Troll: 3/3 with Echo for 1G (CMC 2)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (3): +45 (3 * 15)
    // - Toughness (3): +30 (3 * 10)
    // - CMC (2): +10 (2 * 5)
    // - Echo: -10
    // Total: 80 + 20 + 45 + 30 + 10 - 10 = 175

    let (game, card_id, player_id) = create_test_setup_with_complex_keywords(
        "Albino Troll",
        3,
        3,
        2,
        vec![KeywordArgs::Echo {
            cost: mtg_forge_rs::core::ManaCost::new(),
        }],
    );
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Without echo: 185, with penalty: 175
    assert_eq!(score, 175, "Creature with Echo should have -10 penalty");
}

#[test]
fn test_fading_penalty() {
    // Test a creature with Fading (penalty scales with counters)
    // E.g., Parallax Tide: 2/2 with Fading 5 (starts with 5 fade counters)
    //
    // Since we don't have fade counters set, penalty is -50 (about to die)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (2): +30 (2 * 15)
    // - Toughness (2): +20 (2 * 10)
    // - CMC (4): +20 (4 * 5)
    // - Fading (0 counters): -50
    // Total: 80 + 20 + 30 + 20 + 20 - 50 = 120

    let (game, card_id, player_id) =
        create_test_setup_with_complex_keywords("Parallax Tide", 2, 2, 4, vec![KeywordArgs::Fading { counters: 5 }]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Without fading: 170, with penalty (0 counters on card): 120
    assert_eq!(
        score, 120,
        "Creature with Fading and 0 counters should have -50 penalty"
    );
}

#[test]
fn test_vanishing_penalty() {
    // Test a creature with Vanishing (penalty scales with time counters)
    // E.g., Chronozoa: 3/3 with Vanishing 3
    //
    // Since we don't have time counters set, penalty is -50 (about to die)
    //
    // Expected score calculation:
    // - Base: 80
    // - Non-token: +20
    // - Power (3): +45 (3 * 15)
    // - Toughness (3): +30 (3 * 10)
    // - CMC (4): +20 (4 * 5)
    // - Vanishing (0 counters): -50
    // Total: 80 + 20 + 45 + 30 + 20 - 50 = 145

    let (game, card_id, player_id) =
        create_test_setup_with_complex_keywords("Chronozoa", 3, 3, 4, vec![KeywordArgs::Vanishing { counters: 3 }]);
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Without vanishing: 195, with penalty (0 counters on card): 145
    assert_eq!(
        score, 145,
        "Creature with Vanishing and 0 counters should have -50 penalty"
    );
}

// =========================================================================
// E2E TESTS WITH REAL CARDS FROM DATABASE
// These tests load actual cards from cardsfolder and verify AI evaluation
// =========================================================================

use mtg_forge_rs::loader::CardLoader;
use std::path::PathBuf;

/// Test evaluating real Jötun Grunt (Cumulative Upkeep) from cardsfolder
/// Jötun Grunt: 4/4 for 1W with Cumulative Upkeep
#[test]
fn test_e2e_jotun_grunt_cumulative_upkeep() {
    let path = PathBuf::from("../cardsfolder/j/jotun_grunt.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = CardLoader::load_from_file(&path).expect("Failed to load Jötun Grunt");
    assert_eq!(def.name.as_str(), "Jötun Grunt");

    // Create game and instantiate the card
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;
    let card_id = CardId::new(100);
    let card = def.instantiate(card_id, player_id);

    // Verify Cumulative Upkeep keyword was parsed
    assert!(
        card.has_keyword(Keyword::CumulativeUpkeep),
        "Jötun Grunt should have Cumulative Upkeep keyword"
    );

    // Add card to game
    game.cards.insert(card_id, card);

    // Evaluate
    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Expected calculation:
    // Base: 80 + Non-token: 20 + Power(4)*15: 60 + Toughness(4)*10: 40 + CMC(2)*5: 10 = 210
    // Cumulative Upkeep penalty: -30
    // Total: 180
    assert_eq!(score, 180, "Jötun Grunt (4/4 with Cumulative Upkeep) should score 180");
}

/// Test evaluating real Avalanche Riders (Echo) from cardsfolder
/// Avalanche Riders: 2/2 with Haste for 3R, Echo 3R
#[test]
fn test_e2e_avalanche_riders_echo() {
    let path = PathBuf::from("../cardsfolder/a/avalanche_riders.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = CardLoader::load_from_file(&path).expect("Failed to load Avalanche Riders");
    assert_eq!(def.name.as_str(), "Avalanche Riders");

    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;
    let card_id = CardId::new(100);
    let card = def.instantiate(card_id, player_id);

    // Verify Echo and Haste keywords were parsed
    assert!(
        card.has_keyword(Keyword::Echo),
        "Avalanche Riders should have Echo keyword"
    );
    assert!(
        card.has_keyword(Keyword::Haste),
        "Avalanche Riders should have Haste keyword"
    );

    game.cards.insert(card_id, card);

    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Expected calculation:
    // Base: 80 + Non-token: 20 + Power(2)*15: 30 + Toughness(2)*10: 20 + CMC(4)*5: 20 = 170
    // Note: Haste has no static bonus in creature evaluation (only affects pump spells)
    // Echo penalty: -10
    // Total: 160
    assert_eq!(score, 160, "Avalanche Riders (2/2 Haste with Echo) should score 160");
}

/// Test evaluating real Blastoderm (Fading) from cardsfolder
/// Blastoderm: 5/5 with Shroud for 2GG, Fading 3
#[test]
fn test_e2e_blastoderm_fading() {
    let path = PathBuf::from("../cardsfolder/b/blastoderm.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = CardLoader::load_from_file(&path).expect("Failed to load Blastoderm");
    assert_eq!(def.name.as_str(), "Blastoderm");

    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;
    let card_id = CardId::new(100);
    let card = def.instantiate(card_id, player_id);

    // Verify Fading and Shroud keywords were parsed
    assert!(
        card.has_keyword(Keyword::Fading),
        "Blastoderm should have Fading keyword"
    );
    assert!(
        card.has_keyword(Keyword::Shroud),
        "Blastoderm should have Shroud keyword"
    );

    game.cards.insert(card_id, card);

    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Expected calculation:
    // Base: 80 + Non-token: 20 + Power(5)*15: 75 + Toughness(5)*10: 50 + CMC(4)*5: 20 = 245
    // Shroud: +30
    // Fading penalty (0 counters on card, so -50 "about to die"): -50
    // Total: 225
    assert_eq!(
        score, 225,
        "Blastoderm (5/5 Shroud with Fading, 0 counters) should score 225"
    );
}

/// Test evaluating real Keldon Marauders (Vanishing) from cardsfolder
/// Keldon Marauders: 3/3 for 1R, Vanishing 2
#[test]
fn test_e2e_keldon_marauders_vanishing() {
    let path = PathBuf::from("../cardsfolder/k/keldon_marauders.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = CardLoader::load_from_file(&path).expect("Failed to load Keldon Marauders");
    assert_eq!(def.name.as_str(), "Keldon Marauders");

    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;
    let card_id = CardId::new(100);
    let card = def.instantiate(card_id, player_id);

    // Verify Vanishing keyword was parsed
    assert!(
        card.has_keyword(Keyword::Vanishing),
        "Keldon Marauders should have Vanishing keyword"
    );

    game.cards.insert(card_id, card);

    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Expected calculation:
    // Base: 80 + Non-token: 20 + Power(3)*15: 45 + Toughness(3)*10: 30 + CMC(2)*5: 10 = 185
    // Vanishing penalty (0 counters on card, so -50 "about to die"): -50
    // Total: 135
    assert_eq!(
        score, 135,
        "Keldon Marauders (3/3 with Vanishing, 0 counters) should score 135"
    );
}

/// Test evaluating classic 4ED cards from cardsfolder (Serra Angel, Shivan Dragon)
/// These test the baseline creature evaluation with Flying and Vigilance
#[test]
fn test_e2e_serra_angel_classic() {
    let path = PathBuf::from("../cardsfolder/s/serra_angel.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = CardLoader::load_from_file(&path).expect("Failed to load Serra Angel");
    assert_eq!(def.name.as_str(), "Serra Angel");

    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;
    let card_id = CardId::new(100);
    let card = def.instantiate(card_id, player_id);

    // Verify keywords
    assert!(card.has_keyword(Keyword::Flying), "Serra Angel should have Flying");
    assert!(
        card.has_keyword(Keyword::Vigilance),
        "Serra Angel should have Vigilance"
    );

    game.cards.insert(card_id, card);

    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Expected: same as unit test
    // Base: 80 + Non-token: 20 + P(4)*15: 60 + T(4)*10: 40 + CMC(5)*5: 25 + Flying(4*10): 40 + Vigilance(4*5+4*5): 40 = 305
    assert_eq!(score, 305, "Serra Angel (4/4 Flying Vigilance) should score 305");
}

/// Test evaluating Shivan Dragon from cardsfolder
#[test]
fn test_e2e_shivan_dragon_classic() {
    let path = PathBuf::from("../cardsfolder/s/shivan_dragon.txt");
    if !path.exists() {
        println!("Skipping test: cardsfolder not present");
        return;
    }

    let def = CardLoader::load_from_file(&path).expect("Failed to load Shivan Dragon");
    assert_eq!(def.name.as_str(), "Shivan Dragon");

    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let player_id = game.players[0].id;
    let card_id = CardId::new(100);
    let card = def.instantiate(card_id, player_id);

    assert!(card.has_keyword(Keyword::Flying), "Shivan Dragon should have Flying");

    game.cards.insert(card_id, card);

    let controller = HeuristicController::new(player_id);
    let view = GameStateView::new(&game, player_id);
    let score = controller.evaluate_creature(&view, card_id);

    // Base: 80 + Non-token: 20 + P(5)*15: 75 + T(5)*10: 50 + CMC(6)*5: 30 + Flying(5*10): 50 = 305
    assert_eq!(score, 305, "Shivan Dragon (5/5 Flying) should score 305");
}
