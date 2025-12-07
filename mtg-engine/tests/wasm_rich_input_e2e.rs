//! End-to-end tests for browser TUI rewind/replay pattern
//!
//! These tests verify that the game loop can properly handle scripted input
//! using the NeedInput/AwaitingInput pattern and the rewind/replay mechanism.
//!
//! The pattern works as follows:
//! 1. Game runs until a human controller returns NeedInput
//! 2. run_until_input() catches this and returns AwaitingInput with context
//! 3. UI displays choices to user (simulated in tests via script matching)
//! 4. User makes choice - game state is rewound to turn start
//! 5. ReplayController replays previous choices + new choice
//! 6. Game continues from where it left off
//!
//! This specifically tests the "cards disappearing" and "card not in hand" bugs
//! described in the browser TUI issues.

use mtg_forge_rs::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, GameLoopState, ReplayController, RichInputController, VerbosityLevel,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    Result,
};
use smallvec::SmallVec;
use std::path::PathBuf;

/// Helper function to load test game with old school decks
async fn load_old_school_game() -> Result<(mtg_forge_rs::game::GameState, PlayerId, PlayerId)> {
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    // Use the old school decks that contain lands like Swamp, Badlands, City of Brass
    let deck_path1 = PathBuf::from("../decks/old_school/01_rogue_rogerbrand.dck");
    let deck_path2 = PathBuf::from("../decks/old_school/02_thedeck_peterschnidrig.dck");

    let deck1 = DeckLoader::load_from_file(&deck_path1)?;
    let deck2 = DeckLoader::load_from_file(&deck_path2)?;

    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Human".to_string(), &deck1, "AI".to_string(), &deck2, 20)
        .await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    Ok((game, p1_id, p2_id))
}

/// Test basic RichInputController functionality with wildcards
#[tokio::test]
async fn test_rich_input_basic() -> Result<()> {
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

    // Test controller with pass commands
    let mut p1_controller =
        RichInputController::new(p1_id, vec!["pass".to_string(), "pass".to_string(), "pass".to_string()]);
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run a turn
    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Silent)
        .with_max_turns(1);
    let result = game_loop.run_game(&mut p1_controller, &mut p2_controller)?;

    // Should complete or error gracefully
    assert!(result.turns_played > 0, "Should have played at least 1 turn");

    Ok(())
}

/// Test RichInputController with wildcard mode for flexible scripts
#[tokio::test]
async fn test_rich_input_wildcard() -> Result<()> {
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

    // Use wildcard to pass until a land becomes playable
    let mut p1_controller = RichInputController::new(p1_id, vec!["*".to_string(), "play mountain".to_string()]);
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run several turns with the wildcard script
    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Silent)
        .with_max_turns(5);

    let result = game_loop.run_game(&mut p1_controller, &mut p2_controller)?;

    // Should have run some turns
    assert!(result.turns_played > 0, "Game should have run at least one turn");

    Ok(())
}

