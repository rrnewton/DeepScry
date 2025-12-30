//! MTG Forge Rust - Main Binary
//!
//! Text-based Magic: The Gathering game engine with TUI support

// Use mimalloc as the global allocator for better performance
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use clap::{Parser, Subcommand, ValueEnum};
use mtg_forge_rs::{
    game::{
        random_controller::RandomController, zero_controller::ZeroController, FancyTuiController, GameLoop,
        GameSnapshot, HeuristicController, InteractiveController, RichInputController, StopCondition, VerbosityLevel,
    },
    loader::{AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
    Result,
};
use std::path::PathBuf;

/// Find cardsfolder directory, checking current directory first then parent
fn find_cardsfolder() -> PathBuf {
    let local = PathBuf::from("cardsfolder");
    if local.exists() {
        local
    } else {
        PathBuf::from("../cardsfolder")
    }
}

/// Controller type for AI agents
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ControllerType {
    /// Always chooses first meaningful action (for testing)
    Zero,
    /// Makes random choices
    Random,
    /// Text UI controller for human play via stdin
    Tui,
    /// Full-featured TUI with ratatui (multi-panel interface)
    Fancy,
    /// Heuristic AI controller with strategic decision making
    Heuristic,
    /// Fixed script controller with predetermined choices (requires --fixed-inputs)
    Fixed,
    /// Fancy TUI with fixed scripted inputs (captures screenshots, requires --fixed-inputs)
    FancyFixed,
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

        /// Player 1 name (default: Player1)
        #[arg(long, default_value = "Player1")]
        p1_name: String,

        /// Player 2 name (default: Player2)
        #[arg(long, default_value = "Player2")]
        p2_name: String,

        /// Fixed script input for player 1 (space or comma separated indices, e.g., "1 1 2" or "1,1,2")
        #[arg(long, value_name = "CHOICES")]
        p1_fixed_inputs: Option<String>,

        /// Fixed script input for player 2 (space or comma separated indices, e.g., "1 1 2" or "1,1,2")
        #[arg(long, value_name = "CHOICES")]
        p2_fixed_inputs: Option<String>,

        /// Terminal width for fancy-fixed controller screenshots (default: 240)
        #[arg(long, default_value = "240")]
        screenshot_width: u16,

        /// Terminal height for fancy-fixed controller screenshots (default: 60)
        #[arg(long, default_value = "60")]
        screenshot_height: u16,

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

        /// Separate seed for initial deck shuffling (for sampling different games with same hands)
        ///
        /// When provided, this seed is used ONLY for the initial library shuffling.
        /// The main --seed is then used for all subsequent game RNG. This allows running
        /// multiple games with different RNG streams but identical starting hands, useful
        /// for comparing AI strategies or testing game outcomes under controlled conditions.
        #[arg(long)]
        deck_seed: Option<SeedArg>,

        /// Load all cards from cardsfolder (default: only load cards in decks)
        #[arg(long)]
        load_all_cards: bool,

        /// Verbosity level for game output (0=silent, 1=minimal, 2=normal, 3=verbose)
        #[arg(long, default_value = "normal", short = 'v')]
        verbosity: VerbosityArg,

        /// Use numeric-only choice format (for comparison with Java Forge)
        #[arg(long)]
        numeric_choices: bool,

        // === Fancy TUI Options ===
        /// Enable visual stacking with diagonal offsets for fancy TUI (default: simple stacking)
        #[arg(long)]
        visual_stacks: bool,

        /// Enable state hash debugging (prints hash before each action)
        #[arg(long)]
        debug_state_hash: bool,

        /// Tag official game action logs with [GAMELOG TurnN STEP] prefix
        /// This enables comparing local vs network game logs for correctness
        #[arg(long)]
        tag_gamelogs: bool,

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

        /// Use JSON format for snapshots (default is binary format)
        #[arg(long)]
        json: bool,

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

        /// Only play mirror matches (each deck against itself)
        #[arg(long)]
        mirror_only: bool,
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

        // === Fancy TUI Options ===
        /// Enable visual stacking with diagonal offsets for fancy TUI (default: simple stacking)
        #[arg(long)]
        visual_stacks: bool,

        /// Enable state hash debugging (prints hash before each action)
        #[arg(long)]
        debug_state_hash: bool,

        /// Tag official game action logs with [GAMELOG TurnN STEP] prefix
        /// This enables comparing local vs network game logs for correctness
        #[arg(long)]
        tag_gamelogs: bool,

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

        /// Use JSON format for snapshots (default is binary format)
        #[arg(long)]
        json: bool,

        /// Save final game state when game ends (for determinism testing)
        #[arg(long, value_name = "FILE")]
        save_final_gamestate: Option<PathBuf>,

        /// Only print the last K lines of log output at game exit
        /// (useful with --stop-on-choice to see constant-sized output)
        #[arg(long, value_name = "K")]
        log_tail: Option<usize>,
    },

    /// Print statistics about the card database
    Stats {},

    /// Fast deck entry mode - Interactive TUI for rapid deck building
    ///
    /// Provides a streamlined interface for entering paper decks with minimal keystrokes:
    /// - Start typing to fuzzy search cards
    /// - Press Enter to add 1 copy, or 1-9 to add N copies
    /// - Arrow keys to navigate results
    /// - Escape to save and exit
    DeckBuild {
        /// Deck file to edit (if exists) or create. If not specified, uses output.dck
        #[arg(value_name = "DECK_FILE")]
        deck_file: Option<PathBuf>,

        /// Output file path (overrides deck_file for saving)
        #[arg(long, short = 'o')]
        output_file: Option<PathBuf>,

        /// Path to cardsfolder (default: cardsfolder)
        #[arg(long, default_value = "cardsfolder")]
        cardsfolder: PathBuf,

        /// Only include cards from sets released on or after this year
        #[arg(long)]
        start_year: Option<u16>,

        /// Only include cards from sets released on or before this year
        #[arg(long)]
        end_year: Option<u16>,
    },

    /// Export card database and decks for WASM (browser) builds
    ExportWasm {
        /// Output directory for exported data (default: web/data)
        #[arg(long, short = 'o', default_value = "web/data")]
        output: PathBuf,

        /// Glob pattern(s) for deck files to include (can specify multiple)
        /// Default includes old_school decks and spiderman draft decks
        #[arg(long, short = 'd', default_values_t = vec![
            "decks/old_school/*.dck".to_string(),
            "decks/*spiderman*.dck".to_string(),
        ])]
        deck_globs: Vec<String>,
    },

    /// Download card images from Scryfall for offline use
    ///
    /// Downloads both small (146x204) and normal (488x680) versions by default.
    /// Images are saved to ./images/small/ and ./images/normal/
    Download {
        /// Output directory for images (default: images)
        #[arg(long, short = 'o', default_value = "images")]
        output: PathBuf,

        /// Path to cardsfolder (default: cardsfolder)
        #[arg(long, default_value = "cardsfolder")]
        cardsfolder: PathBuf,

        /// Download only cards from specific deck file(s)
        #[arg(long, short = 'd')]
        deck: Option<Vec<PathBuf>>,

        /// Image sizes to download (comma-separated: small,normal)
        #[arg(long, default_value = "small,normal")]
        sizes: String,

        /// Maximum concurrent downloads (default: 10)
        #[arg(long, default_value = "10")]
        concurrency: usize,

        /// Force re-download even if images exist
        #[arg(long)]
        force: bool,

        /// Delay between requests in milliseconds (default: 100)
        #[arg(long, default_value = "100")]
        rate_limit: u64,
    },

    /// Start a multiplayer game server
    #[cfg(feature = "network")]
    Server {
        /// Port to listen on (default: 17771)
        #[arg(long, short = 'p', default_value = "17771")]
        port: u16,

        /// Password required to join (empty for no password)
        #[arg(long)]
        password: Option<String>,

        /// Path to cardsfolder (default: cardsfolder)
        #[arg(long, default_value = "cardsfolder")]
        cardsfolder: PathBuf,

        /// Starting life total (default: 20)
        #[arg(long, default_value = "20")]
        starting_life: i32,

        /// Share deck lists between players (tournament mode)
        #[arg(long)]
        deck_visibility: bool,

        /// Fixed seed for game RNG (deterministic games). If not specified, uses random seed.
        #[arg(long)]
        seed: Option<u64>,

        /// Tag official game action logs with [GAMELOG TurnN STEP] prefix
        /// This enables comparing local vs network game logs for correctness
        #[arg(long)]
        tag_gamelogs: bool,

        /// Verbosity level for game output (0=silent, 1=minimal, 2=normal, 3=verbose)
        #[arg(long, default_value = "normal", short = 'v')]
        verbosity: VerbosityArg,

        /// Enable network debug mode for synchronization validation.
        /// When enabled, each protocol message includes state hashes and debug info.
        /// Server validates client's state hash matches its own after each choice.
        #[arg(long)]
        network_debug: bool,
    },

    /// Connect to a multiplayer game server
    #[cfg(feature = "network")]
    Connect {
        /// Deck file to use
        deck: PathBuf,

        /// Server address (host:port)
        #[arg(long, short = 's', default_value = "localhost:17771")]
        server: String,

        /// Server password (if required)
        #[arg(long)]
        password: Option<String>,

        /// Your player name
        #[arg(long, short = 'n', default_value = "Player")]
        name: String,

        /// Path to cardsfolder (default: cardsfolder)
        #[arg(long, default_value = "cardsfolder")]
        cardsfolder: PathBuf,

        /// Controller type (default: tui for human play)
        /// Available: zero, random, tui, fancy, heuristic, fixed
        #[arg(long, value_enum, default_value = "tui")]
        controller: ControllerType,

        /// Fixed script input (space or comma separated indices, e.g., "1 1 2" or "1,1,2")
        /// Required when --controller=fixed
        #[arg(long, value_name = "CHOICES")]
        fixed_inputs: Option<String>,

        /// Random seed for controller (for deterministic AI behavior)
        /// Can be a number or "from_entropy" for non-deterministic behavior
        #[arg(long)]
        seed_player: Option<SeedArg>,

        /// Enable visual stacking with diagonal offsets for fancy TUI (default: simple stacking)
        #[arg(long)]
        visual_stacks: bool,

        /// Verbosity level for game output (0=silent, 1=minimal, 2=normal, 3=verbose)
        #[arg(long, default_value = "normal", short = 'v')]
        verbosity: VerbosityArg,

        /// Enable gamelog tagging for equivalence testing.
        /// When enabled, the client's shadow GameLoop logs [GAMELOG] entries
        /// to stdout, which can be compared with server-side logs.
        #[arg(long)]
        tag_gamelogs: bool,

        /// Output file for client gamelogs (default: stdout).
        /// Use this to capture client gamelogs to a file for comparison.
        #[arg(long, value_name = "FILE")]
        gamelog_output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI first to check if we're in fancy TUI mode
    let cli = Cli::parse();

    // Initialize logging (controlled by RUST_LOG environment variable, defaults to Info level)
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.format_timestamp(None).format_target(true);

    // For Fancy TUI mode, redirect logs to a file since TUI takes over the screen
    let log_file_path = if matches!(
        cli.command,
        Commands::Tui {
            p1: ControllerType::Fancy,
            ..
        } | Commands::Tui {
            p2: ControllerType::Fancy,
            ..
        }
    ) {
        use std::fs::OpenOptions;
        let log_path = std::path::PathBuf::from("mtg_forge.log");
        let log_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)?;
        builder.target(env_logger::Target::Pipe(Box::new(log_file)));
        Some(log_path)
    } else {
        None
    };

    builder.init();

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
            screenshot_width,
            screenshot_height,
            seed,
            seed_p1,
            seed_p2,
            deck_seed,
            load_all_cards,
            verbosity,
            numeric_choices,
            visual_stacks,
            debug_state_hash,
            tag_gamelogs,
            stop_on_choice,
            stop_when_fixed_exhausted,
            snapshot_output,
            json,
            start_from,
            save_final_gamestate,
            log_tail,
            p1_draw,
            p2_draw,
        } => {
            // Convert json flag to SnapshotFormat
            let snapshot_format = if json {
                mtg_forge_rs::game::snapshot::SnapshotFormat::Json
            } else {
                mtg_forge_rs::game::snapshot::SnapshotFormat::Bincode
            };

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
                screenshot_width,
                screenshot_height,
                seed,
                seed_p1,
                seed_p2,
                deck_seed,
                load_all_cards,
                verbosity,
                numeric_choices,
                visual_stacks,
                debug_state_hash,
                tag_gamelogs,
                stop_on_choice,
                stop_when_fixed_exhausted,
                snapshot_output,
                snapshot_format,
                start_from,
                save_final_gamestate,
                log_tail,
                p1_draw,
                p2_draw,
            )
            .await?;

            // If we redirected logs to a file for Fancy TUI, print the location
            if let Some(log_path) = log_file_path {
                println!("\nLog file: {}", log_path.display());
            }
        }
        Commands::Profile { games, seed, deck } => run_profile(games, seed, deck).await?,
        Commands::Tourney {
            decks,
            games,
            seconds,
            p1,
            p2,
            seed,
            mirror_only,
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
            mtg_forge_rs::tournament::run_tourney(
                decks,
                games,
                seconds,
                p1_tourney,
                p2_tourney,
                seed_resolved,
                mirror_only,
            )
            .await?
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
            visual_stacks,
            debug_state_hash,
            tag_gamelogs,
            stop_on_choice,
            stop_when_fixed_exhausted,
            snapshot_output,
            json,
            save_final_gamestate,
            log_tail,
        } => {
            // Convert json flag to SnapshotFormat
            let snapshot_format = if json {
                mtg_forge_rs::game::snapshot::SnapshotFormat::Json
            } else {
                mtg_forge_rs::game::snapshot::SnapshotFormat::Bincode
            };

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
                visual_stacks,
                debug_state_hash,
                tag_gamelogs,
                stop_on_choice,
                stop_when_fixed_exhausted,
                snapshot_output,
                snapshot_format,
                save_final_gamestate,
                log_tail,
            )
            .await?
        }
        Commands::Stats {} => run_stats().await?,
        Commands::DeckBuild {
            deck_file,
            output_file,
            cardsfolder,
            start_year,
            end_year,
        } => run_deck_build(deck_file, output_file, cardsfolder, start_year, end_year).await?,
        Commands::ExportWasm { output, deck_globs } => run_export_wasm(output, deck_globs).await?,
        Commands::Download {
            output,
            cardsfolder,
            deck,
            sizes,
            concurrency,
            force,
            rate_limit,
        } => run_download(output, cardsfolder, deck, sizes, concurrency, force, rate_limit).await?,
        #[cfg(feature = "network")]
        Commands::Server {
            port,
            password,
            cardsfolder,
            starting_life,
            deck_visibility,
            seed,
            tag_gamelogs,
            verbosity,
            network_debug,
        } => {
            use mtg_forge_rs::game::VerbosityLevel;
            use mtg_forge_rs::network::{GameServer, ServerConfig};

            let verbosity_level: VerbosityLevel = verbosity.into();

            let config = ServerConfig {
                port,
                password: password.unwrap_or_default(),
                cardsfolder,
                starting_life,
                deck_visibility,
                seed,
                tag_gamelogs,
                verbosity: verbosity_level,
                network_debug,
                ..Default::default()
            };

            let mut server = GameServer::new(config);
            server
                .run()
                .await
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Server error: {}", e)))?;
        }
        #[cfg(feature = "network")]
        Commands::Connect {
            deck,
            server,
            password,
            name,
            cardsfolder,
            controller: controller_type,
            fixed_inputs,
            seed_player,
            visual_stacks,
            verbosity,
            tag_gamelogs,
            gamelog_output,
        } => {
            use mtg_forge_rs::core::PlayerId;
            use mtg_forge_rs::game::{HeuristicController, RichInputController, VerbosityLevel};
            use mtg_forge_rs::network::{ClientConfig, NetworkClient};

            // Validate controller type - Fancy/FancyFixed not supported for network
            // (Fancy TUI requires local terminal and doesn't work with synchronized GameLoop)
            if matches!(controller_type, ControllerType::FancyFixed | ControllerType::Fancy) {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--controller=fancy and --controller=fancy-fixed are not supported for network games".to_string(),
                ));
            }

            // Validate fixed controller has inputs
            if matches!(controller_type, ControllerType::Fixed) && fixed_inputs.is_none() {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--fixed-inputs is required when --controller=fixed".to_string(),
                ));
            }

            // Resolve seed
            let seed_resolved = seed_player.map(|s| s.resolve());

            let config = ClientConfig {
                server,
                password: password.unwrap_or_default(),
                player_name: name,
                deck_path: deck,
                cardsfolder,
            };

            let verbosity_level: VerbosityLevel = verbosity.into();

            let mut client = NetworkClient::new(config);
            client.set_verbosity(verbosity_level);
            client.set_visual_stacks(visual_stacks);
            // Note: network_debug is set by server via GameStarted message
            client.set_tag_gamelogs(tag_gamelogs);
            if let Some(ref path) = gamelog_output {
                client.set_gamelog_output(path.clone());
            }

            client
                .connect()
                .await
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Connection error: {}", e)))?;

            client
                .wait_for_game_start()
                .await
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Game start error: {}", e)))?;

            // Get our player ID from the client state
            let our_player_id = client.our_player_id().unwrap_or(PlayerId::new(0));

            // Create controller based on type and run the synchronized GameLoop
            let result: Option<PlayerId> = match controller_type {
                ControllerType::Zero => {
                    let ctrl = ZeroController::new(our_player_id);
                    client.run_game(ctrl).await
                }
                ControllerType::Random => {
                    let ctrl = if let Some(seed) = seed_resolved {
                        RandomController::with_seed(our_player_id, seed)
                    } else {
                        let entropy_seed = SeedArg::FromEntropy.resolve();
                        log::warn!(
                            "No seed provided for Random controller, using entropy: {}",
                            entropy_seed
                        );
                        RandomController::with_seed(our_player_id, entropy_seed)
                    };
                    client.run_game(ctrl).await
                }
                ControllerType::Tui => {
                    let ctrl = InteractiveController::new(our_player_id);
                    client.run_game(ctrl).await
                }
                ControllerType::Heuristic => {
                    let ctrl = HeuristicController::new(our_player_id);
                    client.run_game(ctrl).await
                }
                ControllerType::Fixed => {
                    let script = parse_fixed_inputs(fixed_inputs.as_ref().unwrap()).map_err(|e| {
                        mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --fixed-inputs: {}", e))
                    })?;
                    let ctrl = RichInputController::new(our_player_id, script);
                    client.run_game(ctrl).await
                }
                ControllerType::Fancy | ControllerType::FancyFixed => unreachable!(), // Already validated above
            }
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Game error: {}", e)))?;

            match result {
                Some(winner) => {
                    if winner == our_player_id {
                        log::info!("Game ended. You won!");
                    } else {
                        log::info!("Game ended. You lost.");
                    }
                }
                None => log::info!("Game ended in a draw"),
            }

            client.disconnect().await.ok();
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
    screenshot_width: u16,
    screenshot_height: u16,
    seed: Option<SeedArg>,
    seed_p1: Option<SeedArg>,
    seed_p2: Option<SeedArg>,
    deck_seed: Option<SeedArg>,
    load_all_cards: bool,
    verbosity: VerbosityArg,
    numeric_choices: bool,
    visual_stacks: bool,
    debug_state_hash: bool,
    tag_gamelogs: bool,
    stop_on_choice: Option<String>,
    stop_when_fixed_exhausted: bool,
    snapshot_output: PathBuf,
    snapshot_format: mtg_forge_rs::game::snapshot::SnapshotFormat,
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
    let deck_seed_resolved = deck_seed.map(|s| s.resolve());

    if !suppress_output {
        log::info!("=== MTG Forge Rust - Text UI Mode ===\n");
    }

    // Parse stop condition if provided
    let stop_condition = if let Some(ref stop_str) = stop_on_choice {
        let condition = StopCondition::parse(stop_str)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --stop-on-choice: {}", e)))?;
        if !suppress_output {
            log::info!("Stop condition: {:?}", condition);
            log::info!("Snapshot output: {}\n", snapshot_output.display());
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
    let cardsfolder = find_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    // Load snapshot early if resuming, so we can extract both game state and player-specific choices
    let loaded_snapshot: Option<GameSnapshot> = if let Some(ref snapshot_file) = start_from {
        let snapshot = GameSnapshot::load_from_file(snapshot_file, snapshot_format)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to load snapshot: {}", e)))?;
        Some(snapshot)
    } else {
        None
    };

    let snapshot_turn_number: Option<u32> = loaded_snapshot.as_ref().map(|s| s.turn_number);

    let mut game = if let Some(ref snapshot) = loaded_snapshot {
        // Load game from snapshot
        if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
            log::info!("Loading snapshot from: {}", start_from.as_ref().unwrap().display());
            log::info!("  Turn number: {}", snapshot.turn_number);
            log::info!("  Intra-turn choices to replay: {}", snapshot.choice_count());
            log::info!("Game loaded from snapshot!\n");
        }

        // Note: We don't need to load cards for snapshots since the GameState
        // already contains all the card data
        snapshot.game_state.clone()
    } else if let Some(puzzle_file) = puzzle_path {
        // Load game from puzzle file
        if !suppress_output {
            log::info!("Loading puzzle file: {}", puzzle_file.display());
        }
        let puzzle_contents = std::fs::read_to_string(&puzzle_file)?;
        let puzzle = PuzzleFile::parse(&puzzle_contents)?;
        if !suppress_output {
            log::info!("  Puzzle: {}", puzzle.metadata.name);
            log::info!("  Goal: {:?}", puzzle.metadata.goal);
            log::info!("  Difficulty: {:?}\n", puzzle.metadata.difficulty);

            // Load cards needed for puzzle
            log::info!("Loading card database...");
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
            log::info!("  Loaded {count} cards");
            log::info!("  (Loading time: {:.2}ms)", duration.as_secs_f64() * 1000.0);

            log::info!("Initializing game from puzzle...");
        }
        load_puzzle_into_game(&puzzle, &card_db).await?
    } else {
        // Load game from deck files
        let deck1_path = deck1_path.expect("deck1 required when not loading from puzzle");
        // If deck2 not provided, use deck1 for both players
        let deck2_path = deck2_path.as_ref().unwrap_or(&deck1_path);

        if !suppress_output {
            log::info!("Loading deck files...");
        }
        let deck1 = DeckLoader::load_from_file(&deck1_path)?;
        let deck2 = DeckLoader::load_from_file(deck2_path)?;

        if !suppress_output {
            if deck2_path == &deck1_path {
                log::info!("  Using same deck for both players: {} cards", deck1.total_cards());
            } else {
                log::info!("  Player 1: {} cards", deck1.total_cards());
                log::info!("  Player 2: {} cards", deck2.total_cards());
            }
            log::info!("");

            // Load cards based on mode
            log::info!("Loading card database...");
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
            log::info!("  Loaded {count} cards");
            log::info!("  (Loading time: {:.2}ms)", duration.as_secs_f64() * 1000.0);

            // Initialize game
            log::info!("Initializing game...");
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
            log::info!("Using random seed: {seed_value}");
        }
    }

    // Report controller seeds if set
    if !suppress_output {
        if let Some(p1_seed_value) = seed_p1_resolved {
            log::info!("Using explicit P1 controller seed: {p1_seed_value}");
        } else if let Some(seed_value) = seed_resolved {
            log::info!(
                "Using derived P1 controller seed: {} (from master seed)",
                seed_value.wrapping_add(0x1234_5678_9ABC_DEF0)
            );
        }

        if let Some(p2_seed_value) = seed_p2_resolved {
            log::info!("Using explicit P2 controller seed: {p2_seed_value}");
        } else if let Some(seed_value) = seed_resolved {
            log::info!(
                "Using derived P2 controller seed: {} (from master seed)",
                seed_value.wrapping_add(0xFEDC_BA98_7654_3210)
            );
        }
    }

    // Enable numeric choices mode if requested
    if numeric_choices {
        game.logger.set_numeric_choices(true);
        if !suppress_output {
            log::info!("Numeric choices mode: enabled");
        }
    }

    // Enable state hash debugging if requested
    if debug_state_hash {
        game.logger.set_debug_state_hash(true);
        if !suppress_output {
            log::info!("State hash debugging: enabled");
        }
    }

    // Enable gamelog tagging if requested
    if tag_gamelogs {
        game.logger.set_tag_gamelogs(true);
        if !suppress_output {
            log::info!("Gamelog tagging: enabled");
        }
    }

    if !suppress_output {
        log::info!("Game initialized!");
        log::info!("  Player 1: {} ({p1_type:?})", p1_name);
        log::info!("  Player 2: {} ({p2_type:?})\n", p2_name);
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
                        log::info!("Player 1 Random controller restored from snapshot");
                    }
                    Box::new(random_controller.clone())
                } else if let Some(p1_seed) = p1_controller_seed {
                    // No saved state, create fresh controller with seed
                    Box::new(RandomController::with_seed(p1_id, p1_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        log::warn!(
                            "Warning: No seed provided for P1 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        log::warn!("  To make this deterministic, use --seed or --seed-p1");
                    }
                    Box::new(RandomController::with_seed(p1_id, entropy_seed))
                }
            } else if let Some(p1_seed) = p1_controller_seed {
                Box::new(RandomController::with_seed(p1_id, p1_seed))
            } else {
                // No seed provided - generate from entropy with warning
                let entropy_seed = SeedArg::FromEntropy.resolve();
                if !suppress_output {
                    log::warn!(
                        "Warning: No seed provided for P1 Random controller, using entropy: {}",
                        entropy_seed
                    );
                    log::warn!("  To make this deterministic, use --seed or --seed-p1");
                }
                Box::new(RandomController::with_seed(p1_id, entropy_seed))
            }
        }
        ControllerType::Tui => Box::new(InteractiveController::with_numeric_choices(p1_id, numeric_choices)),
        ControllerType::Fancy => Box::new(
            FancyTuiController::new(p1_id, visual_stacks)
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to initialize Fancy TUI: {}", e)))?,
        ),
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
                        log::info!(
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
        ControllerType::FancyFixed => {
            use mtg_forge_rs::game::FancyFixedController;

            // FancyFixed requires --p1-fixed-inputs
            if let Some(input) = &p1_fixed_inputs {
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p1-fixed-inputs: {}", e))
                })?;

                // Determine screenshot directory from snapshot-output or use current.game
                let screenshot_dir = if true {
                    snapshot_output.parent().map(|p| p.to_path_buf())
                } else {
                    None
                };

                Box::new(FancyFixedController::with_size(
                    p1_id,
                    script,
                    screenshot_dir,
                    screenshot_width,
                    screenshot_height,
                )?)
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p1-fixed-inputs is required when --p1=fancy-fixed".to_string(),
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
                        log::info!("Player 2 Random controller restored from snapshot");
                    }
                    Box::new(random_controller.clone())
                } else if let Some(p2_seed) = p2_controller_seed {
                    // No saved state, create fresh controller with seed
                    Box::new(RandomController::with_seed(p2_id, p2_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        log::warn!(
                            "Warning: No seed provided for P2 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        log::warn!("  To make this deterministic, use --seed or --seed-p2");
                    }
                    Box::new(RandomController::with_seed(p2_id, entropy_seed))
                }
            } else if let Some(p2_seed) = p2_controller_seed {
                Box::new(RandomController::with_seed(p2_id, p2_seed))
            } else {
                // No seed provided - generate from entropy with warning
                let entropy_seed = SeedArg::FromEntropy.resolve();
                if !suppress_output {
                    log::warn!(
                        "Warning: No seed provided for P2 Random controller, using entropy: {}",
                        entropy_seed
                    );
                    log::warn!("  To make this deterministic, use --seed or --seed-p2");
                }
                Box::new(RandomController::with_seed(p2_id, entropy_seed))
            }
        }
        ControllerType::Tui => Box::new(InteractiveController::with_numeric_choices(p2_id, numeric_choices)),
        ControllerType::Fancy => {
            // Fancy TUI is only available for Player 1
            if !suppress_output {
                log::warn!("Warning: Fancy TUI controller is only available for Player 1");
                log::warn!("  Using regular TUI controller for Player 2 instead");
            }
            Box::new(InteractiveController::with_numeric_choices(p2_id, numeric_choices))
        }
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
                        log::info!(
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
        ControllerType::FancyFixed => {
            use mtg_forge_rs::game::FancyFixedController;

            // FancyFixed requires --p2-fixed-inputs
            if let Some(input) = &p2_fixed_inputs {
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p2-fixed-inputs: {}", e))
                })?;

                // Determine screenshot directory from snapshot-output
                let screenshot_dir = if true {
                    snapshot_output.parent().map(|p| p.to_path_buf())
                } else {
                    None
                };

                Box::new(FancyFixedController::with_size(
                    p2_id,
                    script,
                    screenshot_dir,
                    screenshot_width,
                    screenshot_height,
                )?)
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p2-fixed-inputs is required when --p2=fancy-fixed".to_string(),
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
            // Check if base controller is Fixed or FancyFixed - don't wrap if it is
            let is_fixed = matches!(p1_type, ControllerType::Fixed | ControllerType::FancyFixed);
            if is_fixed {
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!("Player 1 using Fixed/FancyFixed controller (skipping Replay wrapper)");
                }
                base_controller1
            } else {
                let p1_replay_choices = snapshot.extract_replay_choices_for_player(p1_id);
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!("Player 1 will replay {} intra-turn choices", p1_replay_choices.len());
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
            // Check if base controller is Fixed or FancyFixed - don't wrap if it is
            let is_fixed = matches!(p2_type, ControllerType::Fixed | ControllerType::FancyFixed);
            if is_fixed {
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!("Player 2 using Fixed/FancyFixed controller (skipping Replay wrapper)");
                }
                base_controller2
            } else {
                let p2_replay_choices = snapshot.extract_replay_choices_for_player(p2_id);
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!("Player 2 will replay {} intra-turn choices", p2_replay_choices.len());
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
            log::info!("=== Continuing Game ===\n");
        } else {
            log::info!("=== Starting Game ===\n");
        }
    }

    // Enable log tail mode if requested (captures logs to buffer)
    // Must be done BEFORE creating game loop since loop borrows game mutably
    if log_tail.is_some() {
        // Use Both mode to capture AND output to stdout (not Memory which suppresses stdout)
        game.logger
            .set_output_mode(mtg_forge_rs::game::logger::OutputMode::Both);
    }

    // Enable memory-only logging if fancy TUI is being used (prevents screen flickering)
    let is_fancy_tui = matches!(p1_type, ControllerType::Fancy) || matches!(p2_type, ControllerType::Fancy);
    if is_fancy_tui {
        game.logger
            .set_output_mode(mtg_forge_rs::game::logger::OutputMode::Memory);
    }

    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(verbosity)
        .with_snapshot_format(snapshot_format);

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
            log::info!("Baseline choice count (from snapshot): {}", baseline_count);
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
            log::info!("Replay mode enabled: {} choices to replay", replay_choice_count);
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

    // Set separate deck seed for shuffling if provided (--deck-seed)
    // This allows running multiple games with different RNG but same initial hands
    if let Some(deck_seed) = deck_seed_resolved {
        // If deck_seed is set, seed_resolved becomes the game seed after shuffling
        // If seed is not set but deck_seed is, the RNG stays at deck_seed for the whole game
        game_loop = game_loop.with_deck_seed(deck_seed, seed_resolved);
        if !suppress_output {
            log::info!("Using deck seed for shuffle: {deck_seed}");
            if let Some(game_seed) = seed_resolved {
                log::info!("Using game seed after shuffle: {game_seed}");
            }
        }
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
            if let Ok(mut snapshot) = GameSnapshot::load_from_file(&snapshot_output, snapshot_format) {
                // Deserialize JSON back to ControllerState (Fixed or Random) if present
                snapshot.p1_controller_state = p1_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                log::error!("Failed to deserialize P1 controller state: {}", e);
                                log::error!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });
                snapshot.p2_controller_state = p2_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                log::error!("Failed to deserialize P2 controller state: {}", e);
                                log::error!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });

                if let Err(e) = snapshot.save_to_file(&snapshot_output, snapshot_format) {
                    log::warn!("Warning: Failed to update snapshot with controller state: {}", e);
                } else if verbosity >= VerbosityLevel::Verbose {
                    log::info!("Snapshot updated with controller state");
                }
            }
        }
    }

    // Display results (suppress for snapshot exits)
    if verbosity >= VerbosityLevel::Minimal && result.end_reason != GameEndReason::Snapshot {
        log::info!("\n=== Game Over ===");
        match result.winner {
            Some(winner_id) => {
                let winner = game.get_player(winner_id)?;
                log::info!("Winner: {}", winner.name);
            }
            None => {
                log::info!("Game ended in a draw");
            }
        }
        log::info!("Turns played: {}", result.turns_played);
        log::info!("Reason: {:?}", result.end_reason);

        // Final state
        log::info!("\n=== Final State ===");
        for player in game.players.iter() {
            log::info!("  {}: {} life", player.name, player.life);
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
                .save_to_file(&final_state_path, snapshot_format)
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to save final gamestate: {}", e)))?;

            if verbosity >= VerbosityLevel::Verbose {
                log::info!("\nFinal game state saved to: {}", final_state_path.display());
            }
        }
    }

    // Save buffered logs to file if fancy TUI was used (run_tui function)
    if is_fancy_tui {
        use std::io::Write;
        let logs = game.logger.logs();
        let log_count = logs.len();

        if log_count > 0 {
            // Create temp file for logs
            let temp_dir = std::env::temp_dir();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let log_path = temp_dir.join(format!("mtg_forge_game_{}.log", timestamp));

            // Write logs to file
            let mut file = std::fs::File::create(&log_path)?;
            for entry in logs.iter() {
                writeln!(file, "{}", entry.message)?;
            }

            log::info!("\n>>> Game log saved: {} lines written to:", log_count);
            log::info!("    {}", log_path.display());
        }
    }

    Ok(())
}

