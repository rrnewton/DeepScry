//! End-to-end determinism tests
//!
//! Verifies that games with the same seed produce identical output across multiple runs.
//! This test runs the actual binary and compares stdout to ensure deterministic behavior.
//!
//! Tests are automatically generated for each `.dck` file in the `decks/` directory
//! using the `dir-test` procedural macro. No manual test registration needed!
//!
//! ## Binary reuse (mtg-578 convention)
//!
//! Every per-deck test runs a full game TWICE, so the ~65 generated tests invoke
//! the `mtg` binary ~130 times. Spawning `cargo run` per game (the old design) had
//! three costs: (1) all the parallel nextest tests contended on cargo's `target/`
//! build lock, (2) it produced a slow DEBUG build, and (3) it ignored the
//! `target/release/mtg` that CI / `make validate` already build.
//!
//! These tests now invoke a PREBUILT RELEASE binary directly, mirroring the
//! shell-script convention in `tests/lib/test_helpers.sh` (`ensure_mtg_binary` +
//! the `MTG_REUSE_PREBUILT=1` flag). Resolution order for the binary path:
//!
//!   1. `$MTG_BIN` if set (explicit override, same env var the shell tests read).
//!   2. `$CARGO_MANIFEST_DIR/../target/release/mtg` (the canonical release path).
//!
//! If the resolved binary does not yet exist, it is built ONCE (guarded by a
//! `Once`) via `cargo build --release --bin mtg --features network`, so a bare
//! `cargo test` / `cargo nextest run` still works without any wrapper. CI and
//! `make validate` build it up front and set `MTG_REUSE_PREBUILT=1` to skip even
//! the existence-triggered build.

use dir_test::{dir_test, Fixture};
use similar_asserts::assert_eq;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

/// Build the release binary at most once for the whole test binary.
static BUILD_ONCE: Once = Once::new();

/// Default release binary path: `$CARGO_MANIFEST_DIR/../target/release/mtg`.
///
/// `CARGO_MANIFEST_DIR` points at `mtg-engine/`, so `../target/release/mtg` is the
/// workspace release binary.
fn default_mtg_bin() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/release/mtg"))
}

/// Resolve the prebuilt `mtg` release binary, building it once if necessary.
///
/// Mirrors `tests/lib/test_helpers.sh::ensure_mtg_binary`:
/// - Honour `$MTG_BIN` as an explicit override.
/// - When `MTG_REUSE_PREBUILT=1` is set (CI / `make validate`) the binary is
///   assumed to already exist and is used as-is.
/// - Otherwise, build it once if it is missing so a bare `cargo test` still works.
fn mtg_binary() -> PathBuf {
    let bin = std::env::var_os("MTG_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(default_mtg_bin);

    let reuse_prebuilt = std::env::var("MTG_REUSE_PREBUILT").as_deref() == Ok("1");
    if reuse_prebuilt {
        assert!(
            bin.exists(),
            "MTG_REUSE_PREBUILT=1 but prebuilt binary not found at {}. \
             CI / make validate must build it first.",
            bin.display()
        );
        return bin;
    }

    if !bin.exists() {
        // Build exactly once across all parallel tests in this binary. This is the
        // fallback path for a bare `cargo test` / `cargo nextest run` with no
        // pre-built binary; the single build amortizes across all ~130 invocations.
        BUILD_ONCE.call_once(|| {
            build_release_binary();
        });
        assert!(
            bin.exists(),
            "Failed to build release binary at {} (cargo build --release --bin mtg --features network)",
            bin.display()
        );
    }
    bin
}

/// Build the release `mtg` binary with the `network` feature (matches the rest of
/// `make validate`). Invoked at most once via `BUILD_ONCE`.
fn build_release_binary() {
    let status = Command::new("cargo")
        .args(["build", "--release", "--bin", "mtg", "--features", "network"])
        .status()
        .expect("Failed to invoke cargo build for mtg release binary");
    assert!(
        status.success(),
        "cargo build --release --bin mtg --features network failed"
    );
}

/// Helper to run the prebuilt mtg binary and capture stdout.
fn run_game_with_seed(deck_path: &str, seed: u64, verbosity: &str) -> String {
    run_game_with_seed_bin(&mtg_binary(), deck_path, seed, verbosity)
}

/// Run a specific binary; factored out so the multi-seed test can resolve the
/// binary once and reuse it across its four invocations.
fn run_game_with_seed_bin(bin: &Path, deck_path: &str, seed: u64, verbosity: &str) -> String {
    let output = Command::new(bin)
        .args([
            "tui",
            deck_path,
            deck_path,
            "--seed",
            &seed.to_string(),
            "--p1=random",
            "--p2=random",
            &format!("--verbosity={verbosity}"),
        ])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run mtg binary {}: {e}", bin.display()));

    String::from_utf8(output.stdout).expect("Invalid UTF-8 in stdout")
}

// ============================================================================
// Automatic deck determinism tests
// ============================================================================
// The dir_test macro automatically generates one test per .dck file
// No manual test registration needed - just add a .dck file to decks/!

/// Test determinism for all deck files in decks/
/// Automatically generates a separate test for each .dck file found
#[dir_test(
    dir: "$CARGO_MANIFEST_DIR/../decks",
    glob: "**/*.dck",
)]
fn test_deck_determinism(fixture: Fixture<&str>) {
    let deck_path = fixture.path();
    let seed = 42u64;
    let verbosity = "verbose";

    // Run the game twice with the same seed
    let run1 = run_game_with_seed(deck_path, seed, verbosity);
    let run2 = run_game_with_seed(deck_path, seed, verbosity);

    // Verify output is not empty
    assert!(!run1.is_empty(), "Deck {deck_path} produced empty output");

    // Verify both runs produce identical output
    assert_eq!(
        run1, run2,
        "Deck {} produced different output with same seed (seed={})",
        deck_path, seed
    );
}

// ============================================================================
// Multi-seed and cross-validation tests
// ============================================================================

/// Test that different seeds produce consistent but different results
#[test]
fn test_different_seeds_consistency() {
    // Path relative to workspace root
    let deck_path_rel = concat!(env!("CARGO_MANIFEST_DIR"), "/../decks/simple_bolt.dck");
    let deck_path_buf = PathBuf::from(deck_path_rel);
    if !deck_path_buf.exists() {
        return;
    }

    // Canonicalize to absolute path.
    let deck_path = deck_path_buf.canonicalize().expect("Failed to canonicalize deck path");
    let deck_path_str = deck_path.to_str().expect("Invalid UTF-8 in path");

    let verbosity = "verbose";
    let bin = mtg_binary();

    // Verify seed 42 is consistent
    let seed42_run1 = run_game_with_seed_bin(&bin, deck_path_str, 42, verbosity);
    let seed42_run2 = run_game_with_seed_bin(&bin, deck_path_str, 42, verbosity);
    assert_eq!(seed42_run1, seed42_run2, "Seed 42 produced inconsistent output");

    // Verify seed 100 is consistent
    let seed100_run1 = run_game_with_seed_bin(&bin, deck_path_str, 100, verbosity);
    let seed100_run2 = run_game_with_seed_bin(&bin, deck_path_str, 100, verbosity);
    assert_eq!(seed100_run1, seed100_run2, "Seed 100 produced inconsistent output");

    // Verify different seeds produce different output
    assert_ne!(
        seed42_run1, seed100_run1,
        "Different seeds produced identical output (highly unlikely)"
    );
}
