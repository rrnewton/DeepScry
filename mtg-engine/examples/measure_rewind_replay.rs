//! Measure the ENGINE COMPUTE cost of the WASM network-blocking rewind+replay
//! mechanism (mtg-610 / mtg-614 / project_netarch_rewind_vision).
//!
//! ## What this measures (and what it does NOT change)
//!
//! The WASM / network client cannot save a blocked call-stack frame, so to
//! block waiting for the human's input mid-turn it UNWINDS to beginning-of-turn
//! via `undo_log.rewind_to_turn_start(&mut game)` (undoing every logged
//! `GameAction` back to the `ChangeTurn` boundary), then REPLAYS the turn
//! forward via `GameLoop::run_until_input` feeding the recorded `ReplayChoice`s
//! back through a `ReplayController`. Each network block within a turn pays one
//! full rewind+replay cycle, so the intra-turn cost is O(N^2): the K-th block in
//! a turn replays K-1 prior decisions.
//!
//! This harness quantifies that compute cost in fractional milliseconds. It is a
//! PURE MEASUREMENT: it calls the REAL, UNCHANGED `rewind_to_turn_start` +
//! `run_until_input` code (the exact production functions in `undo.rs` /
//! `game_loop`). NO instrumentation is added to shipping engine code; all timing
//! lives here, in the harness, via `std::time::Instant`.
//!
//! ## How a "network block" is simulated
//!
//! In production (`wasm/fancy_tui.rs`), a block point is any priority window
//! where the local human must decide. We model the networked player (P1) with a
//! controller that, on its very next priority decision, returns `NeedInput` —
//! exactly the signal the engine turns into `GameLoopState::AwaitingInput`. The
//! harness then performs the production rewind+replay cycle to get back to that
//! frontier, times it, verifies the state-hash round-trips EXACTLY (the netarch
//! correctness invariant: desync is always fatal), then lets P1 actually answer
//! that one decision and re-arms the block on the NEXT priority window. Each such
//! pause is one timed rewind+replay cycle, mirroring one network round-trip.
//!
//! Run (RELEASE — debug timings are meaningless):
//!   cargo run --release --example measure_rewind_replay --features network
//!
//! Optional env vars:
//!   MEASURE_TURNS=<n>   max turns to play (default 8)
//!   MEASURE_SEED=<n>    RNG seed (default 42)
//!   MEASURE_MIN_CYCLES=<n>  keep playing turns until at least this many timed
//!                           cycles are collected (default 30)

// TODO(mtg-211): wildcard match arms in ChoiceContext handling below.
#![allow(clippy::wildcard_enum_match_arm)]

use mtg_engine::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        compute_state_hash,
        controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, GameLoopState, HeuristicController, ReplayController, VerbosityLevel,
    },
    loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    undo::GameAction,
    Result,
};
use smallvec::SmallVec;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// P1 controller for the FORWARD pass: a real `HeuristicController`, but on its
/// next priority decision it returns `NeedInput` once (a simulated network
/// block), then continues normally. `armed` is set true to make the very next
/// `choose_spell_ability_to_play` block; the harness re-arms it after advancing
/// past each block.
struct BlockingController {
    inner: HeuristicController,
    /// Number of priority decisions to ANSWER (delegate to heuristic) before
    /// blocking. The harness sets this to 0 to block at the very next priority
    /// window, or to 1 to answer the just-blocked decision once and then block
    /// at the NEXT window (used to advance past a measured block).
    answers_before_block: u32,
}

impl BlockingController {
    fn new(player_id: PlayerId, seed: u64) -> Self {
        Self {
            inner: HeuristicController::with_seed(player_id, seed),
            answers_before_block: 0,
        }
    }
}

impl PlayerController for BlockingController {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if self.answers_before_block == 0 {
            // Simulate the network client blocking at this priority window.
            return ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: Vec::new(),
            });
        }
        self.answers_before_block -= 1;
        self.inner.choose_spell_ability_to_play(view, available)
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner
            .choose_targets(view, spell, valid_targets, min_targets, max_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        self.inner.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner.choose_damage_assignment_order(view, attacker, blockers)
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.inner.choose_cards_to_discard(view, hand, count)
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        self.inner.choose_from_library(view, valid_cards)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents)
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        self.inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.inner.on_priority_passed(view);
    }
    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.inner.on_game_end(view, won);
    }
    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