/// Run profiling games
async fn run_profile(iterations: usize, seed: u64, deck_path: PathBuf) -> Result<()> {
    log::info!("=== MTG Forge Rust - Profiling Mode ===\n");

    // Load deck
    log::info!("Loading deck...");
    let deck = DeckLoader::load_from_file(&deck_path)?;
    log::info!("  Deck: {} cards", deck.total_cards());

    // Create card database (lazy loading - only loads cards on-demand)
    let cardsfolder = find_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    // Prefetch deck cards (not all 31k cards, just what we need)
    let start = std::time::Instant::now();
    let unique_names = deck.unique_card_names();
    let (count, _) = card_db.load_cards(&unique_names).await?;
    let duration = start.elapsed();
    log::info!("  Loaded {count} cards in {:.2}ms\n", duration.as_secs_f64() * 1000.0);

    log::info!("Profiling game execution...");
    log::info!("Running {iterations} games with seed {seed}");
    log::info!("");

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
            log::info!("Completed {} games", i + 1);
        }
    }

    log::info!("");
    log::info!("Profiling complete! {iterations} games executed.");
    log::info!("");
    log::info!("For heap profiling:");
    log::info!("  cargo heaptrack --bin mtg -- profile --games {iterations} --seed {seed}");
    log::info!("  Or: make heapprofile");
    log::info!("");
    log::info!("For CPU profiling:");
    log::info!("  cargo flamegraph --bin mtg -- profile --games {iterations} --seed {seed}");
    log::info!("  Or: make profile");

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
    visual_stacks: bool,
    debug_state_hash: bool,
    tag_gamelogs: bool,
    stop_on_choice: Option<String>,
    stop_when_fixed_exhausted: bool,
    snapshot_output: PathBuf,
    snapshot_format: mtg_forge_rs::game::snapshot::SnapshotFormat,
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
        log::info!("=== MTG Forge Rust - Resume Mode ===\n");
    }

    // Parse stop condition if provided
    let stop_condition = if let Some(ref stop_str) = stop_on_choice {
        let condition = StopCondition::parse(stop_str)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --stop-on-choice: {}", e)))?;
        if !suppress_output {
            log::info!("Stop condition: {:?}", condition);
            log::info!("Snapshot output: {}\n", snapshot_output.display());
        }
        Some(condition)
    } else {
        None
    };

    // Load snapshot (always required for resume mode)
    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        log::info!("Loading snapshot from: {}", snapshot_file.display());
    }

    let snapshot = GameSnapshot::load_from_file(&snapshot_file, snapshot_format)
        .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to load snapshot: {}", e)))?;

    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        log::info!("  Turn number: {}", snapshot.turn_number);
        log::info!("  Intra-turn choices to replay: {}", snapshot.choice_count());
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
            mtg_forge_rs::game::ControllerType::FancyFixed => ControllerType::FancyFixed,
            // Remote/Network controllers can't be restored from snapshot - they require network
            mtg_forge_rs::game::ControllerType::Remote => ControllerType::Zero,
            mtg_forge_rs::game::ControllerType::Network => ControllerType::Zero,
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
            mtg_forge_rs::game::ControllerType::FancyFixed => ControllerType::FancyFixed,
            // Remote/Network controllers can't be restored from snapshot - they require network
            mtg_forge_rs::game::ControllerType::Remote => ControllerType::Zero,
            mtg_forge_rs::game::ControllerType::Network => ControllerType::Zero,
        }
    });

    // Print what's being restored vs overridden
    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        if override_p1.is_some() {
            log::info!("Player 1 controller: OVERRIDDEN to {:?}", p1_type);
        } else {
            log::info!("Player 1 controller: restored from snapshot ({:?})", p1_type);
        }

        if override_p2.is_some() {
            log::info!("Player 2 controller: OVERRIDDEN to {:?}", p2_type);
        } else {
            log::info!("Player 2 controller: restored from snapshot ({:?})", p2_type);
        }

        if override_seed.is_some() {
            log::info!("Game engine seed: OVERRIDDEN to {}", override_seed_resolved.unwrap());
        } else {
            log::info!("Game engine seed: restored from snapshot");
        }

        log::info!("Game loaded from snapshot!\n");
    }

    // Restore game state from snapshot
    let mut game = snapshot.game_state.clone();

    // Override game engine seed if requested
    if let Some(seed_value) = override_seed_resolved {
        game.seed_rng(seed_value);
        if !suppress_output {
            log::info!("Overriding game engine seed: {seed_value}");
        }
    }

    // Enable numeric choices mode if requested
    if numeric_choices {
        game.logger.set_numeric_choices(true);
        if !suppress_output {
            log::info!("Numeric choices mode: enabled");
        }
    }

    // Enable state hash debugging if requested
    if debug_state_hash {
        game.logger.set_debug_state_hash(true);
        if !suppress_output {
            log::info!("State hash debugging: enabled");
        }
    }

    // Enable gamelog tagging if requested
    if tag_gamelogs {
        game.logger.set_tag_gamelogs(true);
        if !suppress_output {
            log::info!("Gamelog tagging: enabled");
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
        log::info!("  Player 1: {} ({p1_type:?})", p1_name);
        log::info!("  Player 2: {} ({p2_type:?})\n", p2_name);
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
                        log::info!("Player 1 Random controller: fresh with seed {}", p1_seed);
                    }
                    Box::new(RandomController::with_seed(p1_id, p1_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        log::warn!(
                            "Warning: No seed provided for P1 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        log::warn!("  To make this deterministic, use --override-seed or --override-seed-p1");
                    }
                    Box::new(RandomController::with_seed(p1_id, entropy_seed))
                }
            } else {
                // Restore from snapshot
                if let Some(mtg_forge_rs::game::ControllerState::Random(random_controller)) =
                    &snapshot.p1_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        log::info!("Player 1 Random controller: restored from snapshot");
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
        ControllerType::Fancy => Box::new(
            FancyTuiController::new(p1_id, visual_stacks)
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to initialize Fancy TUI: {}", e)))?,
        ),
        ControllerType::Heuristic => Box::new(HeuristicController::new(p1_id)),
        ControllerType::Fixed => {
            // Priority: CLI --p1-fixed-inputs > snapshot state > error
            if let Some(input) = &p1_fixed_inputs {
                // CLI override - use provided script
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p1-fixed-inputs: {}", e))
                })?;
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!("Player 1 Fixed controller: fresh with {} commands", script.len());
                }
                Box::new(RichInputController::new(p1_id, script))
            } else if let Some(mtg_forge_rs::game::ControllerState::Fixed(fixed_controller)) =
                &snapshot.p1_controller_state
            {
                // Restore from snapshot
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!(
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
        ControllerType::FancyFixed => {
            use mtg_forge_rs::game::FancyFixedController;

            // FancyFixed requires --p1-fixed-inputs
            if let Some(input) = &p1_fixed_inputs {
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p1-fixed-inputs: {}", e))
                })?;

                // FancyFixed does not support snapshot restoration yet
                let screenshot_dir = None; // Default to ./screenshots/

                Box::new(FancyFixedController::new(p1_id, script, screenshot_dir)?)
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p1-fixed-inputs is required when --override-p1=fancy-fixed".to_string(),
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
                        log::info!("Player 2 Random controller: fresh with seed {}", p2_seed);
                    }
                    Box::new(RandomController::with_seed(p2_id, p2_seed))
                } else {
                    // No seed provided - generate from entropy with warning
                    let entropy_seed = SeedArg::FromEntropy.resolve();
                    if !suppress_output {
                        log::warn!(
                            "Warning: No seed provided for P2 Random controller, using entropy: {}",
                            entropy_seed
                        );
                        log::warn!("  To make this deterministic, use --override-seed or --override-seed-p2");
                    }
                    Box::new(RandomController::with_seed(p2_id, entropy_seed))
                }
            } else {
                // Restore from snapshot
                if let Some(mtg_forge_rs::game::ControllerState::Random(random_controller)) =
                    &snapshot.p2_controller_state
                {
                    if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                        log::info!("Player 2 Random controller: restored from snapshot");
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
        ControllerType::Fancy => {
            // Fancy TUI is only available for Player 1
            if !suppress_output {
                log::warn!("Warning: Fancy TUI controller is only available for Player 1");
                log::warn!("  Using regular TUI controller for Player 2 instead");
            }
            Box::new(InteractiveController::with_numeric_choices(p2_id, numeric_choices))
        }
        ControllerType::Heuristic => Box::new(HeuristicController::new(p2_id)),
        ControllerType::Fixed => {
            // Priority: CLI --p2-fixed-inputs > snapshot state > error
            if let Some(input) = &p2_fixed_inputs {
                // CLI override - use provided script
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p2-fixed-inputs: {}", e))
                })?;
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!("Player 2 Fixed controller: fresh with {} commands", script.len());
                }
                Box::new(RichInputController::new(p2_id, script))
            } else if let Some(mtg_forge_rs::game::ControllerState::Fixed(fixed_controller)) =
                &snapshot.p2_controller_state
            {
                // Restore from snapshot
                if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                    log::info!(
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
        ControllerType::FancyFixed => {
            use mtg_forge_rs::game::FancyFixedController;

            // FancyFixed requires --p2-fixed-inputs
            if let Some(input) = &p2_fixed_inputs {
                let script = parse_fixed_inputs(input).map_err(|e| {
                    mtg_forge_rs::MtgError::InvalidAction(format!("Error parsing --p2-fixed-inputs: {}", e))
                })?;

                // FancyFixed does not support snapshot restoration yet
                let screenshot_dir = None; // Default to ./screenshots/

                Box::new(FancyFixedController::new(p2_id, script, screenshot_dir)?)
            } else {
                return Err(mtg_forge_rs::MtgError::InvalidAction(
                    "--p2-fixed-inputs is required when --override-p2=fancy-fixed".to_string(),
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
                log::info!("Player 1 using Fixed controller (skipping Replay wrapper)");
            }
            base_controller1
        } else {
            let p1_replay_choices = snapshot.extract_replay_choices_for_player(p1_id);
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                log::info!("Player 1 will replay {} intra-turn choices", p1_replay_choices.len());
            }
            Box::new(mtg_forge_rs::game::ReplayController::new(
                p1_id,
                base_controller1,
                p1_replay_choices,
            ))
        }
    };

    let mut controller2: Box<dyn mtg_forge_rs::game::controller::PlayerController> = {
        let is_fixed = matches!(p2_type, ControllerType::Fixed | ControllerType::FancyFixed);
        if is_fixed {
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                log::info!("Player 2 using Fixed/FancyFixed controller (skipping Replay wrapper)");
            }
            base_controller2
        } else {
            let p2_replay_choices = snapshot.extract_replay_choices_for_player(p2_id);
            if should_print(verbosity, VerbosityLevel::Verbose, suppress_output) {
                log::info!("Player 2 will replay {} intra-turn choices", p2_replay_choices.len());
            }
            Box::new(mtg_forge_rs::game::ReplayController::new(
                p2_id,
                base_controller2,
                p2_replay_choices,
            ))
        }
    };

    if should_print(verbosity, VerbosityLevel::Minimal, suppress_output) {
        log::info!("=== Resuming Game ===\n");
    }

    // Enable log tail mode if requested (captures logs to buffer)
    // Must be done BEFORE creating game loop since loop borrows game mutably
    if log_tail.is_some() {
        // Use Both mode to capture AND output to stdout (not Memory which suppresses stdout)
        game.logger
            .set_output_mode(mtg_forge_rs::game::logger::OutputMode::Both);
    }

    // Enable memory-only logging if fancy TUI is being used (prevents screen flickering)
    let is_fancy_tui_resume = matches!(p1_type, ControllerType::Fancy) || matches!(p2_type, ControllerType::Fancy);
    if is_fancy_tui_resume {
        game.logger
            .set_output_mode(mtg_forge_rs::game::logger::OutputMode::Memory);
    }

    // Run the game loop
    let mut game_loop = GameLoop::new(&mut game)
        .with_verbosity(verbosity)
        .with_snapshot_format(snapshot_format);

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
            log::info!("Baseline choice count (from snapshot): {}", baseline_count);
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
            log::info!("Replay mode enabled: {} choices to replay", replay_choice_count);
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
            if let Ok(mut snapshot) = GameSnapshot::load_from_file(&snapshot_output, snapshot_format) {
                // Deserialize JSON back to ControllerState (Fixed or Random) if present
                snapshot.p1_controller_state = p1_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                log::error!("Failed to deserialize P1 controller state: {}", e);
                                log::error!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });
                snapshot.p2_controller_state = p2_state_json.and_then(|json| {
                    serde_json::from_value(json.clone())
                        .map_err(|e| {
                            if verbosity >= VerbosityLevel::Verbose {
                                log::error!("Failed to deserialize P2 controller state: {}", e);
                                log::error!("  JSON: {}", json);
                            }
                            e
                        })
                        .ok()
                });

                if let Err(e) = snapshot.save_to_file(&snapshot_output, snapshot_format) {
                    log::warn!("Warning: Failed to update snapshot with controller state: {}", e);
                } else if verbosity >= VerbosityLevel::Verbose {
                    log::info!("Snapshot updated with controller state");
                }
            }
        }
    }

    // Display results (suppress for snapshot exits)
    if verbosity >= VerbosityLevel::Minimal && result.end_reason != GameEndReason::Snapshot {
        log::info!("\n=== Game Over ===");
        match result.winner {
            Some(winner_id) => {
                let winner = game.get_player(winner_id)?;
                log::info!("Winner: {}", winner.name);
            }
            None => {
                log::info!("Game ended in a draw");
            }
        }
        log::info!("Turns played: {}", result.turns_played);
        log::info!("Reason: {:?}", result.end_reason);

        // Final state
        log::info!("\n=== Final State ===");
        for player in game.players.iter() {
            log::info!("  {}: {} life", player.name, player.life);
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
                .save_to_file(&final_state_path, snapshot_format)
                .map_err(|e| mtg_forge_rs::MtgError::InvalidAction(format!("Failed to save final gamestate: {}", e)))?;

            if verbosity >= VerbosityLevel::Verbose {
                log::info!("\nFinal game state saved to: {}", final_state_path.display());
            }
        }
    }

    // Save buffered logs to file if fancy TUI was used (run_resume function)
    if is_fancy_tui_resume {
        use std::io::Write;
        let logs = game.logger.logs();
        let log_count = logs.len();

        if log_count > 0 {
            // Create temp file for logs
            let temp_dir = std::env::temp_dir();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let log_path = temp_dir.join(format!("mtg_forge_game_{}.log", timestamp));

            // Write logs to file
            let mut file = std::fs::File::create(&log_path)?;
            for entry in logs.iter() {
                writeln!(file, "{}", entry.message)?;
            }

            log::info!("\n>>> Game log saved: {} lines written to:", log_count);
            log::info!("    {}", log_path.display());
        }
    }

    Ok(())
}

