//! Microbenchmark: cost of `format_choice_menu` with vs without context.
//!
//! This is the **human-controller path** that's exercised when
//! `controller.wants_context() == true`. AI/Random/Heuristic controllers
//! never enter this path because (a) `should_show_choice_menu` is off in
//! batch mode, and (b) the new `wants_context` gate skips the context
//! `format!` even when the menu is shown.
//!
//! Run with:
//!   cargo run --release --example microbench_choice_context
//!
//! Reports nanoseconds per call. Per-game cost ≈ ns/call × choices/game.

use mtg_forge_rs::core::PlayerId;
use mtg_forge_rs::game::controller::{format_choice_menu, GameStateView};
use mtg_forge_rs::game::{GameState, Step};
use std::time::Instant;

fn main() {
    let mut game = GameState::new_two_player("P1".into(), "P2".into(), 20);
    game.turn.active_player = PlayerId::new(0);
    game.turn.current_step = Step::Main1;
    let view = GameStateView::new(&game, PlayerId::new(0));

    // Keep this small enough to be cheap in `make validate`, but large enough
    // to have stable per-call ns numbers above the noise floor.
    let n: u64 = 200_000;

    // Baseline: just produce the player_name part (no context built)
    let t0 = Instant::now();
    let mut sink = String::new();
    for _ in 0..n {
        let s = format_choice_menu(&view, &[], false);
        sink.push_str(&s[..1]);
    }
    let dt_no_ctx = t0.elapsed();

    // With context: build context string each call
    let t1 = Instant::now();
    let mut sink2 = String::new();
    for _ in 0..n {
        let s = format_choice_menu(&view, &[], true);
        sink2.push_str(&s[..1]);
    }
    let dt_ctx = t1.elapsed();

    println!("format_choice_menu, {} iters", n);
    println!(
        "  wants_context=false: {:?} ({:.1} ns/call)",
        dt_no_ctx,
        dt_no_ctx.as_nanos() as f64 / n as f64
    );
    println!(
        "  wants_context=true : {:?} ({:.1} ns/call)",
        dt_ctx,
        dt_ctx.as_nanos() as f64 / n as f64
    );
    // NOTE: We use checked_sub + signed-int arithmetic for the delta because
    // under heavy load (e.g. parallel `make validate`) the second loop
    // (wants_context=true) sometimes happens to finish faster than the first
    // baseline loop due to cache warm-up / scheduler noise. A naive
    // `dt_ctx - dt_no_ctx` would then panic on a Duration underflow and
    // surface as a flaky example failure. Reporting the signed delta keeps the
    // microbench resilient to that contention while still giving useful
    // numbers when the system is idle.
    let delta_ns = dt_ctx.as_nanos() as i128 - dt_no_ctx.as_nanos() as i128;
    let delta_dur = if dt_ctx >= dt_no_ctx {
        format!("{:?}", dt_ctx - dt_no_ctx)
    } else {
        format!("-{:?}", dt_no_ctx - dt_ctx)
    };
    println!(
        "  delta              : {} ({:.1} ns/call)",
        delta_dur,
        delta_ns as f64 / n as f64
    );
    println!("  sink lens (avoid optimization): {} {}", sink.len(), sink2.len());
}
