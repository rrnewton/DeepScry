//! MTG Forge Rust - Main Binary
//!
//! Text-based Magic: The Gathering game engine with TUI support

use clap::{Parser, Subcommand, ValueEnum};
use mtg_forge_rs::{
    game::{
        random_controller::RandomController, zero_controller::ZeroController, GameLoop, GameSnapshot,
        HeuristicController, InteractiveController, RichInputController, StopCondition, VerbosityLevel,
    },
    loader::{AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
    Result,
};
use std::path::PathBuf;

/// Controller type for AI agents
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ControllerType {
    /// Always chooses first meaningful action (for testing)
    Zero,
    /// Makes random choices
    Random,
    /// Text UI controller for human play via stdin
    Tui,
    /// Heuristic AI controller with strategic decision making
    Heuristic,
    /// Fixed script controller with predetermined choices (requires --fixed-inputs)
    Fixed,
}

/// Verbosity level for game output (custom parser supporting both names and numbers)
#[derive(Debug, Clone, Copy)]
struct VerbosityArg(VerbosityLevel);

impl std::str::FromStr for VerbosityArg {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "silent" | "0" => Ok(VerbosityArg(VerbosityLevel::Silent)),
            "minimal" | "1" => Ok(VerbosityArg(VerbosityLevel::Minimal)),
            "normal" | "2" => Ok(VerbosityArg(VerbosityLevel::Normal)),
            "verbose" | "3" => Ok(VerbosityArg(VerbosityLevel::Verbose)),
            _ => Err(format!(
                "invalid verbosity level '{s}' (expected: silent/0, minimal/1, normal/2, verbose/3)"
            )),
        }
    }
}

impl From<VerbosityArg> for VerbosityLevel {
    fn from(arg: VerbosityArg) -> Self {
        arg.0
    }
}

/// Seed value that can be either a specific u64 or "from_entropy"
///
/// This is the ONLY place in the codebase where system entropy is accessed.
/// All other code must use explicit seeds for deterministic behavior.
#[derive(Debug, Clone, Copy)]
enum SeedArg {
    /// Use a specific seed value for deterministic behavior
    Value(u64),
    /// Generate seed from system entropy (non-deterministic)
    FromEntropy,
}

impl std::str::FromStr for SeedArg {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.to_lowercase() == "from_entropy" {
            Ok(SeedArg::FromEntropy)
        } else {
            s.parse::<u64>()
                .map(SeedArg::Value)
                .map_err(|_| format!("invalid seed '{s}' (expected: u64 number or 'from_entropy')"))
        }
    }
}

impl SeedArg {
    /// Resolve the seed to a u64 value
    ///
    /// This is the ONLY method that calls from_entropy() in the entire codebase.
    /// It should only be called when the user explicitly requests it via CLI.
    fn resolve(self) -> u64 {
        match self {
            SeedArg::Value(v) => v,
            SeedArg::FromEntropy => {
                use rand::SeedableRng;
                let rng = rand_xoshiro::Xoshiro256PlusPlus::from_entropy();
                // Extract a u64 from the RNG state
                use rand::Rng;
                let mut temp_rng = rng;
                temp_rng.gen()
            }
        }
    }
}

