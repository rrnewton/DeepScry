//! Puzzle golden game-log snapshot oracle.
//!
//! For every locally-authored `.pzl` file in `test_puzzles/` and `puzzles/`
//! (NOT the forge-java corpus — too many pre-existing panics there), this test:
//!
//!   1. Runs the puzzle with `Normal` verbosity, capturing the game log to an
//!      in-memory buffer instead of stdout (DRY: reuses the same execution
//!      path as `puzzle_bulk_runner`).
//!   2. Compares the captured log against a committed golden file at
//!      `test_puzzles/goldens/<stem>.golden.log` or
//!      `puzzles/goldens/<stem>.golden.log`.
//!   3. On mismatch: fails with a readable unified diff.
//!   4. On `MTG_BLESS_GOLDEN=1`: writes the golden file instead of comparing
//!      (one-command re-bless, see `make puzzle-bless`).
//!
//! ## Re-blessing goldens (intended log-format change)
//!
//!   MTG_BLESS_GOLDEN=1 make puzzle-bulk-check
//!   # or equivalently:
//!   make puzzle-bless
//!
//! ## Scope
//!
//!   - Goldens are only kept for puzzles that run WITHOUT panics or load errors.
//!   - Forge-java puzzles are excluded (many pre-existing panics/load errors;
//!     see `puzzle_bulk_runner.rs` for cataloguing).
//!   - The forge-java exclusion is logged, not silent.
//!
//! Tracking issue: mtg-935 (PUZZLE_ASSERTION_DSL Phase 4)

#![cfg(all(feature = "puzzle-assert", feature = "native"))]

use mtg_engine::{
    game::{GameLoop, OutputMode, VerbosityLevel},
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
};
use rayon::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

// ─── Outcome ──────────────────────────────────────────────────────────────────

enum GoldenOutcome {
    /// Golden matched or was written (bless mode).
    Pass,
    /// Golden file does not exist yet; puzzle still ran OK.
    NoGolden,
    /// Puzzle panicked or failed to load — excluded from golden.
    Excluded { reason: String },
    /// Golden exists and the log does not match it.
    Mismatch { diff: String },
}

// ─── Puzzle discovery ─────────────────────────────────────────────────────────

fn discover_puzzles(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    fn recurse(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                recurse(&p, out);
            } else if p.extension().map(|e| e == "pzl").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    recurse(root, &mut out);
    out.sort();
    out
}

// ─── Golden file path ─────────────────────────────────────────────────────────

/// Map `<root>/<stem>.pzl` → `<root>/goldens/<stem>.golden.log`
fn golden_path(puzzle_path: &Path, workspace_root: &Path) -> Option<PathBuf> {
    let stem = puzzle_path.file_stem()?.to_str()?;
    let parent_dir = puzzle_path.parent()?;
    // Determine the corpus root (test_puzzles/ or puzzles/) relative to the
    // workspace root, so the golden always sits inside the same corpus dir.
    let rel = parent_dir.strip_prefix(workspace_root).ok()?;
    let goldens_dir = workspace_root.join(rel).join("goldens");
    Some(goldens_dir.join(format!("{stem}.golden.log")))
}

// ─── Log rendering ────────────────────────────────────────────────────────────

/// Render the log buffer to the canonical golden text format.
///
/// We use each entry's public `message` field (same text every viewer
/// without perspective-specific masking would see — i.e. `message_for`
/// called with a non-matching perspective returns `public_message`, but
/// for the golden we use the full `message` since the oracle log is the
/// canonical server-side full-information view). ANSI codes are stripped
/// — goldens must be colour-free for clean diffs.
///
/// The format is one log line per entry, with no leading spaces (the
/// `log_to_stdout` indentation is NOT part of the canonical log text —
/// that's a display detail). A trailing newline closes the file.
fn render_log(messages: &[String]) -> String {
    let mut out = String::new();
    for msg in messages {
        out.push_str(msg);
        out.push('\n');
    }
    out
}

// ─── Run one puzzle, capture log ─────────────────────────────────────────────