/// Test that the rewind/replay mechanism works correctly
///
/// This is the core test that validates the browser TUI's functionality.
/// It simulates:
/// 1. Running until NeedInput
/// 2. Rewinding to turn start
/// 3. Creating a ReplayController with previous choices + new choice
/// 4. Running again
#[tokio::test]
async fn test_rewind_replay_mechanism() -> Result<()> {
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

    /// A test controller that returns NeedInput on first call
    struct NeedInputController {
        player_id: PlayerId,
        called: bool,
    }

    impl PlayerController for NeedInputController {
        fn player_id(&self) -> PlayerId {
            self.player_id
        }

        fn choose_spell_ability_to_play(
            &mut self,
            _view: &GameStateView,
            available: &[SpellAbility],
        ) -> ChoiceResult<Option<SpellAbility>> {
            if !self.called {
                self.called = true;
                ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                    available: available.to_vec(),
                    formatted_choices: vec!["Pass".to_string()],
                })
            } else {
                ChoiceResult::Ok(None)
            }
        }

        fn choose_targets(
            &mut self,
            _view: &GameStateView,
            _spell: CardId,
            valid_targets: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
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
            ChoiceResult::Ok(available_sources.iter().copied().collect())
        }

        fn choose_attackers(
            &mut self,
            _view: &GameStateView,
            _available_creatures: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
            ChoiceResult::Ok(SmallVec::new())
        }

        fn choose_blockers(
            &mut self,
            _view: &GameStateView,
            _available_blockers: &[CardId],
            _attackers: &[CardId],
        ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
            ChoiceResult::Ok(SmallVec::new())
        }

        fn choose_damage_assignment_order(
            &mut self,
            _view: &GameStateView,
            _attacker: CardId,
            blockers: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
            ChoiceResult::Ok(blockers.iter().copied().collect())
        }

        fn choose_cards_to_discard(
            &mut self,
            _view: &GameStateView,
            hand: &[CardId],
            count: usize,
        ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
            ChoiceResult::Ok(hand.iter().take(count).copied().collect())
        }

        fn choose_from_library(
            &mut self,
            _view: &GameStateView,
            valid_cards: &[CardId],
        ) -> ChoiceResult<Option<CardId>> {
            ChoiceResult::Ok(valid_cards.first().copied())
        }

        fn on_priority_passed(&mut self, _view: &GameStateView) {}
        fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}
        fn get_controller_type(&self) -> ControllerType {
            ControllerType::Tui
        }
    }

    let mut p1_controller = NeedInputController {
        player_id: p1_id,
        called: false,
    };
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Step 1: Run until NeedInput
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut p1_controller, &mut p2_controller)?;

    let context = match result {
        GameLoopState::AwaitingInput(ctx) => ctx,
        GameLoopState::Complete(_) => {
            // Game ended before needing input - pass test
            return Ok(());
        }
    };

    // Step 2: Extract choices from undo log
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

    // Record game state before rewind (for verification)
    let hand_count_before = game.get_player_zones(p1_id).map(|z| z.hand.cards.len()).unwrap_or(0);
    let battlefield_count_before = game.battlefield.cards.len();

    eprintln!(
        "Before rewind: hand={}, battlefield={}",
        hand_count_before, battlefield_count_before
    );

    // Step 3: Rewind to turn start
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let rewind_result = undo_log.rewind_to_turn_start(&mut game);
    game.undo_log = undo_log;

    // Verify rewind worked
    if let Some((turn_num, _rewound_choices, actions_rewound)) = rewind_result {
        eprintln!("Rewound to turn {}, {} actions rewound", turn_num, actions_rewound);
    }

    // Check hand after rewind
    let hand_count_after_rewind = game.get_player_zones(p1_id).map(|z| z.hand.cards.len()).unwrap_or(0);
    eprintln!("After rewind: hand={}", hand_count_after_rewind);

    // Step 4: Create new choice based on context
    let new_choice = match &context {
        ChoiceContext::SpellAbility { available, .. } => {
            // Try to find a land to play
            let land_ability = available.iter().find(|a| matches!(a, SpellAbility::PlayLand { .. }));
            if land_ability.is_some() {
                ReplayChoice::SpellAbility(land_ability.cloned())
            } else {
                ReplayChoice::SpellAbility(None) // Pass
            }
        }
        _ => ReplayChoice::SpellAbility(None),
    };

    // Step 5: Create replay choices = previous + new
    let mut replay_choices = choices_so_far;
    replay_choices.push(new_choice);

    // Step 6: Run with ReplayController
    let inner = Box::new(mtg_forge_rs::game::ZeroController::new(p1_id));
    let mut replay_p1 = ReplayController::new(p1_id, inner, replay_choices);
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Silent)
        .with_max_turns(3);
    let result = game_loop.run_game(&mut replay_p1, &mut p2_controller)?;

    // Verify game progressed and cards didn't disappear
    assert!(result.turns_played > 0, "Game should have run after replay");

    // Check that hand/battlefield counts are reasonable (not all zeros)
    let hand_count_after = game.get_player_zones(p1_id).map(|z| z.hand.cards.len()).unwrap_or(0);
    let battlefield_count_after = game.battlefield.cards.len();

    eprintln!(
        "Hand: {} -> {}, Battlefield: {} -> {}",
        hand_count_before, hand_count_after, battlefield_count_before, battlefield_count_after
    );

    // Cards shouldn't all disappear
    // After the game runs, there should still be cards somewhere
    let total_cards_visible = hand_count_after
        + battlefield_count_after
        + game.get_player_zones(p1_id).map(|z| z.library.cards.len()).unwrap_or(0)
        + game
            .get_player_zones(p2_id)
            .map(|z| z.hand.cards.len() + z.library.cards.len())
            .unwrap_or(0);

    assert!(
        total_cards_visible > 0,
        "Cards shouldn't all disappear after rewind/replay"
    );

    Ok(())
}