#[derive(Parser)]
#[command(name = "mtg")]
#[command(about = "MTG Forge Rust - Magic: The Gathering Game Engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Text UI Mode - Interactive Forge Gameplay
    Tui {
        /// Deck file (.dck) for player 1 (required unless --start-state or --start-from is provided)
        #[arg(value_name = "PLAYER1_DECK", required_unless_present_any = ["start_state", "start_from"])]
        deck1: Option<PathBuf>,

        /// Deck file (.dck) for player 2 (optional; if omitted, uses PLAYER1_DECK for both players)
        #[arg(value_name = "PLAYER2_DECK")]
        deck2: Option<PathBuf>,

        /// Load game state from puzzle file (.pzl)
        #[arg(long, value_name = "PUZZLE_FILE")]
        start_state: Option<PathBuf>,

        /// Player 1 controller type (default: human TUI)
        #[arg(long, value_enum, default_value = "tui")]
        p1: ControllerType,

        /// Player 2 controller type (default: heuristic AI)
        #[arg(long, value_enum, default_value = "heuristic")]
        p2: ControllerType,

        /// Player 1 name (default: Alice)
        #[arg(long, default_value = "Alice")]
        p1_name: String,

        /// Player 2 name (default: Bob)
        #[arg(long, default_value = "Bob")]
        p2_name: String,

        /// Fixed script input for player 1 (space or comma separated indices, e.g., "1 1 2" or "1,1,2")
        #[arg(long, value_name = "CHOICES")]
        p1_fixed_inputs: Option<String>,

        /// Fixed script input for player 2 (space or comma separated indices, e.g., "1 1 2" or "1,1,2")
        #[arg(long, value_name = "CHOICES")]
        p2_fixed_inputs: Option<String>,

        /// Set random seed for deterministic testing (master seed for engine and controller defaults)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        seed: Option<SeedArg>,

        /// Set random seed for Player 1 controller (overrides seed-derived default)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        seed_p1: Option<SeedArg>,

        /// Set random seed for Player 2 controller (overrides seed-derived default)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        seed_p2: Option<SeedArg>,

        /// Load all cards from cardsfolder (default: only load cards in decks)
        #[arg(long)]
        load_all_cards: bool,

        /// Verbosity level for game output (0=silent, 1=minimal, 2=normal, 3=verbose)
        #[arg(long, default_value = "normal", short = 'v')]
        verbosity: VerbosityArg,

        /// Use numeric-only choice format (for comparison with Java Forge)
        #[arg(long)]
        numeric_choices: bool,

        /// Enable state hash debugging (prints hash before each action)
        #[arg(long)]
        debug_state_hash: bool,

        /// Stop after N choices by specified player(s) and save snapshot
        /// Format: <NUM>[:[p1|p2]]
        /// Examples: 3 (both players), 1:p1 (only p1), 5:p2 (only p2)
        #[arg(long, value_name = "CONDITION")]
        stop_on_choice: Option<String>,

        /// Stop and save snapshot when fixed controller script is exhausted
        /// (useful for building reproducers incrementally)
        #[arg(long)]
        stop_when_fixed_exhausted: bool,

        /// Output file for game snapshot (default: game.snapshot)
        #[arg(long, value_name = "FILE", default_value = "game.snapshot")]
        snapshot_output: PathBuf,

        /// Load and resume game from snapshot file
        #[arg(long, value_name = "FILE")]
        start_from: Option<PathBuf>,

        /// Save final game state when game ends (for determinism testing)
        #[arg(long, value_name = "FILE")]
        save_final_gamestate: Option<PathBuf>,

        /// Only print the last K lines of log output at game exit
        /// (useful with --stop-on-choice to see constant-sized output)
        #[arg(long, value_name = "K")]
        log_tail: Option<usize>,

        /// Controlled initial hand for Player 1 (semicolon-separated card names, 1-7 cards)
        /// Example: "Mountain;Lightning Bolt;Mountain"
        #[arg(long, value_name = "CARDS")]
        p1_draw: Option<String>,

        /// Controlled initial hand for Player 2 (semicolon-separated card names, 1-7 cards)
        /// Example: "Island;Counterspell;Island"
        #[arg(long, value_name = "CARDS")]
        p2_draw: Option<String>,
    },

    /// Run games for profiling (use with cargo-heaptrack or cargo-flamegraph)
    Profile {
        /// Number of games to run
        #[arg(long, short = 'g', default_value_t = 1000)]
        games: usize,

        /// Random seed for deterministic profiling
        #[arg(long, default_value_t = 42)]
        seed: u64,

        /// Deck file to use (uses same deck for both players)
        #[arg(long, short = 'd', default_value = "decks/simple_bolt.dck")]
        deck: PathBuf,
    },

    /// Tournament Mode - Run multiple games in parallel and collect statistics
    Tourney {
        /// Deck files to include in tournament (at least 1 required)
        #[arg(value_name = "DECKS", required = true, num_args = 1..)]
        decks: Vec<PathBuf>,

        /// Total number of games to run (mutually exclusive with --seconds)
        #[arg(long, short = 'g', conflicts_with = "seconds")]
        games: Option<usize>,

        /// Run for N seconds (mutually exclusive with --games)
        #[arg(long, short = 's', conflicts_with = "games")]
        seconds: Option<u64>,

        /// Player 1 controller type for all games
        #[arg(long, value_enum, default_value = "heuristic")]
        p1: ControllerType,

        /// Player 2 controller type for all games
        #[arg(long, value_enum, default_value = "heuristic")]
        p2: ControllerType,

        /// Random seed for deterministic tournament
        #[arg(long)]
        seed: Option<SeedArg>,
    },

    /// Resume a saved game from snapshot
    ///
    /// By default, restores everything from the snapshot: game state, controller types,
    /// controller RNG states, and intra-turn choices. Use --override flags to replace
    /// controllers or seeds with new values.
    Resume {
        /// Snapshot file to resume from (.snapshot)
        #[arg(value_name = "SNAPSHOT_FILE")]
        snapshot_file: PathBuf,

        /// Override Player 1 controller (default: restore from snapshot)
        #[arg(long, value_enum)]
        override_p1: Option<ControllerType>,

        /// Override Player 2 controller (default: restore from snapshot)
        #[arg(long, value_enum)]
        override_p2: Option<ControllerType>,

        /// Fixed script input for player 1 (required if --override-p1=fixed)
        #[arg(long, value_name = "CHOICES")]
        p1_fixed_inputs: Option<String>,

        /// Fixed script input for player 2 (required if --override-p2=fixed)
        #[arg(long, value_name = "CHOICES")]
        p2_fixed_inputs: Option<String>,

        /// Override game engine seed (default: restore from snapshot)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        override_seed: Option<SeedArg>,

        /// Override Player 1 controller seed (default: restore from snapshot)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        override_seed_p1: Option<SeedArg>,

        /// Override Player 2 controller seed (default: restore from snapshot)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        override_seed_p2: Option<SeedArg>,

        /// Verbosity level for game output (0=silent, 1=minimal, 2=normal, 3=verbose)
        #[arg(long, default_value = "normal", short = 'v')]
        verbosity: VerbosityArg,

        /// Use numeric-only choice format (for comparison with Java Forge)
        #[arg(long)]
        numeric_choices: bool,

        /// Enable state hash debugging (prints hash before each action)
        #[arg(long)]
        debug_state_hash: bool,

        /// Stop after N choices by specified player(s) and save snapshot
        /// Format: <NUM>[:[p1|p2]]
        /// Examples: 3 (both players), 1:p1 (only p1), 5:p2 (only p2)
        #[arg(long, value_name = "CONDITION")]
        stop_on_choice: Option<String>,

        /// Stop and save snapshot when fixed controller script is exhausted
        /// (useful for building reproducers incrementally)
        #[arg(long)]
        stop_when_fixed_exhausted: bool,

        /// Output file for game snapshot (default: game.snapshot)
        #[arg(long, value_name = "FILE", default_value = "game.snapshot")]
        snapshot_output: PathBuf,

        /// Save final game state when game ends (for determinism testing)
        #[arg(long, value_name = "FILE")]
        save_final_gamestate: Option<PathBuf>,

        /// Only print the last K lines of log output at game exit
        /// (useful with --stop-on-choice to see constant-sized output)
        #[arg(long, value_name = "K")]
        log_tail: Option<usize>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Tui {
            deck1,
            deck2,
            start_state,
            p1,
            p2,
            p1_name,
            p2_name,
            p1_fixed_inputs,
            p2_fixed_inputs,
            seed,
            seed_p1,
            seed_p2,
            load_all_cards,
            verbosity,
            numeric_choices,
            debug_state_hash,
            stop_on_choice,
            stop_when_fixed_exhausted,
            snapshot_output,
            start_from,
            save_final_gamestate,
            log_tail,
            p1_draw,
            p2_draw,
        } => {
            run_tui(
                deck1,
                deck2,
                start_state,
                p1,
                p2,
                p1_name,
                p2_name,
                p1_fixed_inputs,
                p2_fixed_inputs,
                seed,
                seed_p1,
                seed_p2,
                load_all_cards,
                verbosity,
                numeric_choices,
                debug_state_hash,
                stop_on_choice,
                stop_when_fixed_exhausted,
                snapshot_output,
                start_from,
                save_final_gamestate,
                log_tail,
                p1_draw,
                p2_draw,
            )
            .await?
        }
        Commands::Profile { games, seed, deck } => run_profile(games, seed, deck).await?,
        Commands::Tourney {
            decks,
            games,
            seconds,
            p1,
            p2,
            seed,
        } => {
            // Convert ControllerType to tournament::ControllerType
            let p1_tourney = match p1 {
                ControllerType::Zero => mtg_forge_rs::tournament::ControllerType::Zero,
                ControllerType::Random => mtg_forge_rs::tournament::ControllerType::Random,
                ControllerType::Heuristic => mtg_forge_rs::tournament::ControllerType::Heuristic,
                _ => {
                    return Err(mtg_forge_rs::MtgError::InvalidAction(
                        "Tournament mode only supports Zero, Random, and Heuristic controllers".to_string(),
                    ))
                }
            };
            let p2_tourney = match p2 {
                ControllerType::Zero => mtg_forge_rs::tournament::ControllerType::Zero,
                ControllerType::Random => mtg_forge_rs::tournament::ControllerType::Random,
                ControllerType::Heuristic => mtg_forge_rs::tournament::ControllerType::Heuristic,
                _ => {
                    return Err(mtg_forge_rs::MtgError::InvalidAction(
                        "Tournament mode only supports Zero, Random, and Heuristic controllers".to_string(),
                    ))
                }
            };
            let seed_resolved = seed.map(|s| s.resolve());
            mtg_forge_rs::tournament::run_tourney(decks, games, seconds, p1_tourney, p2_tourney, seed_resolved).await?
        }
        Commands::Resume {
            snapshot_file,
            override_p1,
            override_p2,
            p1_fixed_inputs,
            p2_fixed_inputs,
            override_seed,
            override_seed_p1,
            override_seed_p2,
            verbosity,
            numeric_choices,
            debug_state_hash,
            stop_on_choice,
            stop_when_fixed_exhausted,
            snapshot_output,
            save_final_gamestate,
            log_tail,
        } => {
            run_resume(
                snapshot_file,
                override_p1,
                override_p2,
                p1_fixed_inputs,
                p2_fixed_inputs,
                override_seed,
                override_seed_p1,
                override_seed_p2,
                verbosity,
                numeric_choices,
                debug_state_hash,
                stop_on_choice,
                stop_when_fixed_exhausted,
                snapshot_output,
                save_final_gamestate,
                log_tail,
            )
            .await?
        }
    }

    Ok(())
}