/// Run the fast deck entry mode TUI
async fn run_deck_build(
    deck_file: Option<PathBuf>,
    output_file: Option<PathBuf>,
    cardsfolder: PathBuf,
    start_year: Option<u16>,
    end_year: Option<u16>,
) -> Result<()> {
    use mtg_forge_rs::deck_builder::{run_deck_builder, DeckBuilderConfig};
    use mtg_forge_rs::loader::{AsyncCardDatabase as CardDatabase, CardEditionIndex};

    println!("=== MTG Forge - Fast Deck Builder ===\n");

    // Determine input and output files
    // If deck_file is provided, it's both input and output (unless output_file overrides)
    // If neither is provided, default to output.dck
    let (input_file, output_path) = match (deck_file, output_file) {
        (Some(deck), Some(out)) => (Some(deck.to_string_lossy().to_string()), out),
        (Some(deck), None) => (Some(deck.to_string_lossy().to_string()), deck),
        (None, Some(out)) => (None, out),
        (None, None) => (None, PathBuf::from("output.dck")),
    };

    // Verify cardsfolder exists
    if !cardsfolder.exists() {
        return Err(mtg_forge_rs::MtgError::InvalidDeckFormat(format!(
            "Cardsfolder not found: {:?}",
            cardsfolder
        )));
    }

    // Load edition data (for year filtering and showing release info in card details)
    let editions_dir = PathBuf::from("editions");
    let edition_index = if editions_dir.exists() {
        print!("Loading edition data...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        match CardEditionIndex::load_from_directory(&editions_dir) {
            Ok(index) => {
                println!(" {} sets, {} cards indexed", index.set_count(), index.card_count());
                Some(index)
            }
            Err(e) => {
                println!(" failed: {}", e);
                if start_year.is_some() || end_year.is_some() {
                    eprintln!("Warning: Year filtering disabled due to edition load error");
                }
                None
            }
        }
    } else {
        if start_year.is_some() || end_year.is_some() {
            eprintln!("Warning: editions/ directory not found, year filtering disabled");
        }
        None
    };

    // Load all cards (including definitions for card details display)
    println!("Loading card database from {:?}...", cardsfolder);
    let card_db = CardDatabase::new(cardsfolder);
    let (loaded, duration) = card_db.eager_load().await?;
    println!("Loaded {} cards in {:?}", loaded, duration);

    // Get card names and definitions
    let all_cards = card_db.all_cards().await;

    // Filter by year if edition index is available
    let filtered_cards: Vec<_> = if let Some(ref index) = edition_index {
        let before_count = all_cards.len();
        let filtered: Vec<_> = all_cards
            .into_iter()
            .filter(|c| index.card_in_year_range(c.name.as_str(), start_year, end_year))
            .collect();
        let after_count = filtered.len();
        println!(
            "Year filter ({}-{}): {} -> {} cards",
            start_year.map(|y| y.to_string()).unwrap_or_else(|| "any".to_string()),
            end_year.map(|y| y.to_string()).unwrap_or_else(|| "any".to_string()),
            before_count,
            after_count
        );
        filtered
    } else {
        all_cards.into_iter().collect()
    };

    let mut card_names: Vec<String> = filtered_cards.iter().map(|c| c.name.to_string()).collect();
    card_names.sort();

    // Build definitions map for card details
    let card_definitions: std::collections::HashMap<String, std::sync::Arc<mtg_forge_rs::loader::CardDefinition>> =
        filtered_cards.into_iter().map(|c| (c.name.to_string(), c)).collect();

    println!();

    let config = DeckBuilderConfig {
        output_file: output_path.to_string_lossy().to_string(),
        input_file,
        start_year,
        end_year,
    };

    run_deck_builder(config, card_names, card_definitions, edition_index).await
}