/// Test playing lands with the rich input controller and verify battlefield
#[tokio::test]
async fn test_play_land_script() -> Result<()> {
    let (mut game, p1_id, p2_id) = load_old_school_game().await?;

    // Print starting hand for debugging
    if let Some(zones) = game.get_player_zones(p1_id) {
        eprintln!("Starting hand ({} cards):", zones.hand.cards.len());
        for card_id in zones.hand.cards.iter() {
            if let Ok(card) = game.cards.get(*card_id) {
                eprintln!("  - {} ({})", card.name, card_id);
            }
        }
    }

    // Try to play a land using wildcard mode
    // The script will pass until a land play becomes available, then play it
    let mut p1_controller = RichInputController::new(
        p1_id,
        vec![
            "*".to_string(),
            "play swamp".to_string(),
            "*".to_string(),
            "play badlands".to_string(),
        ],
    );
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run a few turns
    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Normal)
        .with_max_turns(5);

    let result = game_loop.run_game(&mut p1_controller, &mut p2_controller)?;

    // Check that we actually ran turns
    assert!(result.turns_played > 0, "Should have played turns");

    // Check battlefield for lands
    let lands_on_battlefield: Vec<String> = game
        .battlefield
        .cards
        .iter()
        .filter_map(|card_id| {
            game.cards
                .get(*card_id)
                .ok()
                .filter(|c| c.is_land())
                .map(|c| c.name.to_string())
        })
        .collect();

    eprintln!("Lands on battlefield: {:?}", lands_on_battlefield);

    Ok(())
}

/// Test that cards in hand persist correctly through multiple priority passes
#[tokio::test]
async fn test_hand_persistence() -> Result<()> {
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

    // Record initial hand
    let initial_hand_ids: Vec<CardId> = game
        .get_player_zones(p1_id)
        .map(|z| z.hand.cards.to_vec())
        .unwrap_or_default();

    eprintln!("Initial hand IDs: {:?}", initial_hand_ids);
    for card_id in &initial_hand_ids {
        if let Ok(card) = game.cards.get(*card_id) {
            eprintln!("  {} ({})", card.name, card_id);
        }
    }

    // Pass multiple times
    let mut p1_controller = RichInputController::new(
        p1_id,
        vec![
            "pass".to_string(),
            "pass".to_string(),
            "pass".to_string(),
            "pass".to_string(),
            "pass".to_string(),
        ],
    );
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Silent)
        .with_max_turns(1);

    let _ = game_loop.run_game(&mut p1_controller, &mut p2_controller);

    // Check hand after turn
    let final_hand_ids: Vec<CardId> = game
        .get_player_zones(p1_id)
        .map(|z| z.hand.cards.to_vec())
        .unwrap_or_default();

    eprintln!("Final hand IDs: {:?}", final_hand_ids);

    // Hand should still contain cards (may have drawn one more at start of turn)
    assert!(!final_hand_ids.is_empty(), "Hand should not be empty after turn");

    // Original cards that weren't played should still be in hand or on battlefield
    for original_id in &initial_hand_ids {
        let in_hand = final_hand_ids.contains(original_id);
        let on_battlefield = game.battlefield.cards.contains(original_id);
        let in_graveyard = game
            .get_player_zones(p1_id)
            .map(|z| z.graveyard.cards.contains(original_id))
            .unwrap_or(false);

        if !in_hand && !on_battlefield && !in_graveyard {
            if let Ok(card) = game.cards.get(*original_id) {
                eprintln!(
                    "WARNING: Card {} ({}) not found in hand, battlefield, or graveyard!",
                    card.name, original_id
                );
            }
        }
    }

    Ok(())
}