/// Parse fixed input string into a vector of choice strings
///
/// Splits on semicolons to support rich text commands like "play swamp; cast bolt"
/// Each command can be either a number (legacy) or a rich text command.
fn parse_fixed_inputs(input: &str) -> std::result::Result<Vec<String>, String> {
    Ok(input
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Helper: check if we should print based on verbosity level and suppress flag
#[inline]
fn should_print(verbosity: VerbosityLevel, level: VerbosityLevel, suppress: bool) -> bool {
    verbosity >= level && !suppress
}

// StopCondition is now imported from mtg_forge_rs::game module

/// Run TUI with async card loading
#[allow(clippy::too_many_arguments)] // CLI parameters naturally map to function args
async fn run_tui(
    deck1_path: Option<PathBuf>,
    deck2_path: Option<PathBuf>,
    puzzle_path: Option<PathBuf>,
    p1_type: ControllerType,
    p2_type: ControllerType,
    p1_name: String,
    p2_name: String,
    p1_fixed_inputs: Option<String>,
    p2_fixed_inputs: Option<String>,
    seed: Option<SeedArg>,
    seed_p1: Option<SeedArg>,
    seed_p2: Option<SeedArg>,
    load_all_cards: bool,
    verbosity: VerbosityArg,
    numeric_choices: bool,
    debug_state_hash: bool,
    stop_on_choice: Option<String>,
    stop_when_fixed_exhausted: bool,
    snapshot_output: PathBuf,
    start_from: Option<PathBuf>,
    save_final_gamestate: Option<PathBuf>,
    log_tail: Option<usize>,
    p1_draw: Option<String>,
    p2_draw: Option<String>,
) -> Result<()> {
    let verbosity: VerbosityLevel = verbosity.into();
    let suppress_output = log_tail.is_some();

    // Resolve seeds early - this is the ONLY place in main() where from_entropy() is called
    let seed_resolved = seed.map(|s| s.resolve());
    let seed_p1_resolved = seed_p1.map(|s| s.resolve());
    let seed_p2_resolved = seed_p2.map(|s| s.resolve());

    if !suppress_output {
        println!("=== MTG Forge Rust - Text UI Mode ===\n");
    }

    // Parse stop condition if provided
    let stop_condition = if let Some(ref stop_str) = stop_on_choice {
        let condition = StopCondition::parse(stop_str)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --stop-on-choice: {}", e)))?;
        if !suppress_output {
            println!("Stop condition: {:?}", condition);
            println!("Snapshot output: {}\n", snapshot_output.display());
        }
        Some(condition)
    } else {
        None
    };

    // Parse hand setup if provided
    let p1_hand_setup = if let Some(ref p1_draw_str) = p1_draw {
        Some(mtg_forge_rs::game::HandSetup::parse(p1_draw_str)?)
    } else {
        None
    };

    let p2_hand_setup = if let Some(ref p2_draw_str) = p2_draw {
        Some(mtg_forge_rs::game::HandSetup::parse(p2_draw_str)?)
    } else {
        None
    };

    // Check for conflicting options
    if start_from.is_some() && (deck1_path.is_some() || deck2_path.is_some() || puzzle_path.is_some()) {
        return Err(mtg_forge_rs::MtgError::InvalidAction(
            "Cannot specify both --start-from and deck/puzzle files".to_string(),
        ));
    }

    // Hand setup flags only work at game start, not when resuming from snapshot
    if start_from.is_some() && (p1_draw.is_some() || p2_draw.is_some()) {
        return Err(mtg_forge_rs::MtgError::InvalidAction(
            "--p1-draw and --p2-draw only work at game start (not when resuming from snapshot)".to_string(),
        ));
    }

    // Create async card database
    let cardsfolder = PathBuf::from("cardsfolder");
    let card_db = CardDatabase::new(cardsfolder);

    // Load snapshot early if resuming, so we can extract both game state and player-specific choices
    let loaded_snapshot: Option<GameSnapshot> = if let Some(ref snapshot_file) = start_from {
        let snapshot = GameSnapshot::load_from_file(snapshot_file)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to load snapshot: {}", e)))?;
        Some(snapshot)
    } else {
        None
    };

    let snapshot_turn_number: Option<u32> = loaded_snapshot.as_ref().map(|s| s.turn_number);

    let mut game = if let Some(ref snapshot) = loaded_snapshot {
        // Load game from snapshot
        if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
            println!("Loading snapshot from: {}", start_from.as_ref().unwrap().display());
            println!("  Turn number: {}", snapshot.turn_number);
            println!("  Intra-turn choices to replay: {}", snapshot.choice_count());
            println!("Game loaded from snapshot!\n");
        }

        // Note: We don't need to load cards for snapshots since the GameState
        // already contains all the card data
        snapshot.game_state.clone()
    } else if let Some(puzzle_file) = puzzle_path {
        // Load game from puzzle file
        if !suppress_output {
            println!("Loading puzzle file: {}", puzzle_file.display());
        }
        let puzzle_contents = std::fs::read_to_string(&puzzle_file)?;
        let puzzle = PuzzleFile::parse(&puzzle_contents)?;
        if !suppress_output {
            println!("  Puzzle: {}", puzzle.metadata.name);
            println!("  Goal: {:?}", puzzle.metadata.goal);
            println!("  Difficulty: {:?}\n", puzzle.metadata.difficulty);

            // Load cards needed for puzzle
            println!("Loading card database...");
        }
        let (count, duration) = if load_all_cards {
            card_db.eager_load().await?
        } else {
            // Extract card names from puzzle state
            let mut card_names = std::collections::HashSet::new();
            for player in &puzzle.state.players {
                for card_def in player
                    .hand
                    .iter()
                    .chain(player.battlefield.iter())
                    .chain(player.graveyard.iter())
                    .chain(player.library.iter())
                    .chain(player.exile.iter())
                {
                    card_names.insert(card_def.name.clone());
                }
            }
            card_db.load_cards(&card_names.into_iter().collect::<Vec<_>>()).await?
        };
        if !suppress_output {
            println!("  Loaded {count} cards");
            eprintln!("  (Loading time: {:.2}ms)", duration.as_secs_f64() * 1000.0);

            println!("Initializing game from puzzle...");
        }
        load_puzzle_into_game(&puzzle, &card_db).await?
    } else {
        // Load game from deck files
        let deck1_path = deck1_path.expect("deck1 required when not loading from puzzle");
        // If deck2 not provided, use deck1 for both players
        let deck2_path = deck2_path.as_ref().unwrap_or(&deck1_path);

        if !suppress_output {
            println!("Loading deck files...");
        }
        let deck1 = DeckLoader::load_from_file(&deck1_path)?;
        let deck2 = DeckLoader::load_from_file(deck2_path)?;

        if !suppress_output {
            if deck2_path == &deck1_path {
                println!("  Using same deck for both players: {} cards", deck1.total_cards());
            } else {
                println!("  Player 1: {} cards", deck1.total_cards());
                println!("  Player 2: {} cards", deck2.total_cards());
            }
            println!();

            // Load cards based on mode
            println!("Loading card database...");
        }
        let (count, duration) = if load_all_cards {
            // Load all cards from cardsfolder
            card_db.eager_load().await?
        } else {
            // Load only cards needed for the two decks
            let mut unique_names = deck1.unique_card_names();
            unique_names.extend(deck2.unique_card_names());
            card_db.load_cards(&unique_names).await?
        };
        if !suppress_output {
            println!("  Loaded {count} cards");
            eprintln!("  (Loading time: {:.2}ms)", duration.as_secs_f64() * 1000.0);

            // Initialize game
            println!("Initializing game...");
        }
        let game_init = GameInitializer::new(&card_db);
        game_init
            .init_game(
                p1_name.clone(),
                &deck1,
                p2_name.clone(),
                &deck2,
                20, // starting life
            )
            .await?
    };

    // Set random seed if provided
    if let Some(seed_value) = seed_resolved {
        game.seed_rng(seed_value);
        if !suppress_output {
            println!("Using random seed: {seed_value}");
        }
    }

    // Report controller seeds if set
    if !suppress_output {
        if let Some(p1_seed_value) = seed_p1_resolved {
            println!("Using explicit P1 controller seed: {p1_seed_value}");
        } else if let Some(seed_value) = seed_resolved {
            println!(
                "Using derived P1 controller seed: {} (from master seed)",
                seed_value.wrapping_add(0x1234_5678_9ABC_DEF0)
            );
        }

        if let Some(p2_seed_value) = seed_p2_resolved {
            println!("Using explicit P2 controller seed: {p2_seed_value}");
        } else if let Some(seed_value) = seed_resolved {
            println!(
                "Using derived P2 controller seed: {} (from master seed)",
                seed_value.wrapping_add(0xFEDC_BA98_7654_3210)
            );
        }
    }

    // Enable numeric choices mode if requested
    if numeric_choices {
        game.logger.set_numeric_choices(true);
        if !suppress_output {
            println!("Numeric choices mode: enabled");
        }
    }

    // Enable state hash debugging if requested
    if debug_state_hash {
        game.logger.set_debug_state_hash(true);
        if !suppress_output {
            println!("State hash debugging: enabled");
        }
    }

    if !suppress_output {
        println!("Game initialized!");
        println!("  Player 1: {} ({p1_type:?})", p1_name);
        println!("  Player 2: {} ({p2_type:?})\n", p2_name);
    }

    // Create controllers based on agent types
    let (p1_id, p2_id) = {
        let p1 = game.get_player_by_idx(0).expect("Should have player 1");
        let p2 = game.get_player_by_idx(1).expect("Should have player 2");
        (p1.id, p2.id)
    };

    // Derive controller seeds from master seed using salt constants
    // Priority: explicit --seed-p1/--seed-p2 > derived from --seed > from_entropy (with warning)
    // This ensures P1 and P2 get independent random streams from the same master seed
    let p1_controller_seed = seed_p1_resolved.or_else(|| seed_resolved.map(|s| s.wrapping_add(0x1234_5678_9ABC_DEF0)));
    let p2_controller_seed = seed_p2_resolved.or_else(|| seed_resolved.map(|s| s.wrapping_add(0xFEDC_BA98_7654_3210)));

    // Create base controllers
    let base_controller1: Box<dyn mtg_forge_rs::game::controller::PlayerController> = match p1_type {
        ControllerType::Zero => Box::new(ZeroController::new(p1_id)),
        ControllerType::Random => {
            // Check if we're resuming from snapshot with saved RandomController state
            if let Some(ref snapshot) = loaded_snapshot {
                if let Some(mtg_forge_rs::game::ControllerState::Random(random_controller)) =
                    &snapshot.p1_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!("Player 1 Random controller restored from snapshot");
                    }
                    Box::new(random_controller.clone())
                } else if let Some(p1_seed) = p1_controller_seed {
                    // No saved state, create fresh controller with seed
                    Box::new(RandomController::with_seed(p1_id, p1_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        eprintln!(
                            "Warning: No seed provided for P1 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        eprintln!("  To make this deterministic, use --seed or --seed-p1");
                    }
                    Box::new(RandomController::with_seed(p1_id, entropy_seed))
                }
            } else if let Some(p1_seed) = p1_controller_seed {
                Box::new(RandomController::with_seed(p1_id, p1_seed))
            } else {
                // No seed provided - generate from entropy with warning
                let entropy_seed = SeedArg::FromEntropy.resolve();
                if !suppress_output {
                    eprintln!(
                        "Warning: No seed provided for P1 Random controller, using entropy: {}",
                        entropy_seed
                    );
                    eprintln!("  To make this deterministic, use --seed or --seed-p1");
                }
                Box::new(RandomController::with_seed(p1_id, entropy_seed))
            }
        }
        ControllerType::Tui => Box::new(InteractiveController::with_numeric_choices(p1_id, numeric_choices)),
        ControllerType::Heuristic => Box::new(HeuristicController::new(p1_id)),
        ControllerType::Fixed => {
            // Priority: CLI --p1-fixed-inputs > snapshot state > error
            if let Some(input) = &p1_fixed_inputs {
                // CLI override - use provided script
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p1-fixed-inputs: {}", e))
                })?;
                Box::new(RichInputController::new(p1_id, script))
            } else if let Some(ref snapshot) = loaded_snapshot {
                // Restore from snapshot if available
                if let Some(mtg_forge_rs::game::ControllerState::Fixed(fixed_controller)) =
                    &snapshot.p1_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!(
                            "Player 1 Fixed controller restored from snapshot (at index {})",
                            fixed_controller.current_index
                        );
                    }
                    Box::new(fixed_controller.clone())
                } else {
                    return Err(mtg_forge_rs::MtgError::InvalidAction(
                        "--p1-fixed-inputs is required when --p1=fixed (no snapshot state available or wrong controller type)".to_string(),
                    ));
                }
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p1-fixed-inputs is required when --p1=fixed".to_string(),
                ));
            }
        }
    };

    let base_controller2: Box<dyn mtg_forge_rs::game::controller::PlayerController> = match p2_type {
        ControllerType::Zero => Box::new(ZeroController::new(p2_id)),
        ControllerType::Random => {
            // Check if we're resuming from snapshot with saved RandomController state
            if let Some(ref snapshot) = loaded_snapshot {
                if let Some(mtg_forge_rs::game::ControllerState::Random(random_controller)) =
                    &snapshot.p2_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!("Player 2 Random controller restored from snapshot");
                    }
                    Box::new(random_controller.clone())
                } else if let Some(p2_seed) = p2_controller_seed {
                    // No saved state, create fresh controller with seed
                    Box::new(RandomController::with_seed(p2_id, p2_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        eprintln!(
                            "Warning: No seed provided for P2 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        eprintln!("  To make this deterministic, use --seed or --seed-p2");
                    }
                    Box::new(RandomController::with_seed(p2_id, entropy_seed))
                }
            } else if let Some(p2_seed) = p2_controller_seed {
                Box::new(RandomController::with_seed(p2_id, p2_seed))
            } else {
                // No seed provided - generate from entropy with warning
                let entropy_seed = SeedArg::FromEntropy.resolve();
                if !suppress_output {
                    eprintln!(
                        "Warning: No seed provided for P2 Random controller, using entropy: {}",
                        entropy_seed
                    );
                    eprintln!("  To make this deterministic, use --seed or --seed-p2");
                }
                Box::new(RandomController::with_seed(p2_id, entropy_seed))
            }
        }
        ControllerType::Tui => Box::new(InteractiveController::with_numeric_choices(p2_id, numeric_choices)),
        ControllerType::Heuristic => Box::new(HeuristicController::new(p2_id)),
        ControllerType::Fixed => {
            // Priority: CLI --p2-fixed-inputs > snapshot state > error
            if let Some(input) = &p2_fixed_inputs {
                // CLI override - use provided script
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p2-fixed-inputs: {}", e))
                })?;
                Box::new(RichInputController::new(p2_id, script))
            } else if let Some(ref snapshot) = loaded_snapshot {
                // Restore from snapshot if available
                if let Some(mtg_forge_rs::game::ControllerState::Fixed(fixed_controller)) =
                    &snapshot.p2_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!(
                            "Player 2 Fixed controller restored from snapshot (at index {})",
                            fixed_controller.current_index
                        );
                    }
                    Box::new(fixed_controller.clone())
                } else {
                    return Err(mtg_forge_rs::MtgError::InvalidAction(
                        "--p2-fixed-inputs is required when --p2=fixed (no snapshot state available or wrong controller type)".to_string(),
                    ));
                }
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p2-fixed-inputs is required when --p2=fixed".to_string(),
                ));
            }
        }
    };

    // Wrap with ReplayController if resuming from snapshot
    // CRITICAL: Each controller must only replay its OWN choices, not the other player's!
    //
    // EXCEPTION: Don't wrap FixedScriptController with ReplayController.
    // Fixed controller already has the full game script and wrapping it would cause
    // double-replay (ReplayController replays intra-turn, then Fixed restarts from index 0).
    let mut controller1: Box<dyn mtg_forge_rs::game::controller::PlayerController> =
        if let Some(ref snapshot) = loaded_snapshot {
            // Check if base controller is Fixed - don't wrap if it is
            let is_fixed = matches!(p1_type, ControllerType::Fixed);
            if is_fixed {
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!("Player 1 using Fixed controller (skipping Replay wrapper)");
                }
                base_controller1
            } else {
                let p1_replay_choices = snapshot.extract_replay_choices_for_player(p1_id);
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!("Player 1 will replay {} intra-turn choices", p1_replay_choices.len());
                }
                Box::new(mtg_forge_rs::game::ReplayController::new(
                    p1_id,
                    base_controller1,
                    p1_replay_choices,
                ))
            }
        } else {
            base_controller1
        };

    let mut controller2: Box<dyn mtg_forge_rs::game::controller::PlayerController> =
        if let Some(ref snapshot) = loaded_snapshot {
            // Check if base controller is Fixed - don't wrap if it is
            let is_fixed = matches!(p2_type, ControllerType::Fixed);
            if is_fixed {
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!("Player 2 using Fixed controller (skipping Replay wrapper)");
                }
                base_controller2
            } else {
                let p2_replay_choices = snapshot.extract_replay_choices_for_player(p2_id);
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!("Player 2 will replay {} intra-turn choices", p2_replay_choices.len());
                }
                Box::new(mtg_forge_rs::game::ReplayController::new(
                    p2_id,
                    base_controller2,
                    p2_replay_choices,
                ))
            }
        } else {
            base_controller2
        };

    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        if snapshot_turn_number.is_some() {
            println!("=== Continuing Game ===\n");
        } else {
            println!("=== Starting Game ===\n");
        }
    }

    // Enable log tail mode if requested (captures logs to buffer)
    // Must be done BEFORE creating game loop since loop borrows game mutably
    if log_tail.is_some() {
        game.logger.enable_capture();
    }

    // Run the game loop (with or without snapshots)
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(verbosity);

    // If loading from snapshot, restore the turn counter
    // Note: snapshot.turn_number represents the turn we're STARTING,
    // but turns_elapsed tracks COMPLETED turns, so we need turn_number - 1
    if let Some(turn_num) = snapshot_turn_number {
        // Turn numbers are 1-based (turn 1, 2, 3...), never 0
        // If we see turn 0, that's a critical bug in snapshot serialization
        if turn_num == 0 {
            return Err(mtg_forge_rs::MtgError::InvalidAction(
                "Invalid snapshot: turn_number is 0 (turns are 1-based, not 0-based)".to_string(),
            ));
        }
        let turns_elapsed = turn_num - 1;
        game_loop = game_loop.with_turn_counter(turns_elapsed);
    }

    // Restore choice counter from snapshot if resuming
    if let Some(ref snapshot) = loaded_snapshot {
        game_loop = game_loop.with_choice_counter(snapshot.total_choice_count);
    }

    // Enable stop-when-fixed-exhausted if requested
    if stop_when_fixed_exhausted {
        game_loop = game_loop.with_stop_when_fixed_exhausted(&snapshot_output);
    }

    // If resuming from snapshot, set baseline choice count for replay mode
    // This is ALWAYS needed when resuming to determine when to stop suppressing logs,
    // not just when using --stop-on-choice
    if let Some(ref snapshot) = loaded_snapshot {
        use mtg_forge_rs::undo::GameAction;

        // Count all ChoicePoints in the undo log to establish baseline
        // If stop_condition exists, filter by applicable player; otherwise count all
        let baseline_count = if let Some(ref stop_cond) = stop_condition {
            snapshot
                .game_state
                .undo_log
                .actions()
                .iter()
                .filter(|action| {
                    if let GameAction::ChoicePoint { player_id, .. } = action {
                        stop_cond.applies_to(p1_id, *player_id)
                    } else {
                        false
                    }
                })
                .count()
        } else {
            // No stop condition - count ALL choice points for replay mode
            snapshot
                .game_state
                .undo_log
                .actions()
                .iter()
                .filter(|action| matches!(action, GameAction::ChoicePoint { .. }))
                .count()
        };

        game_loop = game_loop.with_baseline_choice_count(baseline_count);

        if verbosity >= VerbosityLevel::Verbose {
            println!("Baseline choice count (from snapshot): {}", baseline_count);
        }
    }

    // If resuming from snapshot, enable replay mode to suppress logging during replay
    // This must be done AFTER setting baseline, and applies regardless of stop_condition
    if let Some(ref snapshot) = loaded_snapshot {
        use mtg_forge_rs::undo::GameAction;

        // Count ALL ChoicePoint entries - each one will trigger log_choice_point
        // and we need to suppress logging for all of them until replay is complete
        let replay_choice_count = snapshot
            .intra_turn_choices
            .iter()
            .filter(|action| matches!(action, GameAction::ChoicePoint { .. }))
            .count();
        game_loop = game_loop.with_replay_mode(replay_choice_count);

        if verbosity >= VerbosityLevel::Verbose {
            println!("Replay mode enabled: {} choices to replay", replay_choice_count);
        }
    }

    // Enable stop condition (--stop-on-choice) if requested
    if let Some(ref stop_cond) = stop_condition {
        game_loop = game_loop.with_stop_condition(p1_id, stop_cond.clone(), &snapshot_output);
    }

    // Set hand setup for controlled initial hands (testing)
    if let Some(ref p1_setup) = p1_hand_setup {
        game_loop = game_loop.with_p1_hand_setup(p1_setup.clone());
    }
    if let Some(ref p2_setup) = p2_hand_setup {
        game_loop = game_loop.with_p2_hand_setup(p2_setup.clone());
    }

    // Run the game (with mid-turn exits if stop conditions enabled)
    let result = game_loop.run_game(&mut *controller1, &mut *controller2)?;

    // If log_tail was enabled, flush only the last K lines now
    if let Some(tail_lines) = log_tail {
        game.logger.flush_tail(tail_lines);
    }

    // If game ended with a snapshot, reload and add controller state
    use mtg_forge_rs::game::GameEndReason;
    if result.end_reason == GameEndReason::Snapshot && snapshot_output.exists() {
        // Extract controller states by calling get_snapshot_state()
        let p1_state_json = controller1.get_snapshot_state();
        let p2_state_json = controller2.get_snapshot_state();

        // If either controller has state to preserve, update the snapshot
        if p1_state_json.is_some() || p2_state_json.is_some() {
            if let Ok(mut snapshot) = GameSnapshot::load_from_file(&snapshot_output) {
                // Deserialize JSON back to ControllerState (Fixed or Random) if present
                snapshot.p1_controller_state = p1_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                eprintln!("Failed to deserialize P1 controller state: {}", e);
                                eprintln!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });
                snapshot.p2_controller_state = p2_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                eprintln!("Failed to deserialize P2 controller state: {}", e);
                                eprintln!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });

                if let Err(e) = snapshot.save_to_file(&snapshot_output) {
                    eprintln!("Warning: Failed to update snapshot with controller state: {}", e);
                } else if verbosity >= VerbosityLevel::Verbose {
                    println!("Snapshot updated with controller state");
                }
            }
        }
    }

    // Display results (suppress for snapshot exits)
    if verbosity >= VerbosityLevel::Minimal && result.end_reason != GameEndReason::Snapshot {
        println!("\n=== Game Over ===");
        match result.winner {
            Some(winner_id) => {
                let winner = game.get_player(winner_id)?;
                println!("Winner: {}", winner.name);
            }
            None => {
                println!("Game ended in a draw");
            }
        }
        println!("Turns played: {}", result.turns_played);
        println!("Reason: {:?}", result.end_reason);

        // Final state
        println!("\n=== Final State ===");
        for player in game.players.iter() {
            println!("  {}: {} life", player.name, player.life);
        }
    }

    // Save final gamestate if requested (for determinism testing)
    if let Some(final_state_path) = save_final_gamestate {
        if result.end_reason != GameEndReason::Snapshot {
            // Create a snapshot with the final game state
            let final_snapshot = GameSnapshot::new(
                game.clone(),
                result.turns_played,
                Vec::new(), // No intra-turn choices for final state
            );

            final_snapshot
                .save_to_file(&final_state_path)
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to save final gamestate: {}", e)))?;

            if verbosity >= VerbosityLevel::Verbose {
                println!("\nFinal game state saved to: {}", final_state_path.display());
            }
        }
    }

    Ok(())
}