/// Print statistics about the card database
async fn run_stats() -> Result<()> {
    use std::collections::HashMap;

    log::info!("=== MTG Forge Rust - Card Database Statistics ===\n");

    // Find and load cardsfolder
    let cardsfolder = find_cardsfolder();
    let card_db = CardDatabase::new(cardsfolder);

    log::info!("Loading full card database...");
    let (loaded, duration) = card_db.eager_load().await?;
    log::info!("Successfully loaded {} cards in {:?}\n", loaded, duration);

    // Generate comprehensive statistics
    log::info!("=== Card Database Statistics ===");

    let all_cards = card_db.all_cards().await;

    // Card type distribution
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for card in &all_cards {
        for card_type in &card.types {
            *type_counts.entry(format!("{:?}", card_type)).or_insert(0) += 1;
        }
    }

    log::info!("\n--- Card Types ---");
    let mut type_vec: Vec<_> = type_counts.iter().collect();
    type_vec.sort_by(|a, b| b.1.cmp(a.1));
    for (card_type, count) in type_vec {
        log::info!("  {:12} {:6}", card_type, count);
    }

    // Color distribution
    let mut color_counts: HashMap<String, usize> = HashMap::new();
    for card in &all_cards {
        for color in &card.colors {
            *color_counts.entry(format!("{:?}", color)).or_insert(0) += 1;
        }
    }

    log::info!("\n--- Colors ---");
    let mut color_vec: Vec<_> = color_counts.iter().collect();
    color_vec.sort_by(|a, b| b.1.cmp(a.1));
    for (color, count) in color_vec {
        log::info!("  {:12} {:6}", color, count);
    }

    // Top subtypes
    let mut subtype_counts: HashMap<String, usize> = HashMap::new();
    for card in &all_cards {
        for subtype in &card.subtypes {
            *subtype_counts.entry(subtype.as_str().to_string()).or_insert(0) += 1;
        }
    }

    log::info!("\n--- Top 30 Subtypes ---");
    log::info!("  Total distinct subtypes: {}", subtype_counts.len());
    let mut subtype_vec: Vec<_> = subtype_counts.iter().collect();
    subtype_vec.sort_by(|a, b| b.1.cmp(a.1));
    for (subtype, count) in subtype_vec.iter().take(30) {
        log::info!("  {:20} {:6}", subtype, count);
    }

    // Keyword distribution
    let mut keyword_counts: HashMap<String, usize> = HashMap::new();
    for card in &all_cards {
        let instantiated = card.instantiate(mtg_forge_rs::core::CardId::new(0), mtg_forge_rs::core::PlayerId::new(0));

        // Count all keywords
        for keyword in instantiated.keywords.iter() {
            // For simple keywords (no args), just use Debug formatting
            if let Some(args) = instantiated.keywords.get_args(keyword) {
                // Complex keyword - strip parameter for aggregation
                let keyword_name = match args {
                    mtg_forge_rs::core::KeywordArgs::Madness { .. } => "Madness",
                    mtg_forge_rs::core::KeywordArgs::Flashback { .. } => "Flashback",
                    mtg_forge_rs::core::KeywordArgs::Kicker { .. } => "Kicker",
                    mtg_forge_rs::core::KeywordArgs::Cycling { .. } => "Cycling",
                    mtg_forge_rs::core::KeywordArgs::Equip { .. } => "Equip",
                    mtg_forge_rs::core::KeywordArgs::Morph { .. } => "Morph",
                    mtg_forge_rs::core::KeywordArgs::Evoke { .. } => "Evoke",
                    mtg_forge_rs::core::KeywordArgs::Buyback { .. } => "Buyback",
                    mtg_forge_rs::core::KeywordArgs::Echo { .. } => "Echo",
                    mtg_forge_rs::core::KeywordArgs::Suspend { .. } => "Suspend",
                    mtg_forge_rs::core::KeywordArgs::Enchant { .. } => "Enchant",
                    mtg_forge_rs::core::KeywordArgs::Landwalk { .. } => "Landwalk",
                    mtg_forge_rs::core::KeywordArgs::Affinity { .. } => "Affinity",
                    mtg_forge_rs::core::KeywordArgs::Protection { .. } => "Protection",
                    mtg_forge_rs::core::KeywordArgs::Offering { .. } => "Offering",
                    mtg_forge_rs::core::KeywordArgs::Champion { .. } => "Champion",
                    mtg_forge_rs::core::KeywordArgs::Amplify { .. } => "Amplify",
                    mtg_forge_rs::core::KeywordArgs::Annihilator { .. } => "Annihilator",
                    mtg_forge_rs::core::KeywordArgs::Bushido { .. } => "Bushido",
                    mtg_forge_rs::core::KeywordArgs::Fading { .. } => "Fading",
                    mtg_forge_rs::core::KeywordArgs::Vanishing { .. } => "Vanishing",
                    mtg_forge_rs::core::KeywordArgs::Dredge { .. } => "Dredge",
                    mtg_forge_rs::core::KeywordArgs::Modular { .. } => "Modular",
                    mtg_forge_rs::core::KeywordArgs::Absorb { .. } => "Absorb",
                    mtg_forge_rs::core::KeywordArgs::HexproofFrom { .. } => "Hexproof From",
                    mtg_forge_rs::core::KeywordArgs::PartnerWith { .. } => "Partner With",
                    mtg_forge_rs::core::KeywordArgs::Companion { .. } => "Companion",
                    // New cost-based keywords
                    mtg_forge_rs::core::KeywordArgs::AuraSwap { .. } => "Aura Swap",
                    mtg_forge_rs::core::KeywordArgs::Bestow { .. } => "Bestow",
                    mtg_forge_rs::core::KeywordArgs::Blitz { .. } => "Blitz",
                    mtg_forge_rs::core::KeywordArgs::CumulativeUpkeep { .. } => "Cumulative Upkeep",
                    mtg_forge_rs::core::KeywordArgs::Dash { .. } => "Dash",
                    mtg_forge_rs::core::KeywordArgs::Disguise { .. } => "Disguise",
                    mtg_forge_rs::core::KeywordArgs::Disturb { .. } => "Disturb",
                    mtg_forge_rs::core::KeywordArgs::Embalm { .. } => "Embalm",
                    mtg_forge_rs::core::KeywordArgs::Encore { .. } => "Encore",
                    mtg_forge_rs::core::KeywordArgs::Entwine { .. } => "Entwine",
                    mtg_forge_rs::core::KeywordArgs::Escalate { .. } => "Escalate",
                    mtg_forge_rs::core::KeywordArgs::Escape { .. } => "Escape",
                    mtg_forge_rs::core::KeywordArgs::Eternalize { .. } => "Eternalize",
                    mtg_forge_rs::core::KeywordArgs::Foretell { .. } => "Foretell",
                    mtg_forge_rs::core::KeywordArgs::Fortify { .. } => "Fortify",
                    mtg_forge_rs::core::KeywordArgs::Freerunning { .. } => "Freerunning",
                    mtg_forge_rs::core::KeywordArgs::Harmonize { .. } => "Harmonize",
                    mtg_forge_rs::core::KeywordArgs::LevelUp { .. } => "Level Up",
                    mtg_forge_rs::core::KeywordArgs::MayFlashCost { .. } => "MayFlashCost",
                    mtg_forge_rs::core::KeywordArgs::Megamorph { .. } => "Megamorph",
                    mtg_forge_rs::core::KeywordArgs::Miracle { .. } => "Miracle",
                    mtg_forge_rs::core::KeywordArgs::MoreThanMeetsTheEye { .. } => "More Than Meets The Eye",
                    mtg_forge_rs::core::KeywordArgs::Multikicker { .. } => "Multikicker",
                    mtg_forge_rs::core::KeywordArgs::Mutate { .. } => "Mutate",
                    mtg_forge_rs::core::KeywordArgs::Offspring { .. } => "Offspring",
                    mtg_forge_rs::core::KeywordArgs::Outlast { .. } => "Outlast",
                    mtg_forge_rs::core::KeywordArgs::Overload { .. } => "Overload",
                    mtg_forge_rs::core::KeywordArgs::Plot { .. } => "Plot",
                    mtg_forge_rs::core::KeywordArgs::Prowl { .. } => "Prowl",
                    mtg_forge_rs::core::KeywordArgs::Prototype { .. } => "Prototype",
                    mtg_forge_rs::core::KeywordArgs::Reconfigure { .. } => "Reconfigure",
                    mtg_forge_rs::core::KeywordArgs::Reflect { .. } => "Reflect",
                    mtg_forge_rs::core::KeywordArgs::Scavenge { .. } => "Scavenge",
                    mtg_forge_rs::core::KeywordArgs::Sneak { .. } => "Sneak",
                    mtg_forge_rs::core::KeywordArgs::Specialize { .. } => "Specialize",
                    mtg_forge_rs::core::KeywordArgs::Spectacle { .. } => "Spectacle",
                    mtg_forge_rs::core::KeywordArgs::Squad { .. } => "Squad",
                    mtg_forge_rs::core::KeywordArgs::Strive { .. } => "Strive",
                    mtg_forge_rs::core::KeywordArgs::Surge { .. } => "Surge",
                    mtg_forge_rs::core::KeywordArgs::Transfigure { .. } => "Transfigure",
                    mtg_forge_rs::core::KeywordArgs::Transmute { .. } => "Transmute",
                    mtg_forge_rs::core::KeywordArgs::Unearth { .. } => "Unearth",
                    mtg_forge_rs::core::KeywordArgs::Ward { .. } => "Ward",
                    mtg_forge_rs::core::KeywordArgs::Warp { .. } => "Warp",
                    mtg_forge_rs::core::KeywordArgs::WebSlinging { .. } => "Web-Slinging",
                    // New amount-based keywords
                    mtg_forge_rs::core::KeywordArgs::Afflict { .. } => "Afflict",
                    mtg_forge_rs::core::KeywordArgs::Afterlife { .. } => "Afterlife",
                    mtg_forge_rs::core::KeywordArgs::Bloodthirst { .. } => "Bloodthirst",
                    mtg_forge_rs::core::KeywordArgs::Casualty { .. } => "Casualty",
                    mtg_forge_rs::core::KeywordArgs::Crew { .. } => "Crew",
                    mtg_forge_rs::core::KeywordArgs::Fabricate { .. } => "Fabricate",
                    mtg_forge_rs::core::KeywordArgs::Frenzy { .. } => "Frenzy",
                    mtg_forge_rs::core::KeywordArgs::Graft { .. } => "Graft",
                    mtg_forge_rs::core::KeywordArgs::Hideaway { .. } => "Hideaway",
                    mtg_forge_rs::core::KeywordArgs::Mobilize { .. } => "Mobilize",
                    mtg_forge_rs::core::KeywordArgs::Poisonous { .. } => "Poisonous",
                    mtg_forge_rs::core::KeywordArgs::Rampage { .. } => "Rampage",
                    mtg_forge_rs::core::KeywordArgs::Renown { .. } => "Renown",
                    mtg_forge_rs::core::KeywordArgs::Ripple { .. } => "Ripple",
                    mtg_forge_rs::core::KeywordArgs::Saddle { .. } => "Saddle",
                    mtg_forge_rs::core::KeywordArgs::Soulshift { .. } => "Soulshift",
                    mtg_forge_rs::core::KeywordArgs::StartingIntensity { .. } => "Starting Intensity",
                    mtg_forge_rs::core::KeywordArgs::Station { .. } => "Station",
                    mtg_forge_rs::core::KeywordArgs::Toxic { .. } => "Toxic",
                    mtg_forge_rs::core::KeywordArgs::Tribute { .. } => "Tribute",
                    // Cost + amount keywords
                    mtg_forge_rs::core::KeywordArgs::Adapt { .. } => "Adapt",
                    mtg_forge_rs::core::KeywordArgs::Awaken { .. } => "Awaken",
                    mtg_forge_rs::core::KeywordArgs::Backup { .. } => "Backup",
                    mtg_forge_rs::core::KeywordArgs::Impending { .. } => "Impending",
                    mtg_forge_rs::core::KeywordArgs::Monstrosity { .. } => "Monstrosity",
                    mtg_forge_rs::core::KeywordArgs::Reinforce { .. } => "Reinforce",
                    // Cost + type keywords
                    mtg_forge_rs::core::KeywordArgs::Splice { .. } => "Splice",
                    mtg_forge_rs::core::KeywordArgs::Typecycling { .. } => "Typecycling",
                    // Type-based keywords
                    mtg_forge_rs::core::KeywordArgs::BandsWithOther { .. } => "Bands With Other",
                    // Special keywords
                    mtg_forge_rs::core::KeywordArgs::Emerge { .. } => "Emerge",
                    mtg_forge_rs::core::KeywordArgs::Firebending { .. } => "Firebending",
                    mtg_forge_rs::core::KeywordArgs::Ninjutsu { .. } => "Ninjutsu",
                    mtg_forge_rs::core::KeywordArgs::Partner => "Partner",
                    mtg_forge_rs::core::KeywordArgs::Craft { .. } => "Craft",
                    mtg_forge_rs::core::KeywordArgs::Devour { .. } => "Devour",
                    mtg_forge_rs::core::KeywordArgs::Haunt { .. } => "Haunt",
                    mtg_forge_rs::core::KeywordArgs::Replicate { .. } => "Replicate",
                    mtg_forge_rs::core::KeywordArgs::MayEffectFromOpeningHand { .. } => "May Effect From Opening Hand",
                    mtg_forge_rs::core::KeywordArgs::Mayhem { .. } => "Mayhem",
                    mtg_forge_rs::core::KeywordArgs::Recover { .. } => "Recover",
                    mtg_forge_rs::core::KeywordArgs::Visit { .. } => "Visit",
                    mtg_forge_rs::core::KeywordArgs::DeckLimit { .. } => "Deck Limit",
                    mtg_forge_rs::core::KeywordArgs::Dungeon { .. } => "Dungeon",
                    // Saga and Class keywords
                    mtg_forge_rs::core::KeywordArgs::Chapter { .. } => "Chapter",
                    mtg_forge_rs::core::KeywordArgs::Class { .. } => "Class",
                    // ETB keywords
                    mtg_forge_rs::core::KeywordArgs::ETBReplacement { .. } => "ETB Replacement",
                    mtg_forge_rs::core::KeywordArgs::EtbCounter { .. } => "ETB Counter",
                }
                .to_string();
                *keyword_counts.entry(keyword_name).or_insert(0) += 1;
            } else {
                // Simple keyword - use Debug formatting
                let keyword_name = format!("{:?}", keyword);
                *keyword_counts.entry(keyword_name).or_insert(0) += 1;
            }
        }
    }

    log::info!("\n--- Top 30 Keywords ---");
    log::info!("  Total distinct keywords: {}", keyword_counts.len());
    let mut keyword_vec: Vec<_> = keyword_counts.iter().collect();
    keyword_vec.sort_by(|a, b| b.1.cmp(a.1));
    for (keyword, count) in keyword_vec.iter().take(30) {
        log::info!("  {:25} {:6}", keyword, count);
    }

    // Ability statistics
    let cards_with_effects = all_cards.iter().filter(|c| !c.raw_abilities.is_empty()).count();
    let cards_with_keywords = all_cards.iter().filter(|c| !c.raw_keywords.is_empty()).count();

    log::info!("\n--- Ability Statistics ---");
    log::info!("  Cards with raw abilities:  {:6}", cards_with_effects);
    log::info!("  Cards with keywords:       {:6}", cards_with_keywords);

    // Trigger and activated ability counts
    let mut cards_with_triggers = 0;
    let mut cards_with_activated = 0;
    for card in &all_cards {
        let instantiated = card.instantiate(mtg_forge_rs::core::CardId::new(0), mtg_forge_rs::core::PlayerId::new(0));
        if !instantiated.triggers.is_empty() {
            cards_with_triggers += 1;
        }
        if !instantiated.activated_abilities.is_empty() {
            cards_with_activated += 1;
        }
    }

    log::info!("  Cards with triggers:       {:6}", cards_with_triggers);
    log::info!("  Cards with activated abs:  {:6}", cards_with_activated);

    // Mana cost distribution
    let mut cmc_counts = [0; 9]; // 0-7, and 8+ in index 8

    for card in &all_cards {
        let cmc = card.mana_cost.cmc();
        let index = if cmc >= 8 { 8 } else { cmc as usize };
        cmc_counts[index] += 1;
    }

    log::info!("\n--- Mana Cost Distribution ---");
    for (cmc, count) in cmc_counts.iter().enumerate() {
        if cmc < 8 {
            log::info!("  CMC {}:                     {:6}", cmc, count);
        } else {
            log::info!("  CMC 8+:                    {:6}", count);
        }
    }

    Ok(())
}

