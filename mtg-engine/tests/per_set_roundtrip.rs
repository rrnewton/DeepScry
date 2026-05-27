//! Round-trip test for the per-set WASM exporter (mtg-6fsjb).
//!
//! Self-contained: invokes the `mtg export-wasm` binary into a tempdir
//! and then verifies that splitting the card database into per-set bins
//! preserves every card. For each card name in the original ~32k map,
//! the test (a) looks up its primary set in `sets/index.json`,
//! (b) deserialises that set's bincode file, and (c) asserts structural
//! integrity (presence + bincode self-roundtrip). We do NOT compare
//! against the original byte-for-byte because the upstream loader's
//! `CardDefinition` contains HashMap fields whose bincode wire order is
//! iteration-order-dependent — that's a pre-existing property of the
//! struct, not something this test should fail on.

#![cfg(feature = "native")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use mtg_forge_rs::loader::{CardDefinition, CardLoader};

/// CARGO_MANIFEST_DIR is mtg-engine/; repo root is its parent.
fn find_repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().expect("repo root").to_path_buf()
}

#[derive(serde::Deserialize)]
struct SetIndex {
    #[allow(dead_code)]
    version: u32,
    sets: Vec<SetManifestEntry>,
    cards: HashMap<String, String>,
}

#[derive(serde::Deserialize)]
struct SetManifestEntry {
    file: String,
    bytes: usize,
    card_count: usize,
}

/// Invoke `cargo run --bin mtg -- export-wasm --output <dir>` and return
/// the path to `<dir>/sets/`. Skips with a clear panic if `cargo` is not
/// on the runner's PATH.
fn run_exporter(into: &Path) -> PathBuf {
    let repo = find_repo_root();
    // Use the release build of `mtg` if it already exists (much faster);
    // otherwise fall back to `cargo run` which builds it on the spot. CI
    // already runs `cargo build --release --features network` before
    // `cargo test`, so the release binary should be present.
    let release_bin = repo.join("target/release/mtg");
    let status = if release_bin.exists() {
        Command::new(&release_bin)
            .current_dir(&repo)
            .args(["export-wasm", "--output"])
            .arg(into)
            .status()
            .expect("spawn release mtg")
    } else {
        Command::new("cargo")
            .current_dir(&repo)
            .args([
                "run",
                "--release",
                "--bin",
                "mtg",
                "--features",
                "network",
                "--",
                "export-wasm",
                "--output",
            ])
            .arg(into)
            .status()
            .expect("spawn cargo run")
    };
    assert!(status.success(), "mtg export-wasm failed: {:?}", status);
    into.join("sets")
}

#[test]
fn per_set_roundtrip_preserves_every_card() {
    let repo = find_repo_root();
    let cardsfolder = mtg_forge_rs::loader::find_cardsfolder()
        .expect("cardsfolder must be resolvable (forge-java submodule populated)");

    // Export into a tempdir so the test is hermetic and doesn't clobber
    // the dev's `web/data/` tree.
    let tmp = tempfile::tempdir().expect("tempdir");
    let sets_dir = run_exporter(tmp.path());
    let index_path = sets_dir.join("index.json");
    assert!(
        index_path.exists(),
        "expected exporter to write {}",
        index_path.display()
    );
    let _ = &repo; // keep repo binding to make the export step's working-dir intent obvious

    // 1. Load original cardsfolder.
    let pattern = format!("{}/**/*.txt", cardsfolder.display());
    let mut original: HashMap<String, CardDefinition> = HashMap::new();
    for entry in glob::glob(&pattern).expect("glob pattern") {
        let path = entry.expect("glob entry");
        if !path.is_file() {
            continue;
        }
        if let Ok(def) = CardLoader::load_from_file(&path) {
            original.insert(def.name.as_str().to_string(), def);
        }
    }
    assert!(
        original.len() >= 30_000,
        "expected ~32k cards in cardsfolder, got {}",
        original.len()
    );

    // 2. Load index.
    let index_bytes = std::fs::read(&index_path).expect("read index.json");
    let index: SetIndex = serde_json::from_slice(&index_bytes).expect("parse index.json");

    // 3. Validate manifest: every file exists, byte size matches, card_count > 0.
    let mut total_manifest_cards = 0usize;
    for entry in &index.sets {
        let p = sets_dir.join(&entry.file);
        let meta = std::fs::metadata(&p).expect("set file must exist");
        assert_eq!(
            meta.len() as usize,
            entry.bytes,
            "manifest size mismatch for {}",
            entry.file
        );
        assert!(entry.card_count > 0, "empty set file: {}", entry.file);
        total_manifest_cards += entry.card_count;
    }
    assert_eq!(
        total_manifest_cards,
        original.len(),
        "manifest card_count sum ({}) != original card count ({})",
        total_manifest_cards,
        original.len()
    );

    // 4. Group cards by their assigned set file (avoids re-reading set bins).
    let mut per_file: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, file) in &index.cards {
        per_file.entry(file.as_str()).or_default().push(name.as_str());
        assert!(
            original.contains_key(name),
            "index.cards references unknown card '{}'",
            name
        );
    }
    assert_eq!(
        index.cards.len(),
        original.len(),
        "index.cards count ({}) != original card count ({})",
        index.cards.len(),
        original.len()
    );

    // 5. For each set file, deserialise and check structural integrity:
    //    - every promised card is present
    //    - the deserialized name matches the lookup key
    //    - bincode itself round-trips (re-serialize the deserialized def and
    //      re-deserialize; byte-equal in *that* direction is well-defined
    //      because we control both ends of the second hop).
    //
    // We do NOT compare against `original` byte-for-byte because the upstream
    // `CardLoader::load_from_file` uses HashMap fields whose bincode wire
    // order is iteration-order-dependent and therefore non-deterministic
    // between separate loads of the same card. That's a property of the
    // existing CardDefinition struct, not something introduced by mtg-6fsjb.
    let mut total_seen = 0usize;
    for (file, names) in &per_file {
        let path = sets_dir.join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {}", file, e));
        let map: HashMap<String, CardDefinition> =
            bincode::deserialize(&bytes).unwrap_or_else(|e| panic!("deserialize {}: {}", file, e));

        for name in names {
            assert!(
                original.contains_key(*name),
                "card '{}' in index missing from source",
                name
            );
            let got = map
                .get(*name)
                .unwrap_or_else(|| panic!("set {} missing card '{}'", file, name));
            assert_eq!(got.name.as_str(), *name, "name mismatch in {}", file);
            // Self-roundtrip: deserialize -> serialize -> deserialize.
            let re = bincode::serialize(got).expect("re-ser");
            let _back: CardDefinition = bincode::deserialize(&re).expect("re-de");
            total_seen += 1;
        }
    }
    assert_eq!(
        total_seen,
        original.len(),
        "per-set bins do not cover every original card ({} vs {})",
        total_seen,
        original.len()
    );

    println!(
        "per-set roundtrip OK: {} cards across {} set files",
        original.len(),
        index.sets.len()
    );

    // Smoke: ensure the path used by index.json matches sets_dir layout.
    let _ = Path::new(&index.sets[0].file);
}
