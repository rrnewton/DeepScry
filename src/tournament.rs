//! Tournament mode for running multiple games in parallel and collecting statistics
//!
//! This module provides functionality for running MTG tournaments where multiple games
//! are executed concurrently using rayon, with comprehensive statistics collection.

use crate::{
    game::{
        random_controller::RandomController, zero_controller::ZeroController, GameLoop, HeuristicController,
        VerbosityLevel,
    },
    loader::{AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    Result,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task;

/// Controller type for tournament games
#[derive(Debug, Clone, Copy)]
pub enum ControllerType {
    Zero,
    Random,
    Heuristic,
}

/// Statistics collected during tournament
#[derive(Debug, Default)]
struct TournamentStats {
    p1_wins: usize,
    p2_wins: usize,
    draws: usize,
    deck_wins: HashMap<String, usize>,
    deck_games: HashMap<String, usize>,
    matchup_results: HashMap<(String, String), (usize, usize, usize)>, // (p1_wins, p2_wins, draws)
}

/// Run tournament mode - play multiple games in parallel and collect statistics
pub async fn run_tourney(
    deck_paths: Vec<PathBuf>,
    games: Option<usize>,
    seconds: Option<u64>,
    p1_type: ControllerType,
    p2_type: ControllerType,
    seed_resolved: Option<u64>,
) -> Result<()> {
    println!("=== MTG Forge Rust - Tournament Mode ===\n");

    // Validate that we have at least 1 deck
    if deck_paths.is_empty() {
        return Err(crate::MtgError::InvalidAction(
            "Tournament requires at least 1 deck".to_string(),
        ));
    }

    // Validate that either games or seconds is specified
    if games.is_none() && seconds.is_none() {
        return Err(crate::MtgError::InvalidAction(
            "Must specify either --games or --seconds".to_string(),
        ));
    }

    println!("Loading decks...");
    let mut decks = Vec::new();
    for deck_path in &deck_paths {
        let deck = DeckLoader::load_from_file(deck_path)?;
        println!("  {}: {} cards", deck_path.display(), deck.total_cards());
        decks.push((deck_path.clone(), deck));
    }
    println!();

    // Load card database with all unique cards from all decks
    println!("Loading card database...");
    let cardsfolder = PathBuf::from("cardsfolder");
    let card_db = CardDatabase::new(cardsfolder);

    let start = Instant::now();
    let mut all_card_names = std::collections::HashSet::new();
    for (_, deck) in &decks {
        all_card_names.extend(deck.unique_card_names());
    }
    let card_names: Vec<_> = all_card_names.into_iter().collect();
    let (count, _) = card_db.load_cards(&card_names).await?;
    let duration = start.elapsed();
    println!("  Loaded {count} cards in {:.2}ms\n", duration.as_secs_f64() * 1000.0);

    // Determine stopping condition
    let total_games = if let Some(g) = games {
        println!("Running {g} games with {} decks", decks.len());
        g
    } else if let Some(s) = seconds {
        println!("Running for {s} seconds with {} decks", decks.len());
        // Estimate games based on typical game length (will stop when time runs out)
        1_000_000 // Very high number, we'll stop based on time instead
    } else {
        unreachable!("Either games or seconds must be specified");
    };

    if let Some(s) = seed_resolved {
        println!("Using tournament seed: {s}");
    }
    println!("Controllers: P1={:?}, P2={:?}\n", p1_type, p2_type);

    // Statistics tracking (thread-safe)
    let stats = Arc::new(Mutex::new(TournamentStats::default()));
    let start_time = Instant::now();
    let deadline = seconds.map(|s| start_time + Duration::from_secs(s));

    // Use rayon to run games in parallel
    let games_completed = Arc::new(Mutex::new(0usize));

    let card_db = Arc::new(card_db);
    let decks = Arc::new(decks);

    let stats_clone = Arc::clone(&stats);
    let games_completed_clone = Arc::clone(&games_completed);
    let decks_clone = Arc::clone(&decks);

    task::spawn_blocking(move || {
        (0..total_games).into_par_iter().for_each(|game_idx| {
            // Check if we've exceeded time limit
            if let Some(deadline_time) = deadline {
                if Instant::now() >= deadline_time {
                    return; // Skip this game
                }
            }

            // Check if we should still run (for --games mode, could add early termination)
            let completed = {
                let mut count = games_completed_clone.lock().unwrap();
                if games.is_some() && *count >= games.unwrap() {
                    return; // Already completed enough games
                }
                *count += 1;
                *count
            };

            // Select random decks for this game
            let deck_count = decks_clone.len();
            use rand::Rng;
            use rand::SeedableRng;

            // Create a deterministic RNG for deck selection based on master seed + game index
            let deck_rng_seed = seed_resolved.unwrap_or(0).wrapping_add(game_idx as u64);
            let mut deck_rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(deck_rng_seed);

            let deck1_idx = deck_rng.gen_range(0..deck_count);
            let deck2_idx = deck_rng.gen_range(0..deck_count);

            let (deck1_path, deck1) = &decks_clone[deck1_idx];
            let (deck2_path, deck2) = &decks_clone[deck2_idx];

            let card_db_clone = Arc::clone(&card_db);

            // Initialize game (this is async, but we're in a sync context from rayon)
            // Create a new tokio runtime for this thread
            let game_result = tokio::runtime::Runtime::new()
                .expect("Failed to create tokio runtime")
                .block_on(async {
                    let game_init = GameInitializer::new(&card_db_clone);
                    let mut game = game_init
                        .init_game("Player 1".to_string(), deck1, "Player 2".to_string(), deck2, 20)
                        .await?;

                    // Seed the game RNG
                    let game_seed = seed_resolved
                        .unwrap_or(42)
                        .wrapping_add((game_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
                    game.seed_rng(game_seed);

                    // Get player IDs
                    let p1_id = game.get_player_by_idx(0).expect("Should have player 1").id;
                    let p2_id = game.get_player_by_idx(1).expect("Should have player 2").id;

                    // Derive controller seeds
                    let p1_seed = game_seed.wrapping_add(0x1234_5678_9ABC_DEF0);
                    let p2_seed = game_seed.wrapping_add(0xFEDC_BA98_7654_3210);

                    // Create controllers
                    let mut controller1: Box<dyn crate::game::controller::PlayerController> = match p1_type {
                        ControllerType::Zero => Box::new(ZeroController::new(p1_id)),
                        ControllerType::Random => Box::new(RandomController::with_seed(p1_id, p1_seed)),
                        ControllerType::Heuristic => Box::new(HeuristicController::new(p1_id)),
                    };

                    let mut controller2: Box<dyn crate::game::controller::PlayerController> = match p2_type {
                        ControllerType::Zero => Box::new(ZeroController::new(p2_id)),
                        ControllerType::Random => Box::new(RandomController::with_seed(p2_id, p2_seed)),
                        ControllerType::Heuristic => Box::new(HeuristicController::new(p2_id)),
                    };

                    // Run game silently
                    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
                    let result = game_loop.run_game(&mut *controller1, &mut *controller2)?;

                    Ok::<_, crate::MtgError>((result.winner, p1_id, p2_id))
                });

            // Update statistics
            match game_result {
                Ok((winner, p1_id, _p2_id)) => {
                    let mut stats = stats_clone.lock().unwrap();

                    let deck1_name = deck1_path.file_stem().unwrap().to_str().unwrap().to_string();
                    let deck2_name = deck2_path.file_stem().unwrap().to_str().unwrap().to_string();

                    // Update game counts
                    *stats.deck_games.entry(deck1_name.clone()).or_insert(0) += 1;
                    *stats.deck_games.entry(deck2_name.clone()).or_insert(0) += 1;

                    // Update matchup results
                    let matchup_key = if deck1_name <= deck2_name {
                        (deck1_name.clone(), deck2_name.clone())
                    } else {
                        (deck2_name.clone(), deck1_name.clone())
                    };

                    // Update wins
                    match winner {
                        Some(winner_id) => {
                            if winner_id == p1_id {
                                stats.p1_wins += 1;
                                *stats.deck_wins.entry(deck1_name.clone()).or_insert(0) += 1;
                                stats.matchup_results.entry(matchup_key).or_insert((0, 0, 0)).0 += 1;
                            } else {
                                stats.p2_wins += 1;
                                *stats.deck_wins.entry(deck2_name.clone()).or_insert(0) += 1;
                                stats.matchup_results.entry(matchup_key).or_insert((0, 0, 0)).1 += 1;
                            }
                        }
                        None => {
                            stats.draws += 1;
                            stats.matchup_results.entry(matchup_key).or_insert((0, 0, 0)).2 += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Game {} failed: {}", game_idx, e);
                }
            }

            // Print progress every 100 games
            if completed % 100 == 0 {
                println!("Completed {} games", completed);
            }
        });
    })
    .await?;

    let final_count = *games_completed.lock().unwrap();
    let elapsed = start_time.elapsed();

    println!("\n=== Tournament Complete ===");
    println!("Total games played: {}", final_count);
    println!("Elapsed time: {:.2}s", elapsed.as_secs_f64());
    println!("Games per second: {:.2}\n", final_count as f64 / elapsed.as_secs_f64());

    // Display statistics
    let stats = stats.lock().unwrap();

    println!("=== Player Position Statistics ===");
    let total = stats.p1_wins + stats.p2_wins + stats.draws;
    if total > 0 {
        println!(
            "P1 wins: {} ({:.1}%)",
            stats.p1_wins,
            100.0 * stats.p1_wins as f64 / total as f64
        );
        println!(
            "P2 wins: {} ({:.1}%)",
            stats.p2_wins,
            100.0 * stats.p2_wins as f64 / total as f64
        );
        println!(
            "Draws: {} ({:.1}%)",
            stats.draws,
            100.0 * stats.draws as f64 / total as f64
        );
    }

    println!("\n=== Deck Win Rates ===");
    let mut deck_stats: Vec<_> = stats.deck_wins.iter().collect();
    deck_stats.sort_by_key(|(name, _)| *name);
    for (deck_name, wins) in deck_stats {
        let games_played = stats.deck_games.get(deck_name).unwrap_or(&0);
        if *games_played > 0 {
            println!(
                "  {}: {}/{} ({:.1}%)",
                deck_name,
                wins,
                games_played,
                100.0 * *wins as f64 / *games_played as f64
            );
        }
    }

    println!("\n=== Matchup Results ===");
    let mut matchups: Vec<_> = stats.matchup_results.iter().collect();
    matchups.sort_by_key(|&(key, _)| key);
    for ((deck1, deck2), (p1_wins, p2_wins, draws)) in matchups {
        let total_matchup = p1_wins + p2_wins + draws;
        if deck1 == deck2 {
            // Mirror match
            println!("  {} (mirror): {} games", deck1, total_matchup);
            if total_matchup > 0 {
                println!(
                    "    Player 1: {} ({:.1}%)",
                    p1_wins,
                    100.0 * *p1_wins as f64 / total_matchup as f64
                );
                println!(
                    "    Player 2: {} ({:.1}%)",
                    p2_wins,
                    100.0 * *p2_wins as f64 / total_matchup as f64
                );
                if *draws > 0 {
                    println!(
                        "    Draws: {} ({:.1}%)",
                        draws,
                        100.0 * *draws as f64 / total_matchup as f64
                    );
                }
            }
        } else {
            println!("  {} vs {}: {} games", deck1, deck2, total_matchup);
            if total_matchup > 0 {
                println!(
                    "    {} wins: {} ({:.1}%)",
                    deck1,
                    p1_wins,
                    100.0 * *p1_wins as f64 / total_matchup as f64
                );
                println!(
                    "    {} wins: {} ({:.1}%)",
                    deck2,
                    p2_wins,
                    100.0 * *p2_wins as f64 / total_matchup as f64
                );
                if *draws > 0 {
                    println!(
                        "    Draws: {} ({:.1}%)",
                        draws,
                        100.0 * *draws as f64 / total_matchup as f64
                    );
                }
            }
        }
    }

    Ok(())
}