/// Inner controller used UNDER P1's `ReplayController` on the REPLAY pass. Once
/// the undo-logged choices are exhausted (we are back at the block frontier), it
/// returns `NeedInput` so the replay stops exactly where the forward pass
/// stopped — the production "stop at frontier" behaviour. We never answer here;
/// answering (to advance) is done by the forward `BlockingController`.
struct StopController {
    player_id: PlayerId,
    inner: HeuristicController,
}

impl StopController {
    fn new(player_id: PlayerId, seed: u64) -> Self {
        Self {
            player_id,
            inner: HeuristicController::with_seed(player_id, seed),
        }
    }
}

impl PlayerController for StopController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
            available: available.to_vec(),
            formatted_choices: Vec::new(),
        })
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner
            .choose_targets(view, spell, valid_targets, min_targets, max_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        self.inner.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner.choose_damage_assignment_order(view, attacker, blockers)
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.inner.choose_cards_to_discard(view, hand, count)
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        self.inner.choose_from_library(view, valid_cards)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents)
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        self.inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}
    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

/// Partition the intra-turn `ChoicePoint` actions returned by
/// `rewind_to_turn_start` into (our, opponent) `ReplayChoice` lists, mirroring
/// `WasmFancyTuiState::rewind_to_turn_start` (the production rewind/replay path).
fn partition_choices(choice_actions: Vec<GameAction>, our_id: PlayerId) -> (Vec<ReplayChoice>, Vec<ReplayChoice>) {
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for action in choice_actions {
        if let GameAction::ChoicePoint {
            player_id,
            choice: Some(c),
            ..
        } = action
        {
            if player_id == our_id {
                ours.push(c);
            } else {
                theirs.push(c);
            }
        }
    }
    (ours, theirs)
}

/// One timed rewind+replay measurement.
struct CycleMeasurement {
    turn: u32,
    block_index_in_turn: u32,
    rewind: Duration,
    replay: Duration,
    actions_replayed: usize,
    hash_exact: bool,
}

