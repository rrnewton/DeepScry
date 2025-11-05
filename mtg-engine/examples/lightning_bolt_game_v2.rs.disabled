//! Lightning Bolt MVP Demo - Controller-Driven Version
//!
//! Demonstrates the game engine with player controllers making decisions.
//! This shows how the engine drives the game through callbacks to the controller.

use mtg_forge_rs::core::{Card, CardType, Color, Effect, ManaCost, TargetRef};
use mtg_forge_rs::game::{
    GameState, GameStateView, PlayerAction, PlayerController, ScriptedController,
};

/// Print the current game state in a readable format
fn print_game_state(game: &GameState, label: &str) {
    println!("\n{label}");

    // Print player life totals
    for player in &game.players {
        let status = if player.has_lost { " (LOST)" } else { "" };
        println!("  {}: {} life{}", player.name, player.life, status);
    }

    // Print battlefield
    let battlefield_count = game.battlefield.cards.len();
    print!("  Battlefield: ");
    if battlefield_count == 0 {
        println!("(empty)");
    } else {
        let cards: Vec<String> = game
            .battlefield
            .cards
            .iter()
            .filter_map(|id| game.cards.get(*id).ok())
            .map(|c| c.name.as_str().to_string())
            .collect();
        println!("{}", cards.join(", "));
    }

    // Print stack
    let stack_count = game.stack.cards.len();
    print!("  Stack: ");
    if stack_count == 0 {
        println!("(empty)");
    } else {
        let cards: Vec<String> = game
            .stack
            .cards
            .iter()
            .filter_map(|id| game.cards.get(*id).ok())
            .map(|c| c.name.as_str().to_string())
            .collect();
        println!("{}", cards.join(", "));
    }

    // Print player zones for first player
    if let Some(player) = game.players.first() {
        if let Some(zones) = game.get_player_zones(player.id) {
            println!("  Alice's hand: {} cards", zones.hand.cards.len());
            println!("  Alice's graveyard: {} cards", zones.graveyard.cards.len());
        }
    }
}

/// Simple game loop that asks the controller for actions
fn run_game_loop(game: &mut GameState, controller: &mut dyn PlayerController, max_actions: usize) {
    let player_id = controller.player_id();

    for action_num in 1..=max_actions {
        // Create a view of the game state
        let view = GameStateView::new(game, player_id);

        // Ask the controller what to do
        // For now, we don't compute available actions - the controller knows what it wants
        let action = controller.choose_action(&view, &[]);

        match action {
            Some(PlayerAction::PlayLand(card_id)) => {
                println!("\n--- Action {action_num}: Play Land ---");
                if let Ok(card) = game.cards.get(card_id) {
                    println!("Alice plays {}", card.name);
                }

                if let Err(e) = game.play_land(player_id, card_id) {
                    println!("  Error: {e:?}");
                } else {
                    print_game_state(game, "After playing land");
                }
            }
            Some(PlayerAction::TapForMana(card_id)) => {
                println!("\n--- Action {action_num}: Tap for Mana ---");
                if let Ok(card) = game.cards.get(card_id) {
                    println!("Alice taps {} for mana", card.name);
                }

                if let Err(e) = game.tap_for_mana(player_id, card_id) {
                    println!("  Error: {e:?}");
                } else {
                    let player = game.get_player(player_id).unwrap();
                    println!("  Mana pool: {} red", player.mana_pool.red);
                }
            }
            Some(PlayerAction::CastSpell {
                card_id,
                targets: _,
            }) => {
                println!("\n--- Action {action_num}: Cast Spell ---");
                if let Ok(card) = game.cards.get(card_id) {
                    println!("Alice casts {}", card.name);
                }

                // Cast the spell
                if let Err(e) = game.cast_spell(player_id, card_id, vec![]) {
                    println!("  Error: {e:?}");
                } else {
                    println!("  Spell is on the stack");
                    print_game_state(game, "After casting spell");

                    // Resolve the spell immediately (no priority passes for now)
                    println!("\nSpell resolves:");
                    if let Err(e) = game.resolve_spell(card_id) {
                        println!("  Error resolving: {e:?}");
                    } else {
                        print_game_state(game, "After spell resolution");
                    }
                }
            }
            Some(PlayerAction::PassPriority) => {
                println!("\n--- Action {action_num}: Pass Priority ---");
                controller.on_priority_passed(&view);
                break; // End of this player's actions
            }
            Some(PlayerAction::DeclareAttacker(_))
            | Some(PlayerAction::DeclareBlocker { .. })
            | Some(PlayerAction::FinishDeclareAttackers)
            | Some(PlayerAction::FinishDeclareBlockers) => {
                println!("\n--- Combat action not supported in this demo ---");
                break;
            }
            None => {
                println!("\n--- No more actions from controller ---");
                break;
            }
        }
    }
}