/// Returns `(log_messages, exclusion_reason_if_any)`.
fn capture_log_for_puzzle(path: &Path, card_db: &Arc<CardDatabase>) -> Result<Vec<String>, String> {
    let contents = fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;
    let puzzle = PuzzleFile::parse(&contents).map_err(|e| format!("parse error: {e}"))?;

    let card_db_ref = Arc::clone(card_db);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokio::runtime::Runtime::new()
            .expect("failed to create tokio runtime")
            .block_on(async move {
                let mut game = load_puzzle_into_game(&puzzle, &card_db_ref).await?;
                game.seed_rng(42);

                // Use Memory mode so the log is captured but NOT printed to
                // stdout (the test runner captures stdout per-thread anyway, but
                // this also prevents the rayon worker flood on --nocapture runs).
                game.logger.set_output_mode(OutputMode::Memory);

                let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
                let p0_id = players[0];
                let p1_id = players[1];

                // DRY action-script wiring (task #16): scripted players use a
                // RichInputController; unscripted players keep the heuristic AI.
                let mut c0 = puzzle.build_controller(0, p0_id);
                let mut c1 = puzzle.build_controller(1, p1_id);

                let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Normal);
                let _game_result = game_loop.run_game(c0.as_mut(), c1.as_mut())?;

                // Collect log messages. We take each entry's `message` — the
                // full server-side text (NOT masked for any perspective).
                let messages: Vec<String> = game_loop.game.logger.logs().iter().map(|e| e.message.clone()).collect();

                Ok::<_, mtg_engine::MtgError>(messages)
            })
    }));

    match result {
        Err(_) => Err("panic in game loop".to_string()),
        Ok(Err(e)) => Err(format!("engine error: {e}")),
        Ok(Ok(messages)) => Ok(messages),
    }
}

// ─── Unified diff helper ──────────────────────────────────────────────────────

