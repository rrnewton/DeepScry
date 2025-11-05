//! AI vs AI Game Example
//!
//! Demonstrates a complete game loop with two AI players
//! Uses RandomController for both players to play a full game

use mtg_forge_rs::core::{Card, CardType, Color, Effect, ManaCost, TargetRef};
use mtg_forge_rs::game::{GameLoop, GameState};
use mtg_forge_rs::loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    println!("=== MTG Forge - AI vs AI Game ===\n");
    println!("Demonstrating:");
    println!("  - Complete game loop with turn phases");
    println!("  - Priority system");
    println!("  - Random AI controllers");
    println!("  - Win condition checking\n");

    // Load card database
    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        eprintln!("Warning: cardsfolder not found, using simplified manual cards");
        run_simplified_game();
        return;
    }

    // Create simple decks (20 Mountains, 40 Lightning Bolts)
    let deck_content = r#"
[Main]
20 Mountain
40 Lightning Bolt
"#;

    let deck = DeckLoader::parse(deck_content).expect("Failed to parse deck");
    println!("Deck configuration:");
    println!("  - {} Mountains", 20);
    println!("  - {} Lightning Bolts", 40);
    println!("  - {} total cards\n", deck.total_cards());

    // Create card database (lazy loading)
    let card_db = CardDatabase::new(cardsfolder);

    // Prefetch deck cards
    println!("Prefetching deck cards...");
    let start = std::time::Instant::now();
    match prefetch_deck_cards(&card_db, &deck).await {
        Ok((count, _)) => {
            let elapsed = start.elapsed();
            println!("Prefetched {} cards in {} ms\n", count, elapsed.as_millis());
        }
        Err(e) => {
            eprintln!("Error prefetching cards: {e}");
            eprintln!("Using simplified manual cards instead\n");
            run_simplified_game();
            return;
        }
    }

    // Initialize game
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game("Alice (AI)".to_string(), &deck, "Bob (AI)".to_string(), &deck, 20)
        .await
        .expect("Failed to initialize game");

    println!("Game initialized!");
    let players: Vec<_> = game.players.iter().map(|p| (p.id, p.name.to_string())).collect();
    println!("  - {}: 20 life", players[0].1);
    println!("  - {}: 20 life\n", players[1].1);

    // Set up effects for Lightning Bolts in both players' libraries
    // (In a real game, this would be done by the card loader)
    setup_lightning_bolt_effects(&mut game, &players);

    // Seed the game RNG for determinism
    game.seed_rng(42);

    // Create AI controllers
    let mut alice_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(players[0].0, 42);
    let mut bob_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(players[1].0, 42);

    println!("=== Starting Game Loop ===\n");

    // Run the game
    let mut game_loop = GameLoop::new(&mut game).with_max_turns(100);

    let result = game_loop
        .run_game(&mut alice_ai, &mut bob_ai)
        .expect("Game loop failed");

    println!("\n=== Game Complete ===");
    println!("Turns played: {}", result.turns_played);
    println!("End reason: {:?}", result.end_reason);

    if let Some(winner_id) = result.winner {
        let winner_name = game.get_player(winner_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
        println!("Winner: {winner_name}");
    } else {
        println!("Game ended in a draw");
    }

    println!("\nFinal life totals:");
    for player in game.players.iter() {
        println!("  - {}: {} life", player.name, player.life);
    }

    println!("\nFinal statistics:");
    println!("  - Total cards in game: {}", game.cards.len());
    println!("  - Cards on battlefield: {}", game.battlefield.cards.len());
    println!("  - Cards on stack: {}", game.stack.cards.len());
}