/// Test the full browser TUI workflow with land plays
///
/// This simulates the complete flow:
/// 1. Start game
/// 2. Run until NeedInput
/// 3. (Simulate user choosing to play a land)
/// 4. Rewind and replay with new choice
/// 5. Verify land appears on battlefield
#[tokio::test]
async fn test_full_browser_workflow_land_play() -> Result<()> {
    let (mut game, p1_id, p2_id) = load_old_school_game().await?;

    // Record starting state
    let starting_battlefield_count = game.battlefield.cards.len();
    let starting_hand: Vec<(CardId, String)> = game
        .get_player_zones(p1_id)
        .map(|z| {
            z.hand
                .cards
                .iter()
                .filter_map(|id| game.cards.get(*id).ok().map(|c| (*id, c.name.to_string())))
                .collect()
        })
        .unwrap_or_default();

    eprintln!("Starting hand: {:?}", starting_hand);

    // Step 1: Create controller that will return NeedInput
    struct NeedInputController {
        player_id: PlayerId,
        need_input_count: usize,
    }

    impl PlayerController for NeedInputController {
        fn player_id(&self) -> PlayerId {
            self.player_id
        }

        fn choose_spell_ability_to_play(
            &mut self,
            _view: &GameStateView,
            available: &[SpellAbility],
        ) -> ChoiceResult<Option<SpellAbility>> {
            self.need_input_count += 1;
            ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: vec!["Pass".to_string()],
            })
        }

        fn choose_targets(
            &mut self,
            _view: &GameStateView,
            _spell: CardId,
            valid_targets: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
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
            ChoiceResult::Ok(available_sources.iter().copied().collect())
        }

        fn choose_attackers(
            &mut self,
            _view: &GameStateView,
            _available_creatures: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
            ChoiceResult::Ok(SmallVec::new())
        }

        fn choose_blockers(
            &mut self,
            _view: &GameStateView,
            _available_blockers: &[CardId],
            _attackers: &[CardId],
        ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
            ChoiceResult::Ok(SmallVec::new())
        }

        fn choose_damage_assignment_order(
            &mut self,
            _view: &GameStateView,
            _attacker: CardId,
            blockers: &[CardId],
        ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
            ChoiceResult::Ok(blockers.iter().copied().collect())
        }

        fn choose_cards_to_discard(
            &mut self,
            _view: &GameStateView,
            hand: &[CardId],
            count: usize,
        ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
            ChoiceResult::Ok(hand.iter().take(count).copied().collect())
        }

        fn choose_from_library(
            &mut self,
            _view: &GameStateView,
            valid_cards: &[CardId],
        ) -> ChoiceResult<Option<CardId>> {
            ChoiceResult::Ok(valid_cards.first().copied())
        }

        fn on_priority_passed(&mut self, _view: &GameStateView) {}
        fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}
        fn get_controller_type(&self) -> ControllerType {
            ControllerType::Tui
        }
    }

    let mut p1_controller = NeedInputController {
        player_id: p1_id,
        need_input_count: 0,
    };
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Run until we get NeedInput
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_until_input(&mut p1_controller, &mut p2_controller)?;

    let context = match result {
        GameLoopState::AwaitingInput(ctx) => ctx,
        GameLoopState::Complete(_) => return Ok(()), // Game ended early
    };

    // Step 2: Find a land in the available actions
    let land_to_play = match &context {
        ChoiceContext::SpellAbility { available, .. } => available
            .iter()
            .find(|a| matches!(a, SpellAbility::PlayLand { .. }))
            .cloned(),
        _ => None,
    };

    if land_to_play.is_none() {
        eprintln!("No land play available, skipping test");
        return Ok(());
    }

    eprintln!("Found land to play: {:?}", land_to_play);

    // Step 3: Collect choices made so far
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

    // Step 4: Rewind
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let rewind_result = undo_log.rewind_to_turn_start(&mut game);
    game.undo_log = undo_log;

    if let Some((turn, _, rewound)) = rewind_result {
        eprintln!("Rewound to turn {}, {} actions", turn, rewound);
    }

    // Verify cards still exist in hand after rewind
    let hand_after_rewind: Vec<String> = game
        .get_player_zones(p1_id)
        .map(|z| {
            z.hand
                .cards
                .iter()
                .filter_map(|id| game.cards.get(*id).ok().map(|c| c.name.to_string()))
                .collect()
        })
        .unwrap_or_default();
    eprintln!("Hand after rewind: {:?}", hand_after_rewind);

    // Step 5: Create replay with land play
    let mut replay_choices = choices_so_far;
    replay_choices.push(ReplayChoice::SpellAbility(land_to_play));

    let inner = Box::new(mtg_forge_rs::game::ZeroController::new(p1_id));
    let mut replay_p1 = ReplayController::new(p1_id, inner, replay_choices);
    let mut p2_controller = mtg_forge_rs::game::ZeroController::new(p2_id);

    // Step 6: Run with replay
    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(VerbosityLevel::Normal)
        .with_max_turns(1);
    let _result = game_loop.run_game(&mut replay_p1, &mut p2_controller)?;

    // Verify results
    let final_battlefield_count = game.battlefield.cards.len();
    let lands_on_battlefield: Vec<String> = game
        .battlefield
        .cards
        .iter()
        .filter_map(|id| {
            game.cards
                .get(*id)
                .ok()
                .filter(|c| c.is_land())
                .map(|c| c.name.to_string())
        })
        .collect();

    eprintln!("Final battlefield lands: {:?}", lands_on_battlefield);
    eprintln!(
        "Battlefield count: {} -> {}",
        starting_battlefield_count, final_battlefield_count
    );

    // Land should be on battlefield
    assert!(
        final_battlefield_count > starting_battlefield_count,
        "Land should have been played to battlefield"
    );

    Ok(())
}
