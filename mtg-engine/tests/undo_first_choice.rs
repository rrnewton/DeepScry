//! Test for undoing the first choice in a game
//!
//! This test specifically addresses the scenario where a player:
//! 1. Plays a Forest as their first choice
//! 2. Game progresses to Turn 3
//! 3. Player presses Z to undo
//! 4. Should return to state before first choice with Forest back in hand

use mtg_forge_rs::{
    core::{Card, CardId, CardType, PlayerId, SpellAbility},
    game::{
        controller::{ChoiceResult, GameStateView, PlayerController},
        random_controller::RandomController,
        GameLoop, VerbosityLevel,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    Result,
};
use smallvec::SmallVec;
use std::path::PathBuf;

/// Simple controller that plays one land then requests undo
#[derive(Debug)]
struct UndoFirstChoiceController {
    player_id: PlayerId,
    choices_made: std::cell::RefCell<usize>,
}

impl UndoFirstChoiceController {
    fn new(player_id: PlayerId) -> Self {
        UndoFirstChoiceController {
            player_id,
            choices_made: std::cell::RefCell::new(0),
        }
    }
}

impl PlayerController for UndoFirstChoiceController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        let choices_made = *self.choices_made.borrow();

        println!(
            "  [Controller] choose_spell_ability_to_play called (choices_made: {})",
            choices_made
        );

        if choices_made == 0 {
            // First time - play a land if available
            for ability in available {
                if matches!(ability, SpellAbility::PlayLand { .. }) {
                    *self.choices_made.borrow_mut() = 1;
                    println!("  [Controller] Playing land (first choice)");
                    return ChoiceResult::Ok(Some(ability.clone()));
                }
            }
            // No lands available, pass and increment counter so we don't stay at 0
            *self.choices_made.borrow_mut() = 1;
            ChoiceResult::Ok(None)
        } else if choices_made == 1 {
            // Second time we're asked for a choice - request undo
            println!("  [Controller] Requesting undo to previous choice point");
            *self.choices_made.borrow_mut() = 2; // Prevent infinite undo requests
            ChoiceResult::UndoRequest(usize::MAX)
        } else {
            // After undo, just pass to let test observe the state
            println!("  [Controller] After undo, passing");
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
            return ChoiceResult::Ok(SmallVec::new());
        }
        let mut targets = SmallVec::new();
        targets.push(valid_targets[0]);
        ChoiceResult::Ok(targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        cost: &mtg_forge_rs::core::ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;
        for &source_id in available_sources.iter().take(needed) {
            sources.push(source_id);
        }
        ChoiceResult::Ok(sources)
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

    fn choose_from_library(&mut self, _view: &GameStateView, valid_card_names: &[&str]) -> ChoiceResult<Option<usize>> {
        ChoiceResult::Ok(if valid_card_names.is_empty() { None } else { Some(0) })
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

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Untap everything
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Default to first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> mtg_forge_rs::game::snapshot::ControllerType {
        mtg_forge_rs::game::snapshot::ControllerType::Tui
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        None
    }

    fn has_more_choices(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn test_undo_first_choice_forest() -> Result<()> {
    // Load card database
    let cardsfolder = require_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    // Use simple deck with just forests (integration tests run from mtg-engine/ directory)
    let deck_path = PathBuf::from("../decks/simple_bolt.dck"); // Has mountains, but we'll add forests manually
    let deck = DeckLoader::load_from_file(&deck_path)?;

    // Initialize game
    let game_init = GameInitializer::new(&card_db);
    let mut game = game_init
        .init_game("Player 1".to_string(), &deck, "Player 2".to_string(), &deck, 20)
        .await?;
    game.seed_rng(12345);

    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let p1_id = players[0];
    let p2_id = players[1];

    // Add multiple Forests to P1's hand so they can make choices on multiple turns
    let forest1_id = game.next_card_id();
    let mut forest1 = Card::new(forest1_id, "Forest", p1_id);
    forest1.add_type(CardType::Land);
    game.cards.insert(forest1_id, forest1);

    let forest2_id = game.next_card_id();
    let mut forest2 = Card::new(forest2_id, "Forest", p1_id);
    forest2.add_type(CardType::Land);
    game.cards.insert(forest2_id, forest2);

    let forest3_id = game.next_card_id();
    let mut forest3 = Card::new(forest3_id, "Forest", p1_id);
    forest3.add_type(CardType::Land);
    game.cards.insert(forest3_id, forest3);

    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.hand.add(forest1_id);
        zones.hand.add(forest2_id);
        zones.hand.add(forest3_id);
    }

    // We'll track the first forest for our assertions
    let forest_id = forest1_id;

    println!("\n=== Test: Undo First Choice (Forest) ===");
    println!("Testing the scenario where user plays a Forest, game progresses, then undoes.\n");

    // Capture initial state BEFORE the first choice
    let initial_undo_log_size = game.undo_log.len();
    let initial_turn = game.turn.turn_number;
    let initial_step = game.turn.current_step;
    let initial_p1_hand = game
        .get_player_zones(p1_id)
        .map(|z| z.hand.cards.clone())
        .unwrap_or_default();
    let initial_p1_battlefield_lands = game
        .battlefield
        .cards
        .iter()
        .filter(|&&id| {
            if let Ok(card) = game.cards.get(id) {
                card.owner == p1_id && card.is_land()
            } else {
                false
            }
        })
        .count();

    println!("Initial state (before any choices):");
    println!("  Turn: {}, Step: {:?}", initial_turn, initial_step);
    println!("  Undo log size: {}", initial_undo_log_size);
    println!("  P1 hand: {} cards", initial_p1_hand.len());
    println!("  P1 lands on battlefield: {}", initial_p1_battlefield_lands);
    println!(
        "  Forests in hand: {}",
        initial_p1_hand
            .iter()
            .filter(|&&id| id == forest1_id || id == forest2_id || id == forest3_id)
            .count()
    );
    println!(
        "  First forest (tracking) in hand: {}",
        initial_p1_hand.contains(&forest_id)
    );
    println!(
        "  First forest (tracking) on battlefield: {}",
        game.battlefield.contains(forest_id)
    );

    assert!(
        initial_p1_hand.contains(&forest_id),
        "First forest should start in hand"
    );
    assert!(
        !game.battlefield.contains(forest_id),
        "First forest should not start on battlefield"
    );

    // Create controllers
    let mut controller1 = UndoFirstChoiceController::new(p1_id);
    let mut controller2 = RandomController::with_seed(p2_id, 99999);

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Verbose);

    println!("\n--- Running turns until undo request ---");

    // Run turns until undo is requested (controller will play forest then request undo)
    let mut turns_run = 0;
    const MAX_TURNS: usize = 5;

    while turns_run < MAX_TURNS {
        println!("\nTurn {} before:", game_loop.game.turn.turn_number);
        println!("  Undo log: {}", game_loop.game.undo_log.len());
        println!(
            "  Forest in hand: {}",
            game_loop
                .game
                .get_player_zones(p1_id)
                .map(|z| z.hand.contains(forest_id))
                .unwrap_or(false)
        );
        println!(
            "  Forest on battlefield: {}",
            game_loop.game.battlefield.contains(forest_id)
        );

        let result = game_loop.run_turn_once(&mut controller1, &mut controller2)?;

        if result.is_some() {
            println!("\nGame ended");
            break;
        }

        turns_run += 1;

        // Check if controller has made 2 choices (played land, then requested undo)
        // After undo, the forest should be back in hand
        if *controller1.choices_made.borrow() >= 2 {
            println!("\nUndo should have been processed by now");
            break;
        }
    }

    // Now check the state after undo
    let after_undo_log_size = game_loop.game.undo_log.len();
    let after_undo_turn = game_loop.game.turn.turn_number;
    let after_undo_step = game_loop.game.turn.current_step;
    let after_undo_p1_hand = game_loop
        .game
        .get_player_zones(p1_id)
        .map(|z| z.hand.cards.clone())
        .unwrap_or_default();
    let after_undo_p1_battlefield_lands = game_loop
        .game
        .battlefield
        .cards
        .iter()
        .filter(|&&id| {
            if let Ok(card) = game_loop.game.cards.get(id) {
                card.owner == p1_id && card.is_land()
            } else {
                false
            }
        })
        .count();

    println!("\n=== State After Undo ===");
    println!("  Turn: {}, Step: {:?}", after_undo_turn, after_undo_step);
    println!("  Undo log size: {}", after_undo_log_size);
    println!("  P1 hand: {} cards", after_undo_p1_hand.len());
    println!("  P1 lands on battlefield: {}", after_undo_p1_battlefield_lands);
    println!("  Forest in hand: {}", after_undo_p1_hand.contains(&forest_id));
    println!(
        "  Forest on battlefield: {}",
        game_loop.game.battlefield.contains(forest_id)
    );

    println!("\n=== Verification ===");

    // After undo, the game state was restored to before the first choice (Turn 1 Main1),
    // then the game loop continued executing. The controller passed, Turn 1 ended,
    // and Turn 2 began. So we expect to be at the start of Turn 2.
    // What matters is that the forest was correctly moved back to hand.

    println!(
        "Turn number after undo: {} (started at {}, undo restored to {}, then turn continued to {})",
        after_undo_turn, initial_turn, initial_turn, after_undo_turn
    );
    println!(
        "Forest in hand: {} (should be true)",
        after_undo_p1_hand.contains(&forest_id)
    );
    println!(
        "Forest on battlefield: {} (should be false)",
        game_loop.game.battlefield.contains(forest_id)
    );
    println!(
        "Forests in hand: {}",
        after_undo_p1_hand
            .iter()
            .filter(|&&id| id == forest1_id || id == forest2_id || id == forest3_id)
            .count()
    );

    // The key assertion: the forest should be back in hand
    assert!(
        after_undo_p1_hand.contains(&forest_id),
        "Forest should be back in hand after undo"
    );

    assert!(
        !game_loop.game.battlefield.contains(forest_id),
        "Forest should NOT be on battlefield after undo"
    );

    // Verify all 3 forests are back in hand (the first was played, then undone)
    let forests_in_hand = after_undo_p1_hand
        .iter()
        .filter(|&&id| id == forest1_id || id == forest2_id || id == forest3_id)
        .count();
    assert_eq!(forests_in_hand, 3, "All 3 forests should be back in hand after undo");

    println!("\n✓ Test passed! Undo correctly returned forest to hand.");

    Ok(())
}