/// Set up Lightning Bolt effects for all Lightning Bolts in the game
/// In a real implementation, this would be done by the ability parser
fn setup_lightning_bolt_effects(game: &mut GameState, players: &[(mtg_forge_rs::core::PlayerId, String)]) {
    // Get opponent mapping
    let opponent_of = |player_id: mtg_forge_rs::core::PlayerId| {
        if player_id == players[0].0 {
            players[1].0
        } else {
            players[0].0
        }
    };

    // Find all Lightning Bolts and add effects
    let lightning_bolt_ids: Vec<_> = game
        .cards
        .iter()
        .filter_map(|(id, card)| {
            if card.name.as_str().contains("Lightning Bolt") {
                Some((*id, card.owner))
            } else {
                None
            }
        })
        .collect();

    let bolt_count = lightning_bolt_ids.len();

    for (card_id, owner) in lightning_bolt_ids {
        let opponent = opponent_of(owner);
        if let Ok(card) = game.cards.get_mut(card_id) {
            // Add damage effect targeting opponent
            card.effects.push(Effect::DealDamage {
                target: TargetRef::Player(opponent),
                amount: 3,
            });
        }
    }

    println!("Set up effects for {bolt_count} Lightning Bolts\n");
}

/// Run a simplified game without cardsfolder
fn run_simplified_game() {
    println!("Running simplified game with manual card creation...\n");

    let mut game = GameState::new_two_player("Alice (AI)".to_string(), "Bob (AI)".to_string(), 20);

    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice = players[0];
    let bob = players[1];

    // Create simplified decks
    // Each player gets 20 Mountains and 20 Lightning Bolts
    for player_id in &[alice, bob] {
        // Add 20 Mountains to library
        for i in 0..20 {
            let card_id = game.next_card_id();
            let mut card = Card::new(card_id, format!("Mountain {i}"), *player_id);
            card.types.push(CardType::Land);
            card.colors.push(Color::Red);
            game.cards.insert(card_id, card);

            if let Some(zones) = game.get_player_zones_mut(*player_id) {
                zones.library.add(card_id);
            }
        }

        // Add 20 Lightning Bolts to library
        let opponent = if *player_id == alice { bob } else { alice };
        for i in 0..20 {
            let card_id = game.next_card_id();
            let mut card = Card::new(card_id, format!("Lightning Bolt {i}"), *player_id);
            card.types.push(CardType::Instant);
            card.colors.push(Color::Red);
            card.mana_cost = ManaCost::from_string("R");
            card.effects.push(Effect::DealDamage {
                target: TargetRef::Player(opponent),
                amount: 3,
            });
            game.cards.insert(card_id, card);

            if let Some(zones) = game.get_player_zones_mut(*player_id) {
                zones.library.add(card_id);
            }
        }
    }

    println!("Created simplified decks:");
    println!("  - 20 Mountains per player");
    println!("  - 20 Lightning Bolts per player\n");

    // Draw starting hands (7 cards each)
    for player_id in &[alice, bob] {
        for _ in 0..7 {
            let _ = game.draw_card(*player_id);
        }
    }

    let alice_hand = game.get_player_zones(alice).map(|z| z.hand.cards.len()).unwrap_or(0);
    let bob_hand = game.get_player_zones(bob).map(|z| z.hand.cards.len()).unwrap_or(0);
    println!("Starting hands drawn:");
    println!("  - Alice: {alice_hand} cards");
    println!("  - Bob: {bob_hand} cards\n");

    // Seed the game RNG for determinism
    game.seed_rng(42);

    // Create AI controllers
    let mut alice_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(alice, 42);
    let mut bob_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(bob, 42);

    println!("=== Starting Game Loop ===\n");

    // Run the game
    let mut game_loop = GameLoop::new(&mut game).with_max_turns(50);

    let result = game_loop
        .run_game(&mut alice_ai, &mut bob_ai)
        .expect("Game loop failed");

    println!("\n=== Game Complete ===");
    println!("Turns played: {}", result.turns_played);
    println!("End reason: {:?}", result.end_reason);

    if let Some(winner_id) = result.winner {
        let winner_name = game.get_player(winner_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
        println!("Winner: {winner_name}");
    } else {
        println!("Game ended in a draw");
    }

    println!("\nFinal life totals:");
    for player in game.players.iter() {
        println!("  - {}: {} life", player.name, player.life);
    }
}