/// Run profiling games
async fn run_profile(iterations: usize, seed: u64, deck_path: PathBuf) -> Result<()> {
    println!("=== MTG Forge Rust - Profiling Mode ===\n");

    // Load deck
    println!("Loading deck...");
    let deck = DeckLoader::load_from_file(&deck_path)?;
    println!("  Deck: {} cards", deck.total_cards());

    // Create card database (lazy loading - only loads cards on-demand)
    let cardsfolder = PathBuf::from("cardsfolder");
    let card_db = CardDatabase::new(cardsfolder);

    // Prefetch deck cards (not all 31k cards, just what we need)
    let start = std::time::Instant::now();
    let unique_names = deck.unique_card_names();
    let (count, _) = card_db.load_cards(&unique_names).await?;
    let duration = start.elapsed();
    println!("  Loaded {count} cards in {:.2}ms\n", duration.as_secs_f64() * 1000.0);

    println!("Profiling game execution...");
    println!("Running {iterations} games with seed {seed}");
    println!();

    // Run games in a tight loop for profiling
    for i in 0..iterations {
        // Initialize game
        let game_init = GameInitializer::new(&card_db);
        let mut game = game_init
            .init_game("Player 1".to_string(), &deck, "Player 2".to_string(), &deck, 20)
            .await?;
        game.seed_rng(seed);

        // Create random controllers with deterministic seeds derived from master seed
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Use same salt constants as main game for consistency
        let p1_seed = seed.wrapping_add(0x1234_5678_9ABC_DEF0);
        let p2_seed = seed.wrapping_add(0xFEDC_BA98_7654_3210);

        let mut controller1 = RandomController::with_seed(p1_id, p1_seed);
        let mut controller2 = RandomController::with_seed(p2_id, p2_seed);

        // Run game
        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        game_loop.run_game(&mut controller1, &mut controller2)?;

        // Print progress every 100 games
        if (i + 1) % 100 == 0 {
            println!("Completed {} games", i + 1);
        }
    }

    println!();
    println!("Profiling complete! {iterations} games executed.");
    println!();
    println!("For heap profiling:");
    println!("  cargo heaptrack --bin mtg -- profile --games {iterations} --seed {seed}");
    println!("  Or: make heapprofile");
    println!();
    println!("For CPU profiling:");
    println!("  cargo flamegraph --bin mtg -- profile --games {iterations} --seed {seed}");
    println!("  Or: make profile");

    Ok(())
}

