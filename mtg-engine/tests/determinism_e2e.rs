//! End-to-end determinism tests
//!
//! Verifies that games with the same seed produce identical output across multiple runs.
//! This test runs the actual binary and compares stdout to ensure deterministic behavior.
//!
//! Tests are automatically generated for each `.dck` file in the `decks/` directory
//! using the `dir-test` procedural macro. No manual test registration needed!

use dir_test::{dir_test, Fixture};
use similar_asserts::assert_eq;
use std::path::PathBuf;
use std::process::Command;

/// Helper to run the mtg binary and capture stdout
fn run_game_with_seed(deck_path: &str, seed: u64, verbosity: &str) -> String {
    let output = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "--bin",
            "mtg",
            "--",
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
        .expect("Failed to run mtg binary");

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
    dir: "$CARGO_MANIFEST_DIR/decks",
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
    let deck_path = "decks/simple_bolt.dck";
    if !PathBuf::from(deck_path).exists() {
        return;
    }

    let verbosity = "verbose";

    // Verify seed 42 is consistent
    let seed42_run1 = run_game_with_seed(deck_path, 42, verbosity);
    let seed42_run2 = run_game_with_seed(deck_path, 42, verbosity);
    assert_eq!(seed42_run1, seed42_run2, "Seed 42 produced inconsistent output");

    // Verify seed 100 is consistent
    let seed100_run1 = run_game_with_seed(deck_path, 100, verbosity);
    let seed100_run2 = run_game_with_seed(deck_path, 100, verbosity);
    assert_eq!(seed100_run1, seed100_run2, "Seed 100 produced inconsistent output");

    // Verify different seeds produce different output
    assert_ne!(
        seed42_run1, seed100_run1,
        "Different seeds produced identical output (highly unlikely)"
    );
}
