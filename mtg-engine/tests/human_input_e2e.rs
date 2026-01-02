//! End-to-end tests for human input handling (WASM-like rewind/replay pattern)
//!
//! These tests verify that the game loop can properly handle human input using
//! the interrupt pattern (NeedInput/AwaitingInput) and the rewind/replay mechanism.
//!
//! The pattern works as follows:
//! 1. Game runs until a human controller returns NeedInput
//! 2. run_until_input() catches this and returns AwaitingInput with context
//! 3. UI displays choices to user (simulated in tests)
//! 4. User makes choice - game state is rewound to turn start
//! 5. ReplayController replays previous choices + new choice
//! 6. Game continues from where it left off

use mtg_forge_rs::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, GameLoopState, ReplayController, VerbosityLevel,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    Result,
};
use smallvec::SmallVec;
use std::path::PathBuf;

/// A test controller that simulates human input by returning NeedInput
/// until a pending choice is set, then returns that choice.
struct TestHumanController {
    player_id: PlayerId,
    /// Pending choice to return (consumed on use)
    pending_spell_ability: Option<Option<SpellAbility>>,
    /// Count how many times we returned NeedInput
    need_input_count: usize,
}

impl TestHumanController {
    fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            pending_spell_ability: None,
            need_input_count: 0,
        }
    }

    fn set_spell_ability_choice(&mut self, choice: Option<SpellAbility>) {
        self.pending_spell_ability = Some(choice);
    }

    fn need_input_count(&self) -> usize {
        self.need_input_count
    }
}

impl PlayerController for TestHumanController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if let Some(choice) = self.pending_spell_ability.take() {
            ChoiceResult::Ok(choice)
        } else {
            self.need_input_count += 1;
            ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: available.iter().map(|_| "test".to_string()).collect(),
            })
        }
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Auto-select first valid target for simplicity
        if valid_targets.is_empty() {
            ChoiceResult::Ok(SmallVec::new())
        } else {
            ChoiceResult::Ok(smallvec::smallvec![valid_targets[0]])
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Auto-select all available sources for simplicity
        ChoiceResult::Ok(available_sources.iter().copied().collect())
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        _available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Don't attack
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        _available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Don't block
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Return blockers in order
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Discard first N cards
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(&mut self, _view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // Select first valid card
        ChoiceResult::Ok(valid_cards.first().copied())
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(valid_permanents.iter().take(count).copied().collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

/// Test that run_until_input properly returns AwaitingInput when controller returns NeedInput
#[tokio::test]
async fn test_run_until_input_returns_awaiting_input() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Human".to_string(), &deck, "AI".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Human controller for P1, Zero for P2
    let mut human = TestHumanController::new(p1_id);
    let mut ai = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run until input is needed
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut human, &mut ai)?;

    // Should be waiting for input
    assert!(
        matches!(result, GameLoopState::AwaitingInput(_)),
        "Expected AwaitingInput, got {:?}",
        result
    );

    // Human controller should have been asked for input at least once
    assert!(
        human.need_input_count() > 0,
        "Human controller should have been asked for input"
    );

    Ok(())
}

/// Test that providing a choice and running again continues the game
#[tokio::test]
async fn test_run_until_input_continues_with_choice() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Human".to_string(), &deck, "AI".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    let mut human = TestHumanController::new(p1_id);
    let mut ai = mtg_forge_rs::game::ZeroController::new(p2_id);

    // First run - should return AwaitingInput
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut human, &mut ai)?;
    assert!(matches!(result, GameLoopState::AwaitingInput(_)));

    // Set choice to "pass" and run again
    human.set_spell_ability_choice(None);

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut human, &mut ai)?;

    // Could be either another AwaitingInput (if more choices needed) or Complete
    // The important thing is it didn't panic
    match result {
        GameLoopState::AwaitingInput(_) => {
            // More input needed - this is valid
        }
        GameLoopState::Complete(game_result) => {
            // Game ended - also valid
            assert!(game_result.turns_played > 0);
        }
    }

    Ok(())
}