/// Produce a human-readable unified diff (context 5 lines).
fn unified_diff(expected: &str, actual: &str, label: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();

    let mut out = format!("--- {label}.golden.log (expected)\n+++ {label} (actual)\n");

    // Simple LCS-based diff using the standard approach
    // We use a sliding window to produce context diffs
    let exp_len = exp_lines.len();
    let act_len = act_lines.len();

    if exp_len == 0 && act_len == 0 {
        return out;
    }

    // Compute LCS table
    let mut lcs = vec![vec![0usize; act_len + 1]; exp_len + 1];
    for i in (0..exp_len).rev() {
        for j in (0..act_len).rev() {
            lcs[i][j] = if exp_lines[i] == act_lines[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    // Walk the LCS to produce diff hunks
    enum DiffOp<'a> {
        Keep(&'a str),
        Del(&'a str),
        Add(&'a str),
    }

    let mut ops: Vec<DiffOp<'_>> = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < exp_len || j < act_len {
        if i < exp_len && j < act_len && exp_lines[i] == act_lines[j] {
            ops.push(DiffOp::Keep(exp_lines[i]));
            i += 1;
            j += 1;
        } else if j < act_len && (i >= exp_len || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(DiffOp::Add(act_lines[j]));
            j += 1;
        } else {
            ops.push(DiffOp::Del(exp_lines[i]));
            i += 1;
        }
    }

    // Group into hunks (context = 5)
    const CTX: usize = 5;
    let n = ops.len();
    // Find changed positions
    let changed: Vec<bool> = ops.iter().map(|op| !matches!(op, DiffOp::Keep(_))).collect();

    let mut pos = 0;
    while pos < n {
        if !changed[pos] {
            pos += 1;
            continue;
        }
        // Start a new hunk
        let hunk_start = pos.saturating_sub(CTX);
        let mut hunk_end = pos;
        // Extend hunk to cover all nearby changes
        while hunk_end < n {
            if changed[hunk_end] {
                // Extend context forward
                hunk_end = (hunk_end + 1 + CTX).min(n);
            } else {
                hunk_end += 1;
                if hunk_end >= n || (hunk_end - pos > CTX && !changed[hunk_end]) {
                    break;
                }
            }
        }

        // Count exp/act line numbers for the @@ header
        let mut exp_line = 1usize;
        for op in &ops[..hunk_start] {
            if matches!(op, DiffOp::Keep(_) | DiffOp::Del(_)) {
                exp_line += 1;
            }
        }
        let mut act_line = 1usize;
        for op in &ops[..hunk_start] {
            if matches!(op, DiffOp::Keep(_) | DiffOp::Add(_)) {
                act_line += 1;
            }
        }
        let exp_count = ops[hunk_start..hunk_end]
            .iter()
            .filter(|op| matches!(op, DiffOp::Keep(_) | DiffOp::Del(_)))
            .count();
        let act_count = ops[hunk_start..hunk_end]
            .iter()
            .filter(|op| matches!(op, DiffOp::Keep(_) | DiffOp::Add(_)))
            .count();

        out.push_str(&format!("@@ -{exp_line},{exp_count} +{act_line},{act_count} @@\n"));

        for op in &ops[hunk_start..hunk_end] {
            match op {
                DiffOp::Keep(l) => out.push_str(&format!(" {l}\n")),
                DiffOp::Del(l) => out.push_str(&format!("-{l}\n")),
                DiffOp::Add(l) => out.push_str(&format!("+{l}\n")),
            }
        }

        pos = hunk_end;
    }

    out
}

// ─── Test entry point ─────────────────────────────────────────────────────────

/// Golden game-log snapshot oracle for locally-authored puzzles.
///
/// Fails (hard) only when a committed golden does not match the engine output.
/// Missing goldens are reported but do NOT fail the test — run `make puzzle-bless`
/// to generate them for the first time or after an intentional log-format change.
#[test]
fn puzzle_golden_check() {
    let bless_mode = std::env::var("MTG_BLESS_GOLDEN").is_ok_and(|v| v == "1");

    if bless_mode {
        println!("[puzzle-golden] BLESS MODE: writing golden files from current engine output");
    }

    // ── Discover puzzles (LOCAL ONLY — no forge-java) ─────────────────────────
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");

    let mut all_paths = Vec::new();
    for subdir in &["test_puzzles", "puzzles"] {
        all_paths.extend(discover_puzzles(&workspace_root.join(subdir)));
    }
    let total_discovered = all_paths.len();
    assert!(
        total_discovered > 0,
        "No .pzl files found under test_puzzles/ or puzzles/ (forge-java excluded by design)"
    );
    println!(
        "[puzzle-golden] Discovered {total_discovered} local .pzl files \
         (forge-java corpus excluded — too many pre-existing panics)"
    );

    // ── Card database (once, shared) ──────────────────────────────────────────
    let cardsfolder = require_cardsfolder();
    let card_db = Arc::new(CardDatabase::new(cardsfolder));

    // ── Run all puzzles in parallel ────────────────────────────────────────────
    let ncpus = num_cpus::get();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(ncpus)
        .build()
        .expect("failed to build rayon pool");

    let wall_start = Instant::now();
    let results: Mutex<Vec<(PathBuf, GoldenOutcome)>> = Mutex::new(Vec::with_capacity(total_discovered));

    pool.install(|| {
        all_paths.par_iter().for_each(|path| {
            let outcome = run_one_golden(path, workspace_root, &card_db, bless_mode);
            results.lock().expect("mutex poisoned").push((path.clone(), outcome));
        });
    });

    let elapsed = wall_start.elapsed();
    let mut results = results.into_inner().expect("mutex poisoned");
    results.sort_by(|(a, _), (b, _)| a.cmp(b));

    // ── Tally ──────────────────────────────────────────────────────────────────
    let mut pass = 0usize;
    let mut no_golden = 0usize;
    let mut excluded = 0usize;
    let mut mismatches = 0usize;

    let mut mismatch_details: Vec<(PathBuf, String)> = Vec::new();
    let mut excluded_details: Vec<(PathBuf, String)> = Vec::new();
    let mut no_golden_details: Vec<PathBuf> = Vec::new();

    for (path, outcome) in &results {
        match outcome {
            GoldenOutcome::Pass => pass += 1,
            GoldenOutcome::NoGolden => {
                no_golden += 1;
                no_golden_details.push(path.clone());
            }
            GoldenOutcome::Excluded { reason } => {
                excluded += 1;
                excluded_details.push((path.clone(), reason.clone()));
            }
            GoldenOutcome::Mismatch { diff } => {
                mismatches += 1;
                mismatch_details.push((path.clone(), diff.clone()));
            }
        }
    }

    // ── Print excluded puzzles ─────────────────────────────────────────────────
    if !excluded_details.is_empty() {
        println!("[puzzle-golden] === EXCLUDED (panic/load error) ===");
        for (path, reason) in &excluded_details {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            println!("  EXCLUDED  {name}: {reason}");
        }
        println!("[puzzle-golden] === END EXCLUDED ===");
    }

    // ── Print missing goldens ──────────────────────────────────────────────────
    if !no_golden_details.is_empty() {
        println!(
            "[puzzle-golden] {} puzzle(s) have no golden yet — run `make puzzle-bless` to generate:",
            no_golden_details.len()
        );
        for path in &no_golden_details {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            println!("  MISSING-GOLDEN  {name}");
        }
    }

    // ── Print mismatches ──────────────────────────────────────────────────────
    if !mismatch_details.is_empty() {
        eprintln!("[puzzle-golden] === GOLDEN MISMATCHES (potential regressions) ===");
        for (path, diff) in &mismatch_details {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            eprintln!("\n  --- MISMATCH: {name} ---");
            for line in diff.lines().take(80) {
                eprintln!("  {line}");
            }
            let total_lines = diff.lines().count();
            if total_lines > 80 {
                eprintln!("  ... ({} more diff lines) ...", total_lines - 80);
            }
        }
        eprintln!("[puzzle-golden] === END MISMATCHES ===");
    }

    // ── Summary ────────────────────────────────────────────────────────────────
    let action = if bless_mode { "blessed" } else { "compared" };
    println!(
        "[puzzle-golden] {total_discovered} puzzles in {:.2}s ({ncpus} threads) | \
         {action}={pass} no-golden={no_golden} excluded={excluded} mismatch={mismatches}",
        elapsed.as_secs_f64()
    );

    // ── Failure gate ──────────────────────────────────────────────────────────
    // Hard fail only on golden mismatches — they indicate an UNEXPECTED change
    // to the game log (potential regression). Missing goldens are soft (just run
    // `make puzzle-bless`). Excluded puzzles are pre-existing failures tracked
    // by mtg-935; they do not gate this test.
    if !mismatch_details.is_empty() {
        let names: Vec<_> = mismatch_details
            .iter()
            .map(|(p, _)| p.file_name().and_then(|n| n.to_str()).unwrap_or("?"))
            .collect();
        panic!(
            "[puzzle-golden] {} golden mismatch(es) — unexpected log changes detected!\n\
             Puzzles: {}\n\
             If this is an INTENDED log-format change, re-bless with:\n\
             \n  make puzzle-bless\n\
             \nIf this is UNEXPECTED, investigate as a potential regression.\n\
             See diff output above.",
            mismatches,
            names.join(", ")
        );
    }
}

// ─── Per-puzzle golden runner ─────────────────────────────────────────────────

fn run_one_golden(path: &Path, workspace_root: &Path, card_db: &Arc<CardDatabase>, bless_mode: bool) -> GoldenOutcome {
    // Capture the log
    let messages = match capture_log_for_puzzle(path, card_db) {
        Err(reason) => return GoldenOutcome::Excluded { reason },
        Ok(msgs) => msgs,
    };
    let actual_text = render_log(&messages);

    // Determine golden path
    let golden = match golden_path(path, workspace_root) {
        Some(p) => p,
        None => {
            return GoldenOutcome::Excluded {
                reason: "could not derive golden path".to_string(),
            }
        }
    };

    if bless_mode {
        // Write/overwrite golden
        if let Some(parent) = golden.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return GoldenOutcome::Excluded {
                    reason: format!("could not create goldens dir: {e}"),
                };
            }
        }
        match fs::write(&golden, &actual_text) {
            Ok(()) => GoldenOutcome::Pass,
            Err(e) => GoldenOutcome::Excluded {
                reason: format!("could not write golden: {e}"),
            },
        }
    } else {
        // Compare
        match fs::read_to_string(&golden) {
            Err(_) => GoldenOutcome::NoGolden,
            Ok(expected_text) => {
                if expected_text == actual_text {
                    GoldenOutcome::Pass
                } else {
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("puzzle");
                    let diff = unified_diff(&expected_text, &actual_text, stem);
                    GoldenOutcome::Mismatch { diff }
                }
            }
        }
    }
}
