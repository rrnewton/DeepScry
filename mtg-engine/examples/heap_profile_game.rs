//! Heap Profiling Example
//!
//! Uses DHAT to profile heap allocations during a game
//! Run with: cargo run --release --example heap_profile_game
//! View results at: https://nnethercote.github.io/dh_view/dh_view.html

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use mtg_forge_rs::core::{Card, CardType, Color, Effect, ManaCost, TargetRef};
use mtg_forge_rs::game::{GameLoop, GameState};

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    println!("=== DHAT Heap Profiling - Running Game ===\n");

    // Create a simple game
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);

    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice = players[0];
    let bob = players[1];

    // Create simplified decks - 20 Mountains and 20 Lightning Bolts per player
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

    println!("Created decks: 20 Mountains + 20 Lightning Bolts per player");

    // Draw starting hands (7 cards each)
    for player_id in &[alice, bob] {
        for _ in 0..7 {
            let _ = game.draw_card(*player_id);
        }
    }

    println!("Drew starting hands (7 cards each)");

    // Seed RNG for determinism
    game.seed_rng(42);

    // Create AI controllers
    let mut alice_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(alice, 42);
    let mut bob_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(bob, 42);

    println!("Starting game loop...\n");

    // Run the game for 20 turns to get meaningful profiling data
    let mut game_loop = GameLoop::new(&mut game).with_max_turns(20);

    let result = game_loop
        .run_game(&mut alice_ai, &mut bob_ai)
        .expect("Game loop failed");

    println!("=== Game Complete ===");
    println!("Turns played: {}", result.turns_played);
    println!("End reason: {:?}", result.end_reason);

    if let Some(winner_id) = result.winner {
        let winner_name = game.get_player(winner_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
        println!("Winner: {winner_name}");
    }

    println!("\nFinal life totals:");
    for player in game.players.iter() {
        println!("  - {}: {} life", player.name, player.life);
    }

    println!("\n=== Profiling Complete ===");
    println!("DHAT results saved to: dhat-heap.json");
    println!("View at: https://nnethercote.github.io/dh_view/dh_view.html");
}