/// Test run_one_turn for AI step-through mode
#[tokio::test]
async fn test_run_one_turn_advances_exactly_one_turn() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("AI 1".to_string(), &deck, "AI 2".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    let mut ai1 = mtg_forge_rs::game::ZeroController::new(p1_id);
    let mut ai2 = mtg_forge_rs::game::ZeroController::new(p2_id);

    let initial_turn = game.turn.turn_number;

    // Run exactly one turn
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_one_turn(&mut ai1, &mut ai2)?;

    // Turn number should have advanced by exactly 1 (or game ended)
    match result {
        None => {
            assert_eq!(
                game.turn.turn_number,
                initial_turn + 1,
                "Turn should advance by exactly 1"
            );
        }
        Some(game_result) => {
            // Game ended during first turn (possible with certain setups)
            assert!(game_result.turns_played >= 1);
        }
    }

    Ok(())
}

/// Test ReplayController correctly replays choices
#[tokio::test]
async fn test_replay_controller_replays_choices() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Human".to_string(), &deck, "AI".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Create replay choices that will be played back
    let replay_choices = vec![
        ReplayChoice::SpellAbility(None), // Pass priority
        ReplayChoice::SpellAbility(None), // Pass again
    ];

    // Inner controller to delegate to after replay
    let inner = Box::new(mtg_forge_rs::game::ZeroController::new(p1_id));

    // Create replay controller
    let mut replay = ReplayController::new(p1_id, inner, replay_choices);
    let mut ai2 = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run the game - replay controller should consume its choices then delegate
    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Silent)
        .with_max_turns(5); // Limit turns to avoid infinite game

    let result = game_loop.run_game(&mut replay, &mut ai2)?;

    // Game should complete (with turn limit or player death)
    assert!(result.turns_played > 0, "Game should have run some turns");

    Ok(())
}

/// Test the rewind mechanism - extract choices from undo log
///
/// Note: ChoicePoint actions are only logged when a controller is ASKED to make a choice
/// (i.e., when there are available spell abilities). If no actions are available, the
/// game automatically passes priority without logging a ChoicePoint.
///
/// This test uses a TestHumanController to ensure choices are actually presented.
#[tokio::test]
async fn test_rewind_extracts_choices_from_undo_log() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Human".to_string(), &deck, "AI".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Use TestHumanController which returns NeedInput when there are choices
    let mut human = TestHumanController::new(p1_id);
    let mut ai = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run until human needs to make a choice - this will log choice points
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut human, &mut ai)?;

    // Check that undo log has some actions
    let actions = game.undo_log.actions();
    assert!(!actions.is_empty(), "Undo log should have actions after running");

    // If we got AwaitingInput, the game ran through at least one turn of priority passes
    // The undo log should contain Step/Turn actions at minimum
    match result {
        GameLoopState::AwaitingInput(_) => {
            // We got to a point where a choice was needed.
            // The ChoicePoint for THIS choice hasn't been logged yet (it's pending).
            // But there should be Turn/Step actions in the log.
            assert!(
                actions
                    .iter()
                    .any(|a| matches!(a, mtg_forge_rs::undo::GameAction::AdvanceStep { .. })),
                "Should have AdvanceStep actions in undo log"
            );
        }
        GameLoopState::Complete(_) => {
            // Game completed before any human choices needed
            // Still valid - just check we have game actions
            assert!(!actions.is_empty(), "Should have some actions logged");
        }
    }

    Ok(())
}