fn main() {
    println!("=== MTG Forge - Lightning Bolt Controller Demo ===\n");
    println!("This demonstrates the engine-driven architecture");
    println!("where a PlayerController makes decisions.\n");

    // Create a two-player game
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);

    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice = players[0];
    let bob = players[1];

    println!("Players:");
    println!("  Alice (P1): 20 life");
    println!("  Bob (P2): 20 life\n");

    // Setup: Give Alice some cards
    println!("Setup: Giving Alice 3 Mountains and 2 Lightning Bolts");

    let mut mountains = vec![];
    let mut bolts = vec![];

    // Create 3 Mountains for Alice
    for i in 0..3 {
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, format!("Mountain {}", i + 1), alice);
        card.types.push(CardType::Land);
        card.colors.push(Color::Red);
        game.cards.insert(card_id, card);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(alice) {
            zones.hand.add(card_id);
        }

        mountains.push(card_id);
    }

    // Create 2 Lightning Bolts for Alice
    for i in 0..2 {
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, format!("Lightning Bolt {}", i + 1), alice);
        card.types.push(CardType::Instant);
        card.mana_cost = ManaCost::from_string("R");
        card.colors.push(Color::Red);
        card.text = "Lightning Bolt deals 3 damage to any target.".to_string();
        // Add effect that deals 3 damage to target (targeting Bob for this demo)
        card.effects.push(Effect::DealDamage {
            target: TargetRef::Player(bob),
            amount: 3,
        });
        game.cards.insert(card_id, card);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(alice) {
            zones.hand.add(card_id);
        }

        bolts.push(card_id);
    }

    println!("\nAlice's hand: 3 Mountains, 2 Lightning Bolts\n");

    // Create a scripted controller that will execute our sequence
    // The controller now drives the game!
    println!("=== Creating Controller ===");
    println!("Alice's controller will:");
    println!("  1. Play Mountain 1");
    println!("  2. Play Mountain 2");
    println!("  3. Play Mountain 3");
    println!("  4. Tap Mountain 1 for mana");
    println!("  5. Cast Lightning Bolt targeting Bob");
    println!();

    let script = vec![
        PlayerAction::PlayLand(mountains[0]),
        PlayerAction::PlayLand(mountains[1]),
        PlayerAction::PlayLand(mountains[2]),
        PlayerAction::TapForMana(mountains[0]),
        PlayerAction::CastSpell {
            card_id: bolts[0],
            targets: vec![],
        },
    ];

    let mut controller = ScriptedController::new(alice, script);

    // Reset lands played counter before running
    game.get_player_mut(alice).unwrap().reset_lands_played();

    // Run the game loop - the controller drives the game!
    println!("=== Game Loop Starting ===");
    run_game_loop(&mut game, &mut controller, 10);

    // Check game result
    println!("\n=== Game Result ===");
    print_game_state(&game, "Final Game State");

    let alice_life = game.get_player(alice).unwrap().life;
    let bob_life = game.get_player(bob).unwrap().life;

    println!("\nFinal life totals:");
    println!("  Alice: {alice_life}");
    println!("  Bob: {bob_life}");

    if game.is_game_over() {
        if let Some(winner_id) = game.get_winner() {
            let winner = game.get_player(winner_id).unwrap();
            println!("\n{} wins!", winner.name);
        }
    } else {
        println!("\nGame continues...");
    }

    println!("\n=== Controller-Driven Demo Complete ===");
    println!("Key differences from direct-mutation approach:");
    println!("  ✓ Controller makes all decisions via callbacks");
    println!("  ✓ Controller only sees read-only GameStateView");
    println!("  ✓ Game engine executes actions chosen by controller");
    println!("  ✓ Easily swap controllers (AI, UI, scripted)");
}