fn fms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[tokio::main]
async fn main() -> Result<()> {
    let max_turns: u32 = std::env::var("MEASURE_TURNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let seed: u64 = std::env::var("MEASURE_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(42);
    let min_cycles: usize = std::env::var("MEASURE_MIN_CYCLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    // ── Decks: realistic old-school 60-card decks with creatures + instants so
    //    combat + priority create multiple intra-turn block points per turn. ──
    let deck1_path = Path::new("decks/old_school/06_jeskai_aggro_joseantonioprieto.dck");
    let deck2_path = Path::new("decks/old_school/01_rogue_rogerbrand.dck");
    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        eprintln!("ERROR: cardsfolder/ not found — run from the repo root.");
        std::process::exit(1);
    }

    let deck1 = DeckLoader::load_from_file(deck1_path)?;
    let deck2 = DeckLoader::load_from_file(deck2_path)?;
    let card_db = CardDatabase::new(cardsfolder);
    prefetch_deck_cards(&card_db, &deck1).await?;
    prefetch_deck_cards(&card_db, &deck2).await?;

    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game("P1-net".to_string(), &deck1, "P2".to_string(), &deck2, 20)
        .await?;
    game.seed_rng(seed);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // P1 = simulated networked player (blocks at each priority window).
    // P2 = local heuristic opponent.
    let mut p1 = BlockingController::new(p1_id, seed);
    let mut p2 = HeuristicController::with_seed(p2_id, seed.wrapping_add(1));

    let mut measurements: Vec<CycleMeasurement> = Vec::new();
    // Total engine actions executed across the WHOLE game including every replay
    // re-execution, vs. the single-forward-pass baseline (actions added net of
    // replays). Used for the O(N^2) recompute factor.
    let mut total_actions_executed_incl_replays: u64 = 0;
    let mut block_index_in_turn: u32 = 0;
    let mut last_turn: u32 = 0;

    // Drive the game forward to the FIRST network block (P1 NeedInputs at its
    // next priority window). After that, each loop iteration measures a block
    // then advances exactly one priority decision to the next block, keeping the
    // live state moving forward through the game (just like the production WASM
    // client, which keeps live golden state and only re-runs the current turn).
    p1.answers_before_block = 0;
    {
        let undo_len_before = game.undo_log.len();
        let st = {
            let mut gl = GameLoop::new(&mut game)
                .with_verbosity(VerbosityLevel::Silent)
                .with_max_turns(max_turns);
            gl.run_until_input(&mut p1, &mut p2)?
        };
        total_actions_executed_incl_replays += game.undo_log.len().saturating_sub(undo_len_before) as u64;
        if let GameLoopState::Complete(_) = st {
            eprintln!("ERROR: game completed before any block point — pick livelier decks/seed.");
            std::process::exit(2);
        }
    }

    loop {
        let turn_at_block = game.turn.turn_number;
        if turn_at_block != last_turn {
            block_index_in_turn = 0;
            last_turn = turn_at_block;
        }
        block_index_in_turn += 1;

        // We are at a network block point. Capture the pre-block hash.
        let hash_before_block = compute_state_hash(&game);

        // ── TIMED rewind+replay cycle (the production network-block path) ──
        // Phase 1: rewind to turn start (undo all logged actions back to the
        // ChangeTurn boundary), extracting the recorded choices.
        let t_rewind = Instant::now();
        let mut undo_log = std::mem::take(&mut game.undo_log);
        let rewind = undo_log.rewind_to_turn_start(&mut game);
        game.undo_log = undo_log;
        let rewind_dur = t_rewind.elapsed();

        let (_turn, choice_actions, actions_rewound, log_size_at_turn) =
            rewind.expect("rewind_to_turn_start must succeed (undo log enabled)");
        // Match the production path: truncate game logs to the rewound state so
        // replay does not double-append log entries.
        game.logger.truncate_to(log_size_at_turn);
        let (p1_choices, p2_choices) = partition_choices(choice_actions, p1_id);
        let n_replay_choices = p1_choices.len() + p2_choices.len();

        // Phase 2: replay forward. P1 replays its recorded choices then
        // NeedInputs at the frontier (StopController); P2 replays its recorded
        // choices then also stops. This re-executes the whole turn up to the
        // block — the O(N^2) intra-turn recompute.
        let undo_len_at_turn_start = game.undo_log.len();
        let t_replay = Instant::now();
        {
            let mut p1_replay = ReplayController::new(p1_id, Box::new(StopController::new(p1_id, seed)), p1_choices);
            let mut p2_replay = ReplayController::new(
                p2_id,
                Box::new(StopController::new(p2_id, seed.wrapping_add(1))),
                p2_choices,
            );
            let mut gl = GameLoop::new(&mut game)
                .with_verbosity(VerbosityLevel::Silent)
                .with_max_turns(max_turns);
            let _ = gl.run_until_input(&mut p1_replay, &mut p2_replay)?;
        }
        let replay_dur = t_replay.elapsed();
        let actions_replayed = game.undo_log.len().saturating_sub(undo_len_at_turn_start);
        total_actions_executed_incl_replays += actions_replayed as u64;

        // ── CORRECTNESS: hash must round-trip EXACTLY (desync is fatal) ──
        let hash_after_replay = compute_state_hash(&game);
        let hash_exact = hash_after_replay == hash_before_block;
        if !hash_exact {
            eprintln!(
                "HASH DIVERGENCE at turn {turn_at_block} block {block_index_in_turn}: \
                 before=0x{hash_before_block:016x} after=0x{hash_after_replay:016x} \
                 (rewound {actions_rewound} actions, replayed {actions_replayed})"
            );
        }

        measurements.push(CycleMeasurement {
            turn: turn_at_block,
            block_index_in_turn,
            rewind: rewind_dur,
            replay: replay_dur,
            actions_replayed: actions_replayed.max(n_replay_choices),
            hash_exact,
        });

        // Stopping condition: enough cycles collected. The game naturally ends
        // (a player wins) well before max_turns for the chosen decks, but if it
        // does not we stop once we have enough samples.
        if measurements.len() >= min_cycles {
            break;
        }
        // Absolute safety bound: never spin forever. Once the live game reaches
        // the turn cap it can no longer advance turns, so P1 would block at every
        // priority window of that final turn indefinitely. Cap on that.
        if game.turn.turn_number >= max_turns {
            break;
        }

        // ── Advance to the NEXT block. The live state is at the frontier of the
        //    just-measured block. Answer that one decision (answers_before_block
        //    = 1) and block at the next priority window. ──
        p1.answers_before_block = 1;
        let undo_len_pre_advance = game.undo_log.len();
        let adv = {
            let mut gl = GameLoop::new(&mut game)
                .with_verbosity(VerbosityLevel::Silent)
                .with_max_turns(max_turns);
            gl.run_until_input(&mut p1, &mut p2)?
        };
        total_actions_executed_incl_replays += game.undo_log.len().saturating_sub(undo_len_pre_advance) as u64;
        if let GameLoopState::Complete(_) = adv {
            break;
        }
    }

    // ── Report ────────────────────────────────────────────────────────────────
    let n = measurements.len();
    if n == 0 {
        eprintln!("ERROR: collected 0 rewind/replay cycles — no block points were hit.");
        std::process::exit(2);
    }

    let mut totals: Vec<f64> = measurements.iter().map(|m| fms(m.rewind + m.replay)).collect();
    let mut rewinds: Vec<f64> = measurements.iter().map(|m| fms(m.rewind)).collect();
    let mut replays: Vec<f64> = measurements.iter().map(|m| fms(m.replay)).collect();
    totals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    rewinds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    replays.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let all_exact = measurements.iter().all(|m| m.hash_exact);
    let n_inexact = measurements.iter().filter(|m| !m.hash_exact).count();
    let total_ms_game: f64 = totals.iter().sum();
    let last_turn_seen = measurements.iter().map(|m| m.turn).max().unwrap_or(0);
    let max_blocks_in_a_turn = measurements.iter().map(|m| m.block_index_in_turn).max().unwrap_or(0);

    // Recompute factor: total actions executed (incl. replays) / a single
    // forward pass. The single-forward-pass baseline = total UNIQUE actions had
    // we never rewound = the actions that survive in the final undo log plus the
    // ones consumed by turn boundaries. We approximate the single-pass cost as
    // the sum of `actions_replayed` for the LAST block of each turn (the full
    // turn) — but more simply we report executed-incl-replays vs. the net.
    let final_log_len = game.undo_log.len() as u64;
    let recompute_factor = if final_log_len > 0 {
        total_actions_executed_incl_replays as f64 / final_log_len as f64
    } else {
        f64::NAN
    };

    println!("# Rewind+Replay Compute Cost Measurement");
    println!("seed={seed} max_turns={max_turns} turns_reached={last_turn_seen}");
    println!("cycles={n} max_blocks_in_a_turn={max_blocks_in_a_turn}");
    println!();
    println!("## Per-cycle TOTAL (rewind+replay) fractional ms");
    println!("  min    = {:.4}", totals[0]);
    println!("  median = {:.4}", percentile(&totals, 0.50));
    println!("  p90    = {:.4}", percentile(&totals, 0.90));
    println!("  max    = {:.4}", totals[n - 1]);
    println!();
    println!("## Rewind-only fractional ms (min / median / p90 / max)");
    println!(
        "  {:.4} / {:.4} / {:.4} / {:.4}",
        rewinds[0],
        percentile(&rewinds, 0.50),
        percentile(&rewinds, 0.90),
        rewinds[n - 1]
    );
    println!("## Replay-only fractional ms (min / median / p90 / max)");
    println!(
        "  {:.4} / {:.4} / {:.4} / {:.4}",
        replays[0],
        percentile(&replays, 0.50),
        percentile(&replays, 0.90),
        replays[n - 1]
    );
    println!();
    println!("## Aggregate");
    println!("  total ms in rewind+replay this game = {total_ms_game:.4}");
    println!("  cycles/turn (max blocks in one turn) = {max_blocks_in_a_turn}");
    println!("  total engine actions executed (incl replays) = {total_actions_executed_incl_replays}");
    println!("  final undo-log length (≈ single-pass actions)= {final_log_len}");
    println!("  recompute factor (executed ÷ single-pass)    = {recompute_factor:.2}x");
    println!();
    println!("## Correctness (hash round-trip)");
    if all_exact {
        println!("  ALL {n} cycles round-tripped EXACTLY (hash before == hash after).");
    } else {
        println!("  {n_inexact} / {n} cycles had a HASH DIVERGENCE (undo log incomplete for that block class).");
    }
    println!();

    // ── CSV to stdout (parsed by the capture script) ────────────────────────────
    println!("CSV_BEGIN");
    println!("turn,block_index_in_turn,rewind_ms,replay_ms,total_ms,actions_replayed,hash_exact");
    for m in &measurements {
        println!(
            "{},{},{:.6},{:.6},{:.6},{},{}",
            m.turn,
            m.block_index_in_turn,
            fms(m.rewind),
            fms(m.replay),
            fms(m.rewind + m.replay),
            m.actions_replayed,
            m.hash_exact
        );
    }
    println!("CSV_END");

    Ok(())
}
