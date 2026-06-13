//! Benchmark: logging overhead — OFF vs string-only vs string+events
//!
//! Measures what fraction of game-simulation time is spent in the logging
//! subsystem by running N mirror games in three configurations:
//!
//!  1. Silent   (VerbosityLevel::Silent, event_log OFF) — baseline, ~no logging
//!  2. Memory   (VerbosityLevel::Normal, OutputMode::Memory, event_log OFF) — current path
//!  3. MemEvt   (VerbosityLevel::Normal, OutputMode::Memory, event_log ON)  — structured log
//!
//! Run (RELEASE — debug timings are meaningless):
//!   cargo run --release --example bench_logging_overhead --features network
//!
//! Optional env vars:
//!   BENCH_GAMES=<n>   number of games per configuration (default 300)
//!   BENCH_SEED=<n>    RNG seed base (default 42)
//!   BENCH_DECK=<path> deck file path (default decks/fuzz_bolt_mirror.dck)

use mtg_engine::{
    game::{logger::OutputMode, GameLoop, HeuristicController, VerbosityLevel},
    loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckList, DeckLoader, GameInitializer},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() {
    let n_games: u64 = std::env::var("BENCH_GAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let seed_base: u64 = std::env::var("BENCH_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(42);
    let deck_path = std::env::var("BENCH_DECK").unwrap_or_else(|_| "decks/fuzz_bolt_mirror.dck".to_string());

    println!("=== bench_logging_overhead ===");
    println!("Games per config : {n_games}");
    println!("Deck             : {deck_path}");
    println!("Seed base        : {seed_base}");
    println!();

    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        eprintln!("ERROR: cardsfolder not found — run from repo root");
        std::process::exit(1);
    }

    let deck_content = std::fs::read_to_string(&deck_path).unwrap_or_else(|e| panic!("Cannot read {deck_path}: {e}"));
    let deck = DeckLoader::parse(&deck_content).expect("Failed to parse deck");

    let card_db = CardDatabase::new(cardsfolder);
    print!("Prefetching deck cards... ");
    let (count, _) = prefetch_deck_cards(&card_db, &deck).await.expect("prefetch failed");
    println!("{count} cards loaded");
    println!();

    // ── Warmup ──────────────────────────────────────────────────────────────
    print!("Warmup (10 games, silent)... ");
    run_n_games(
        &card_db,
        &deck,
        10,
        seed_base,
        VerbosityLevel::Silent,
        OutputMode::Stdout,
        false,
    )
    .await;
    println!("done");
    println!();

    // ── 1. Baseline: Silent / no logging ────────────────────────────────────
    print!("(1) Silent  (logging OFF): running {n_games} games... ");
    let t_silent = time_n_games(
        &card_db,
        &deck,
        n_games,
        seed_base,
        VerbosityLevel::Silent,
        OutputMode::Stdout,
        false,
    )
    .await;
    println!("done  ({:.3}s total)", t_silent.as_secs_f64());

    // ── 2. Memory: Normal verbosity + string buffer (existing path) ──────────
    print!("(2) Memory  (string log ON, events OFF): running {n_games} games... ");
    let t_memory = time_n_games(
        &card_db,
        &deck,
        n_games,
        seed_base,
        VerbosityLevel::Normal,
        OutputMode::Memory,
        false,
    )
    .await;
    println!("done  ({:.3}s total)", t_memory.as_secs_f64());

    // ── 3. MemEvt: Normal verbosity + string buffer + event log ─────────────
    print!("(3) MemEvt  (string log ON, events ON): running {n_games} games... ");
    let t_memevt = time_n_games(
        &card_db,
        &deck,
        n_games,
        seed_base,
        VerbosityLevel::Normal,
        OutputMode::Memory,
        true,
    )
    .await;
    println!("done  ({:.3}s total)", t_memevt.as_secs_f64());

    // ── Results ──────────────────────────────────────────────────────────────
    let ms = |d: Duration| -> f64 { d.as_secs_f64() * 1000.0 };
    let per = |d: Duration| -> f64 { ms(d) / n_games as f64 };

    let overhead_string = ms(t_memory) - ms(t_silent);
    let overhead_events = ms(t_memevt) - ms(t_memory);
    let pct_string = overhead_string / ms(t_memory) * 100.0;
    let pct_events = if ms(t_memevt) > 0.0 {
        overhead_events / ms(t_memevt) * 100.0
    } else {
        0.0
    };

    println!();
    println!("=== Results ===");
    println!(
        "(1) Silent   (OFF):     {:.3}s total  {:.3}ms/game",
        t_silent.as_secs_f64(),
        per(t_silent)
    );
    println!(
        "(2) Memory   (str ON):  {:.3}s total  {:.3}ms/game",
        t_memory.as_secs_f64(),
        per(t_memory)
    );
    println!(
        "(3) MemEvt   (str+evt): {:.3}s total  {:.3}ms/game",
        t_memevt.as_secs_f64(),
        per(t_memevt)
    );
    println!();
    println!(
        "String-log overhead vs silent:         +{:.3}ms/game  ({:.1}% of str-ON path)",
        overhead_string / n_games as f64,
        pct_string
    );
    println!(
        "Event-log overhead vs string-only:     +{:.3}ms/game  ({:.1}% of str+evt path)",
        overhead_events / n_games as f64,
        pct_events
    );
    println!();
    println!(
        "= DISABLE PATH: Silent removes ~{:.1}% of total game-sim time.",
        pct_string.max(0.0)
    );
    println!(
        "= EVENTS ADD:   structured events add ~{:.3}ms/game on top of string logging.",
        overhead_events / n_games as f64
    );
}

async fn time_n_games(
    card_db: &CardDatabase,
    deck: &DeckList,
    n: u64,
    seed_base: u64,
    verbosity: VerbosityLevel,
    output_mode: OutputMode,
    enable_events: bool,
) -> Duration {
    let start = Instant::now();
    run_n_games(card_db, deck, n, seed_base, verbosity, output_mode, enable_events).await;
    start.elapsed()
}

async fn run_n_games(
    card_db: &CardDatabase,
    deck: &DeckList,
    n: u64,
    seed_base: u64,
    verbosity: VerbosityLevel,
    output_mode: OutputMode,
    enable_events: bool,
) {
    let initializer = GameInitializer::new(card_db);
    for i in 0..n {
        let seed = seed_base.wrapping_add(i);
        let mut game = initializer
            .init_game("Alice".to_string(), deck, "Bob".to_string(), deck, 20)
            .await
            .expect("init_game failed");
        game.seed_rng(seed);

        // Configure logging mode
        game.logger.set_verbosity(verbosity);
        game.logger.set_output_mode(output_mode);
        if enable_events {
            game.logger.enable_event_log();
        }

        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let mut c1 = HeuristicController::with_seed(p1_id, seed);
        let mut c2 = HeuristicController::with_seed(p2_id, seed.wrapping_add(1));

        let _result = GameLoop::new(&mut game)
            .with_verbosity(verbosity)
            .with_max_turns(50)
            .run_game(&mut c1, &mut c2)
            .expect("run_game failed");
    }
}
