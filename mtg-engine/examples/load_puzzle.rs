//! Example: Load a puzzle from a PZL file
//!
//! This demonstrates loading a puzzle file and creating a game state from it.

use mtg_forge_rs::loader::AsyncCardDatabase;
use mtg_forge_rs::puzzle::{load_puzzle_into_game, PuzzleFile};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> mtg_forge_rs::Result<()> {
    // Parse a simple inline puzzle
    let puzzle_contents = r#"
[metadata]
Name:Simple Test Puzzle
Goal:Win
Turns:1
Difficulty:Easy
Description:A simple test puzzle with basic cards

[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0battlefield=Mountain;Mountain;Mountain
p1life=3
p1battlefield=Grizzly Bears
"#;

    println!("Parsing puzzle file...");
    let puzzle = PuzzleFile::parse(puzzle_contents)?;

    println!("Puzzle: {}", puzzle.metadata.name);
    println!("Goal: {:?}", puzzle.metadata.goal);
    println!("Difficulty: {:?}", puzzle.metadata.difficulty);
    println!("Turn limit: {}", puzzle.metadata.turns);
    println!();

    // Create card database
    let cardsfolder = PathBuf::from("cardsfolder");
    println!("Loading card database from {:?}...", cardsfolder);
    let card_db = Arc::new(AsyncCardDatabase::new(cardsfolder));

    // Load the puzzle into a game state
    println!("Loading puzzle into game state...");
    let game = load_puzzle_into_game(&puzzle, &card_db).await?;

    // Display game state
    println!("\n=== Game State ===");
    println!("Turn: {}", game.turn.turn_number);
    println!("Phase: {:?}", game.turn.current_step);
    println!("Active player: {:?}", game.turn.active_player);
    println!();

    println!("Player 1: {} life", game.players[0].life);
    if let Some(zones) = game.get_player_zones(game.players[0].id) {
        println!("  Hand: {} cards", zones.hand.len());
        println!("  Library: {} cards", zones.library.len());
        println!("  Graveyard: {} cards", zones.graveyard.len());
    }
    println!();

    println!("Player 2: {} life", game.players[1].life);
    if let Some(zones) = game.get_player_zones(game.players[1].id) {
        println!("  Hand: {} cards", zones.hand.len());
        println!("  Library: {} cards", zones.library.len());
        println!("  Graveyard: {} cards", zones.graveyard.len());
    }
    println!();

    println!("Battlefield: {} permanents", game.battlefield.len());
    for card_id in &game.battlefield.cards {
        if let Ok(card) = game.cards.get(*card_id) {
            println!(
                "  - {} ({})",
                card.name,
                if card.tapped { "tapped" } else { "untapped" }
            );
            if let (Some(p), Some(t)) = (card.power, card.toughness) {
                println!("    {}/{}", p, t);
            }
        }
    }

    println!("\nPuzzle loaded successfully!");

    Ok(())
}