/// Test rewind_to_turn_start reverts game state and returns choices
///
/// This test verifies that the undo log's rewind mechanism works correctly.
/// Note: With ZeroController (which auto-passes), the game progresses without
/// any ChoicePoint actions being logged. This test focuses on verifying the
/// rewind mechanism undoes game state correctly.
#[tokio::test]
async fn test_rewind_to_turn_start() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("P1".to_string(), &deck, "P2".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    let mut ai1 = mtg_forge_rs::game::ZeroController::new(p1_id);
    let mut ai2 = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run a couple of turns
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let _ = game_loop.run_one_turn(&mut ai1, &mut ai2)?;

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let _ = game_loop.run_one_turn(&mut ai1, &mut ai2)?;

    // Record state before rewind
    let turn_before = game.turn.turn_number;
    let actions_before = game.undo_log.actions().len();

    // Verify we're at turn 3 or later (turn 1 + 2 runs = turn 3)
    assert!(turn_before >= 3, "Should be at turn 3 or later, got {}", turn_before);
    assert!(actions_before > 0, "Should have actions before rewind");

    // Rewind to turn start
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let result = undo_log.rewind_to_turn_start(&mut game);
    game.undo_log = undo_log;

    // Should find a turn boundary to rewind to
    assert!(
        result.is_some(),
        "Should find turn boundary to rewind to (turn {})",
        turn_before
    );

    let (rewound_turn, _choices, actions_rewound, _log_size) = result.unwrap();

    // Debug output
    eprintln!("Rewound from turn {} to turn {}", turn_before, rewound_turn);
    eprintln!(
        "Actions rewound: {}, actions remaining: {}",
        actions_rewound,
        game.undo_log.actions().len()
    );

    // The rewind returns the turn we rewound TO (which has the ChangeTurn action)
    // actions_rewound is the number of actions popped from the log
    // Note: actions_rewound could be 0 if we're AT the turn boundary
    if actions_rewound > 0 {
        // Should have rewound some actions (at least the step changes within the turn)
        // This means we were in the middle of a turn

        // Turn number in rewind result should match or be less than before
        assert!(
            rewound_turn <= turn_before,
            "Rewound turn {} should be <= turn before {}",
            rewound_turn,
            turn_before
        );

        // Actions in log should be less than before
        assert!(
            game.undo_log.actions().len() < actions_before,
            "Undo log should have fewer actions after rewind ({} vs {})",
            game.undo_log.actions().len(),
            actions_before
        );
    }

    Ok(())
}

/// Test complete rewind/replay cycle simulating WASM flow
#[tokio::test]
async fn test_full_rewind_replay_cycle() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = PathBuf::from("../decks/simple_bolt.dck");
    let deck = DeckLoader::load_from_file(&deck_path)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Human".to_string(), &deck, "AI".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Step 1: Run until human needs to make a choice
    let mut human = TestHumanController::new(p1_id);
    let mut ai = mtg_forge_rs::game::ZeroController::new(p2_id);

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut human, &mut ai)?;

    // Verify we're awaiting input
    let context = match result {
        GameLoopState::AwaitingInput(ctx) => ctx,
        GameLoopState::Complete(_) => {
            // Game ended before needing input - this can happen, just return success
            return Ok(());
        }
    };

    // Step 2: Extract choices made so far from undo log
    let choices_so_far: Vec<ReplayChoice> = game
        .undo_log
        .actions()
        .iter()
        .filter_map(|action| {
            if let mtg_forge_rs::undo::GameAction::ChoicePoint { choice: Some(c), .. } = action {
                Some(c.clone())
            } else {
                None
            }
        })
        .collect();

    // Step 3: Rewind game state to turn start
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let _rewind_result = undo_log.rewind_to_turn_start(&mut game);
    game.undo_log = undo_log;

    // Step 4: Create new choice based on context
    let new_choice = match context {
        ChoiceContext::SpellAbility { .. } => ReplayChoice::SpellAbility(None), // Pass
        _ => ReplayChoice::SpellAbility(None),                                  // Default to pass for test
    };

    // Step 5: Create replay choices = previous choices + new choice
    let mut replay_choices = choices_so_far;
    replay_choices.push(new_choice);

    // Step 6: Create ReplayController and run
    let inner = Box::new(mtg_forge_rs::game::ZeroController::new(p1_id));
    let mut replay = ReplayController::new(p1_id, inner, replay_choices);
    let mut ai = mtg_forge_rs::game::ZeroController::new(p2_id);

    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Silent)
        .with_max_turns(3);
    let result = game_loop.run_game(&mut replay, &mut ai)?;

    // Game should have progressed
    assert!(result.turns_played > 0, "Game should have run after replay");

    Ok(())
}