/// Export card database and decks for WASM browser builds
///
/// Creates bincode-serialized files for:
/// - All card definitions from cardsfolder
/// - Selected deck files matching the glob pattern
///
/// These files can be loaded by the WASM module in the browser.
async fn run_export_wasm(output: PathBuf, deck_globs: Vec<String>) -> Result<()> {
    use mtg_forge_rs::loader::CardLoader;
    use std::collections::HashMap;
    use std::fs;

    println!("=== MTG Forge Rust - WASM Export ===\n");

    // Create output directory if it doesn't exist
    fs::create_dir_all(&output).map_err(|e| {
        mtg_forge_rs::MtgError::IoError(std::io::Error::other(format!(
            "Failed to create output directory {}: {}",
            output.display(),
            e
        )))
    })?;

    // Find cardsfolder
    let cardsfolder = find_cardsfolder();
    println!("Loading card definitions from {}...", cardsfolder.display());

    // Load all card files directly from cardsfolder (using glob)
    let mut card_definitions: HashMap<String, mtg_forge_rs::loader::CardDefinition> = HashMap::new();
    let mut load_errors = 0;

    let card_pattern = format!("{}/**/*.txt", cardsfolder.display());
    for entry in glob::glob(&card_pattern)
        .map_err(|e| mtg_forge_rs::MtgError::InvalidCardFormat(format!("Invalid glob pattern: {}", e)))?
    {
        match entry {
            Ok(path) => {
                if path.is_file() {
                    match CardLoader::load_from_file(&path) {
                        Ok(def) => {
                            let card_name = def.name.as_str().to_string();
                            card_definitions.insert(card_name, def);
                        }
                        Err(e) => {
                            eprintln!("  Warning: Failed to load {}: {}", path.display(), e);
                            load_errors += 1;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("  Warning: Glob error: {}", e);
            }
        }
    }

    println!(
        "Loaded {} card definitions ({} errors)",
        card_definitions.len(),
        load_errors
    );

    // Serialize cards to bincode
    let cards_path = output.join("cards.bin");
    let cards_data = bincode::serialize(&card_definitions)
        .map_err(|e| mtg_forge_rs::MtgError::InvalidCardFormat(format!("Failed to serialize cards: {}", e)))?;
    fs::write(&cards_path, &cards_data).map_err(mtg_forge_rs::MtgError::IoError)?;
    println!(
        "\nExported {} cards to {} ({} bytes)",
        card_definitions.len(),
        cards_path.display(),
        cards_data.len()
    );

    // Find and load deck files matching the glob patterns
    println!("\nSearching for decks matching patterns:");
    for pattern in &deck_globs {
        println!("  - {}", pattern);
    }
    let mut decks: HashMap<String, mtg_forge_rs::loader::DeckList> = HashMap::new();

    // Use glob to find matching deck files from all patterns
    for deck_glob in &deck_globs {
        for entry in glob::glob(deck_glob)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidDeckFormat(format!("Invalid glob pattern: {}", e)))?
        {
            match entry {
                Ok(path) => {
                    if path.is_file() {
                        match DeckLoader::load_from_file(&path) {
                            Ok(deck) => {
                                // Use filename without extension as deck name
                                let deck_name = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                println!("  Loaded deck: {} ({} cards)", deck_name, deck.total_cards());
                                decks.insert(deck_name, deck);
                            }
                            Err(e) => {
                                eprintln!("  Warning: Failed to load deck {}: {}", path.display(), e);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  Warning: Glob error: {}", e);
                }
            }
        }
    }

    if decks.is_empty() {
        eprintln!("Warning: No decks found matching patterns: {:?}", deck_globs);
    }

    // Serialize decks to bincode
    let decks_path = output.join("decks.bin");
    let decks_data = bincode::serialize(&decks)
        .map_err(|e| mtg_forge_rs::MtgError::InvalidDeckFormat(format!("Failed to serialize decks: {}", e)))?;
    fs::write(&decks_path, &decks_data).map_err(mtg_forge_rs::MtgError::IoError)?;
    println!(
        "\nExported {} decks to {} ({} bytes)",
        decks.len(),
        decks_path.display(),
        decks_data.len()
    );

    // Generate per-deck card packs (optimization for fast loading)
    // Each deck gets a mini cards.bin containing only the cards it needs
    let deck_cards_dir = output.join("deck_cards");

    // Clean up old deck_cards directory to remove stale files
    if deck_cards_dir.exists() {
        fs::remove_dir_all(&deck_cards_dir).map_err(|e| {
            mtg_forge_rs::MtgError::IoError(std::io::Error::other(format!(
                "Failed to clean deck_cards directory: {}",
                e
            )))
        })?;
    }
    fs::create_dir_all(&deck_cards_dir).map_err(|e| {
        mtg_forge_rs::MtgError::IoError(std::io::Error::other(format!(
            "Failed to create deck_cards directory: {}",
            e
        )))
    })?;

    println!("\nGenerating per-deck card packs...");
    let mut deck_card_sizes: HashMap<String, usize> = HashMap::new();

    for (deck_name, deck) in &decks {
        let unique_names = deck.unique_card_names();
        let mut deck_cards: HashMap<String, mtg_forge_rs::loader::CardDefinition> = HashMap::new();

        for card_name in &unique_names {
            if let Some(card_def) = card_definitions.get(card_name) {
                deck_cards.insert(card_name.clone(), card_def.clone());
            } else {
                eprintln!("  Warning: Card '{}' not found for deck '{}'", card_name, deck_name);
            }
        }

        // Serialize this deck's cards
        let deck_cards_path = deck_cards_dir.join(format!("{}.bin", deck_name));
        let deck_cards_data = bincode::serialize(&deck_cards)
            .map_err(|e| mtg_forge_rs::MtgError::InvalidCardFormat(format!("Failed to serialize deck cards: {}", e)))?;

        fs::write(&deck_cards_path, &deck_cards_data).map_err(mtg_forge_rs::MtgError::IoError)?;
        deck_card_sizes.insert(deck_name.clone(), deck_cards_data.len());

        println!(
            "  {} - {} unique cards ({} bytes)",
            deck_name,
            deck_cards.len(),
            deck_cards_data.len()
        );
    }

    // Generate deck index (names, sizes, and card pack sizes for UI)
    #[derive(serde::Serialize)]
    struct DeckIndexEntry {
        name: String,
        card_count: usize,
        unique_cards: usize,
        card_pack_bytes: usize,
    }

    let deck_index: Vec<DeckIndexEntry> = decks
        .iter()
        .map(|(name, deck)| DeckIndexEntry {
            name: name.clone(),
            card_count: deck.total_cards(),
            unique_cards: deck.unique_card_names().len(),
            card_pack_bytes: deck_card_sizes.get(name).copied().unwrap_or(0),
        })
        .collect();
    let index_path = output.join("deck_index.json");
    let index_json = serde_json::to_string_pretty(&deck_index)
        .map_err(|e| mtg_forge_rs::MtgError::InvalidDeckFormat(format!("Failed to serialize deck index: {}", e)))?;
    fs::write(&index_path, &index_json).map_err(mtg_forge_rs::MtgError::IoError)?;
    println!("\nExported deck index to {}", index_path.display());

    let total_deck_cards_size: usize = deck_card_sizes.values().sum();
    println!("\n=== Export Complete ===");
    println!("Files created in {}:", output.display());
    println!(
        "  cards.bin       - {} card definitions ({} bytes) [fallback]",
        card_definitions.len(),
        cards_data.len()
    );
    println!("  decks.bin       - {} decks ({} bytes)", decks.len(), decks_data.len());
    println!(
        "  deck_cards/*.bin - {} per-deck card packs ({} bytes total)",
        decks.len(),
        total_deck_cards_size
    );
    println!("  deck_index.json - deck metadata");

    Ok(())
}

/// Download card images from Scryfall
async fn run_download(
    output: PathBuf,
    cardsfolder: PathBuf,
    deck_files: Option<Vec<PathBuf>>,
    sizes_str: String,
    concurrency: usize,
    force: bool,
    rate_limit: u64,
) -> Result<()> {
    use mtg_forge_rs::download::{
        load_card_names_from_cardsfolder, load_card_names_from_deck, DownloadConfig, ImageDownloader, ImageSize,
    };

    println!("=== MTG Forge - Image Downloader ===\n");

    // Parse image sizes
    let sizes: Vec<ImageSize> = sizes_str
        .split(',')
        .filter_map(|s| ImageSize::parse(s.trim()))
        .collect();

    if sizes.is_empty() {
        return Err(mtg_forge_rs::MtgError::InvalidAction(
            "No valid image sizes specified. Use: small, normal".to_string(),
        ));
    }

    println!(
        "Image sizes: {}",
        sizes.iter().map(|s| s.api_version()).collect::<Vec<_>>().join(", ")
    );
    println!("Output directory: {}", output.display());
    println!("Concurrency: {}", concurrency);
    println!("Rate limit: {}ms between requests", rate_limit);
    println!("Skip existing: {}\n", !force);

    // Load card names
    let card_names = if let Some(decks) = deck_files {
        // Load from specified deck files
        let mut names = std::collections::HashSet::new();
        for deck_path in decks {
            println!("Loading cards from deck: {}", deck_path.display());
            match load_card_names_from_deck(&deck_path).await {
                Ok(deck_names) => {
                    println!("  Found {} unique cards", deck_names.len());
                    names.extend(deck_names);
                }
                Err(e) => {
                    eprintln!("  Warning: Failed to load deck: {}", e);
                }
            }
        }
        let mut result: Vec<String> = names.into_iter().collect();
        result.sort();
        result
    } else {
        // Load all cards from cardsfolder
        println!("Loading cards from cardsfolder: {}", cardsfolder.display());
        load_card_names_from_cardsfolder(&cardsfolder).await?
    };

    println!("\nFound {} unique card names", card_names.len());
    println!(
        "Total images to check: {} ({} sizes)",
        card_names.len() * sizes.len(),
        sizes.len()
    );
    println!();

    // Create downloader
    let config = DownloadConfig {
        output_dir: output,
        card_names,
        sizes,
        concurrency,
        skip_existing: !force,
        rate_limit_ms: rate_limit,
    };

    let downloader = ImageDownloader::new(config);

    // Run downloads
    let stats = downloader.download_all().await?;

    println!("\n=== Download Complete ===");
    println!("{}", stats);

    if stats.failed > 0 {
        println!(
            "\nNote: {} images failed to download. These may be cards not in Scryfall",
            stats.failed
        );
        println!("(e.g., custom tokens, test cards, or cards with non-standard names)");
    }

    Ok(())
}