/// Resume a saved game from snapshot
///
/// Default behavior: Restores ALL state from snapshot (game, controllers, RNG states, choices).
/// Use --override flags to selectively replace controllers or seeds with new values.
#[allow(clippy::too_many_arguments)]
async fn run_resume(
    snapshot_file: PathBuf,
    override_p1: Option<ControllerType>,
    override_p2: Option<ControllerType>,
    p1_fixed_inputs: Option<String>,
    p2_fixed_inputs: Option<String>,
    override_seed: Option<SeedArg>,
    override_seed_p1: Option<SeedArg>,
    override_seed_p2: Option<SeedArg>,
    verbosity: VerbosityArg,
    numeric_choices: bool,
    debug_state_hash: bool,
    stop_on_choice: Option<String>,
    stop_when_fixed_exhausted: bool,
    snapshot_output: PathBuf,
    save_final_gamestate: Option<PathBuf>,
    log_tail: Option<usize>,
) -> Result<()> {
    let verbosity: VerbosityLevel = verbosity.into();
    let suppress_output = log_tail.is_some();

    // Resolve override seeds early if provided
    let override_seed_resolved = override_seed.map(|s| s.resolve());
    let override_seed_p1_resolved = override_seed_p1.map(|s| s.resolve());
    let override_seed_p2_resolved = override_seed_p2.map(|s| s.resolve());

    if !suppress_output {
        println!("=== MTG Forge Rust - Resume Mode ===\n");
    }

    // Parse stop condition if provided
    let stop_condition = if let Some(ref stop_str) = stop_on_choice {
        let condition = StopCondition::parse(stop_str)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --stop-on-choice: {}", e)))?;
        if !suppress_output {
            println!("Stop condition: {:?}", condition);
            println!("Snapshot output: {}\n", snapshot_output.display());
        }
        Some(condition)
    } else {
        None
    };

    // Load snapshot (always required for resume mode)
    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        println!("Loading snapshot from: {}", snapshot_file.display());
    }

    let snapshot = GameSnapshot::load_from_file(&snapshot_file)
        .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to load snapshot: {}", e)))?;

    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        println!("  Turn number: {}", snapshot.turn_number);
        println!("  Intra-turn choices to replay: {}", snapshot.choice_count());
    }

    // Determine controller types (restore from snapshot or use overrides)
    let p1_type = override_p1.unwrap_or({
        // Use the saved controller type from snapshot
        match snapshot.p1_controller_type {
            mtg_forge_rs::game::ControllerType::Zero => ControllerType::Zero,
            mtg_forge_rs::game::ControllerType::Random => ControllerType::Random,
            mtg_forge_rs::game::ControllerType::Tui => ControllerType::Tui,
            mtg_forge_rs::game::ControllerType::Heuristic => ControllerType::Heuristic,
            mtg_forge_rs::game::ControllerType::Fixed => ControllerType::Fixed,
        }
    });

    let p2_type = override_p2.unwrap_or({
        // Use the saved controller type from snapshot
        match snapshot.p2_controller_type {
            mtg_forge_rs::game::ControllerType::Zero => ControllerType::Zero,
            mtg_forge_rs::game::ControllerType::Random => ControllerType::Random,
            mtg_forge_rs::game::ControllerType::Tui => ControllerType::Tui,
            mtg_forge_rs::game::ControllerType::Heuristic => ControllerType::Heuristic,
            mtg_forge_rs::game::ControllerType::Fixed => ControllerType::Fixed,
        }
    });

    // Print what's being restored vs overridden
    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        if override_p1.is_some() {
            println!("Player 1 controller: OVERRIDDEN to {:?}", p1_type);
        } else {
            println!("Player 1 controller: restored from snapshot ({:?})", p1_type);
        }

        if override_p2.is_some() {
            println!("Player 2 controller: OVERRIDDEN to {:?}", p2_type);
        } else {
            println!("Player 2 controller: restored from snapshot ({:?})", p2_type);
        }

        if override_seed.is_some() {
            println!("Game engine seed: OVERRIDDEN to {}", override_seed_resolved.unwrap());
        } else {
            println!("Game engine seed: restored from snapshot");
        }

        println!("Game loaded from snapshot!\n");
    }

    // Restore game state from snapshot
    let mut game = snapshot.game_state.clone();

    // Override game engine seed if requested
    if let Some(seed_value) = override_seed_resolved {
        game.seed_rng(seed_value);
        if !suppress_output {
            println!("Overriding game engine seed: {seed_value}");
        }
    }

    // Enable numeric choices mode if requested
    if numeric_choices {
        game.logger.set_numeric_choices(true);
        if !suppress_output {
            println!("Numeric choices mode: enabled");
        }
    }

    // Enable state hash debugging if requested
    if debug_state_hash {
        game.logger.set_debug_state_hash(true);
        if !suppress_output {
            println!("State hash debugging: enabled");
        }
    }

    // Get player IDs
    let (p1_id, p2_id) = {
        let p1 = game.get_player_by_idx(0).expect("Should have player 1");
        let p2 = game.get_player_by_idx(1).expect("Should have player 2");
        (p1.id, p2.id)
    };

    // Get player names for display
    let p1_name = game.get_player(p1_id)?.name.clone();
    let p2_name = game.get_player(p2_id)?.name.clone();

    if !suppress_output {
        println!("  Player 1: {} ({p1_type:?})", p1_name);
        println!("  Player 2: {} ({p2_type:?})\n", p2_name);
    }

    // Derive controller seeds (override takes precedence, otherwise restore from snapshot)
    // If overriding with no explicit seed and controller needs one, use master seed derivation
    let p1_controller_seed = if override_p1.is_some() {
        // We're overriding P1 controller - use explicit override seed or derive from master seed
        override_seed_p1_resolved.or_else(|| override_seed_resolved.map(|s| s.wrapping_add(0x1234_5678_9ABC_DEF0)))
    } else {
        // Restoring P1 controller - override seed takes precedence, otherwise None (use snapshot state)
        override_seed_p1_resolved
    };

    let p2_controller_seed = if override_p2.is_some() {
        // We're overriding P2 controller - use explicit override seed or derive from master seed
        override_seed_p2_resolved.or_else(|| override_seed_resolved.map(|s| s.wrapping_add(0xFEDC_BA98_7654_3210)))
    } else {
        // Restoring P2 controller - override seed takes precedence, otherwise None (use snapshot state)
        override_seed_p2_resolved
    };

    // Create base controllers
    let base_controller1: Box<dyn mtg_forge_rs::game::controller::PlayerController> = match p1_type {
        ControllerType::Zero => Box::new(ZeroController::new(p1_id)),
        ControllerType::Random => {
            // If overriding or if override seed provided, create fresh controller
            if override_p1.is_some() || p1_controller_seed.is_some() {
                if let Some(p1_seed) = p1_controller_seed {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!("Player 1 Random controller: fresh with seed {}", p1_seed);
                    }
                    Box::new(RandomController::with_seed(p1_id, p1_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        eprintln!(
                            "Warning: No seed provided for P1 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        eprintln!("  To make this deterministic, use --override-seed or --override-seed-p1");
                    }
                    Box::new(RandomController::with_seed(p1_id, entropy_seed))
                }
            } else {
                // Restore from snapshot
                if let Some(mtg_forge_rs::game::ControllerState::Random(random_controller)) =
                    &snapshot.p1_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!("Player 1 Random controller: restored from snapshot");
                    }
                    Box::new(random_controller.clone())
                } else {
                    return Err(mtg_forge_rs::MtgError::InvalidAction(
                        "Cannot restore Random controller: no saved state in snapshot".to_string(),
                    ));
                }
            }
        }
        ControllerType::Tui => Box::new(InteractiveController::with_numeric_choices(p1_id, numeric_choices)),
        ControllerType::Heuristic => Box::new(HeuristicController::new(p1_id)),
        ControllerType::Fixed => {
            // Priority: CLI --p1-fixed-inputs > snapshot state > error
            if let Some(input) = &p1_fixed_inputs {
                // CLI override - use provided script
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p1-fixed-inputs: {}", e))
                })?;
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!("Player 1 Fixed controller: fresh with {} commands", script.len());
                }
                Box::new(RichInputController::new(p1_id, script))
            } else if let Some(mtg_forge_rs::game::ControllerState::Fixed(fixed_controller)) =
                &snapshot.p1_controller_state
            {
                // Restore from snapshot
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!(
                        "Player 1 Fixed controller: restored from snapshot (at index {})",
                        fixed_controller.current_index
                    );
                }
                Box::new(fixed_controller.clone())
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p1-fixed-inputs is required when --override-p1=fixed (no snapshot state available)".to_string(),
                ));
            }
        }
    };

    let base_controller2: Box<dyn mtg_forge_rs::game::controller::PlayerController> = match p2_type {
        ControllerType::Zero => Box::new(ZeroController::new(p2_id)),
        ControllerType::Random => {
            // If overriding or if override seed provided, create fresh controller
            if override_p2.is_some() || p2_controller_seed.is_some() {
                if let Some(p2_seed) = p2_controller_seed {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!("Player 2 Random controller: fresh with seed {}", p2_seed);
                    }
                    Box::new(RandomController::with_seed(p2_id, p2_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        eprintln!(
                            "Warning: No seed provided for P2 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        eprintln!("  To make this deterministic, use --override-seed or --override-seed-p2");
                    }
                    Box::new(RandomController::with_seed(p2_id, entropy_seed))
                }
            } else {
                // Restore from snapshot
                if let Some(mtg_forge_rs::game::ControllerState::Random(random_controller)) =
                    &snapshot.p2_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        println!("Player 2 Random controller: restored from snapshot");
                    }
                    Box::new(random_controller.clone())
                } else {
                    return Err(mtg_forge_rs::MtgError::InvalidAction(
                        "Cannot restore Random controller: no saved state in snapshot".to_string(),
                    ));
                }
            }
        }
        ControllerType::Tui => Box::new(InteractiveController::with_numeric_choices(p2_id, numeric_choices)),
        ControllerType::Heuristic => Box::new(HeuristicController::new(p2_id)),
        ControllerType::Fixed => {
            // Priority: CLI --p2-fixed-inputs > snapshot state > error
            if let Some(input) = &p2_fixed_inputs {
                // CLI override - use provided script
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p2-fixed-inputs: {}", e))
                })?;
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!("Player 2 Fixed controller: fresh with {} commands", script.len());
                }
                Box::new(RichInputController::new(p2_id, script))
            } else if let Some(mtg_forge_rs::game::ControllerState::Fixed(fixed_controller)) =
                &snapshot.p2_controller_state
            {
                // Restore from snapshot
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    println!(
                        "Player 2 Fixed controller: restored from snapshot (at index {})",
                        fixed_controller.current_index
                    );
                }
                Box::new(fixed_controller.clone())
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p2-fixed-inputs is required when --override-p2=fixed (no snapshot state available)".to_string(),
                ));
            }
        }
    };

    // Wrap with ReplayController (always necessary when resuming from snapshot)
    // EXCEPTION: Don't wrap FixedScriptController with ReplayController.
    // Fixed controller already has the full game script and wrapping it would cause
    // double-replay (ReplayController replays intra-turn, then Fixed restarts from index 0).
    let mut controller1: Box<dyn mtg_forge_rs::game::controller::PlayerController> = {
        let is_fixed = matches!(p1_type, ControllerType::Fixed);
        if is_fixed {
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                println!("Player 1 using Fixed controller (skipping Replay wrapper)");
            }
            base_controller1
        } else {
            let p1_replay_choices = snapshot.extract_replay_choices_for_player(p1_id);
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                println!("Player 1 will replay {} intra-turn choices", p1_replay_choices.len());
            }
            Box::new(mtg_forge_rs::game::ReplayController::new(
                p1_id,
                base_controller1,
                p1_replay_choices,
            ))
        }
    };

    let mut controller2: Box<dyn mtg_forge_rs::game::controller::PlayerController> = {
        let is_fixed = matches!(p2_type, ControllerType::Fixed);
        if is_fixed {
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                println!("Player 2 using Fixed controller (skipping Replay wrapper)");
            }
            base_controller2
        } else {
            let p2_replay_choices = snapshot.extract_replay_choices_for_player(p2_id);
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                println!("Player 2 will replay {} intra-turn choices", p2_replay_choices.len());
            }
            Box::new(mtg_forge_rs::game::ReplayController::new(
                p2_id,
                base_controller2,
                p2_replay_choices,
            ))
        }
    };

    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        println!("=== Resuming Game ===\n");
    }

    // Enable log tail mode if requested (captures logs to buffer)
    // Must be done BEFORE creating game loop since loop borrows game mutably
    if log_tail.is_some() {
        game.logger.enable_capture();
    }

    // Run the game loop
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(verbosity);

    // Restore the turn counter
    // Note: snapshot.turn_number represents the turn we're STARTING,
    // but turns_elapsed tracks COMPLETED turns, so we need turn_number - 1
    let turn_num = snapshot.turn_number;
    if turn_num == 0 {
        return Err(mtg_forge_rs::MtgError::InvalidAction(
            "Invalid snapshot: turn_number is 0 (turns are 1-based, not 0-based)".to_string(),
        ));
    }
    let turns_elapsed = turn_num - 1;
    game_loop = game_loop.with_turn_counter(turns_elapsed);

    // Restore choice counter from snapshot
    game_loop = game_loop.with_choice_counter(snapshot.total_choice_count);

    // Enable stop-when-fixed-exhausted if requested
    if stop_when_fixed_exhausted {
        game_loop = game_loop.with_stop_when_fixed_exhausted(&snapshot_output);
    }

    // Set baseline choice count for replay mode
    // This is ALWAYS needed when resuming to determine when to stop suppressing logs
    {
        use mtg_forge_rs::undo::GameAction;

        // Count all ChoicePoints in the undo log to establish baseline
        // If stop_condition exists, filter by applicable player; otherwise count all
        let baseline_count = if let Some(ref stop_cond) = stop_condition {
            snapshot
                .game_state
                .undo_log
                .actions()
                .iter()
                .filter(|action| {
                    if let GameAction::ChoicePoint { player_id, .. } = action {
                        stop_cond.applies_to(p1_id, *player_id)
                    } else {
                        false
                    }
                })
                .count()
        } else {
            // No stop condition - count ALL choice points for replay mode
            snapshot
                .game_state
                .undo_log
                .actions()
                .iter()
                .filter(|action| matches!(action, GameAction::ChoicePoint { .. }))
                .count()
        };

        game_loop = game_loop.with_baseline_choice_count(baseline_count);

        if verbosity >= VerbosityLevel::Verbose {
            println!("Baseline choice count (from snapshot): {}", baseline_count);
        }
    }

    // Enable replay mode to suppress logging during replay
    // This must be done AFTER setting baseline
    {
        use mtg_forge_rs::undo::GameAction;

        // Count ALL ChoicePoint entries - each one will trigger log_choice_point
        // and we need to suppress logging for all of them until replay is complete
        let replay_choice_count = snapshot
            .intra_turn_choices
            .iter()
            .filter(|action| matches!(action, GameAction::ChoicePoint { .. }))
            .count();
        game_loop = game_loop.with_replay_mode(replay_choice_count);

        if verbosity >= VerbosityLevel::Verbose {
            println!("Replay mode enabled: {} choices to replay", replay_choice_count);
        }
    }

    // Enable stop condition (--stop-on-choice) if requested
    if let Some(ref stop_cond) = stop_condition {
        game_loop = game_loop.with_stop_condition(p1_id, stop_cond.clone(), &snapshot_output);
    }

    // Run the game (with mid-turn exits if stop conditions enabled)
    let result = game_loop.run_game(&mut *controller1, &mut *controller2)?;

    // If log_tail was enabled, flush only the last K lines now
    if let Some(tail_lines) = log_tail {
        game.logger.flush_tail(tail_lines);
    }

    // If game ended with a snapshot, reload and add controller state
    use mtg_forge_rs::game::GameEndReason;
    if result.end_reason == GameEndReason::Snapshot && snapshot_output.exists() {
        // Extract controller states by calling get_snapshot_state()
        let p1_state_json = controller1.get_snapshot_state();
        let p2_state_json = controller2.get_snapshot_state();

        // If either controller has state to preserve, update the snapshot
        if p1_state_json.is_some() || p2_state_json.is_some() {
            if let Ok(mut snapshot) = GameSnapshot::load_from_file(&snapshot_output) {
                // Deserialize JSON back to ControllerState (Fixed or Random) if present
                snapshot.p1_controller_state = p1_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                eprintln!("Failed to deserialize P1 controller state: {}", e);
                                eprintln!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });
                snapshot.p2_controller_state = p2_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                eprintln!("Failed to deserialize P2 controller state: {}", e);
                                eprintln!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });

                if let Err(e) = snapshot.save_to_file(&snapshot_output) {
                    eprintln!("Warning: Failed to update snapshot with controller state: {}", e);
                } else if verbosity >= VerbosityLevel::Verbose {
                    println!("Snapshot updated with controller state");
                }
            }
        }
    }

    // Display results (suppress for snapshot exits)
    if verbosity >= VerbosityLevel::Minimal && result.end_reason != GameEndReason::Snapshot {
        println!("\n=== Game Over ===");
        match result.winner {
            Some(winner_id) => {
                let winner = game.get_player(winner_id)?;
                println!("Winner: {}", winner.name);
            }
            None => {
                println!("Game ended in a draw");
            }
        }
        println!("Turns played: {}", result.turns_played);
        println!("Reason: {:?}", result.end_reason);

        // Final state
        println!("\n=== Final State ===");
        for player in game.players.iter() {
            println!("  {}: {} life", player.name, player.life);
        }
    }

    // Save final gamestate if requested (for determinism testing)
    if let Some(final_state_path) = save_final_gamestate {
        if result.end_reason != GameEndReason::Snapshot {
            // Create a snapshot with the final game state
            let final_snapshot = GameSnapshot::new(
                game.clone(),
                result.turns_played,
                Vec::new(), // No intra-turn choices for final state
            );

            final_snapshot
                .save_to_file(&final_state_path)
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to save final gamestate: {}", e)))?;

            if verbosity >= VerbosityLevel::Verbose {
                println!("\nFinal game state saved to: {}", final_state_path.display());
            }
        }
    }

    Ok(())
}
