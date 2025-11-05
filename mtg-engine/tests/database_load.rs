//! Database loading test
//!
//! This is the ONLY test that loads the full card database.
//! All other tests should load only the specific cards they need.

use mtg_forge_rs::{
    loader::{AsyncCardDatabase as CardDatabase, DeckLoader},
    Result,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Test deck loading for a given directory with a maximum allowed failure count
/// This serves as a regression test to ensure deck loading doesn't get worse
async fn test_deck_directory(
    card_db: &CardDatabase,
    directory: &Path,
    max_allowed_failures: usize,
    directory_name: &str,
) -> Result<()> {
    println!("\n=== Testing Deck Loading: {} ===", directory_name);

    if !directory.exists() {
        println!("Skipping {} deck loading test - directory not present", directory_name);
        return Ok(());
    }

    println!("Discovering .dck files in {}...", directory.display());

    // Discover all .dck files using jwalk (parallel directory walking)
    let deck_paths: Vec<PathBuf> = jwalk::WalkDir::new(directory)
        .skip_hidden(false)
        .into_iter()
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                if e.file_type().is_file() {
                    e.path()
                        .extension()
                        .and_then(|ext| if ext == "dck" { Some(e.path()) } else { None })
                } else {
                    None
                }
            })
        })
        .collect();

    let deck_count = deck_paths.len();
    println!("Found {} .dck files", deck_count);

    if deck_count == 0 {
        println!("No decks found in {}, skipping", directory_name);
        return Ok(());
    }

    // Load all decks in parallel with concurrency limit
    println!("Loading all decks and verifying card resolution...");
    let start = std::time::Instant::now();

    // Limit concurrency to avoid overwhelming the system
    let semaphore = Arc::new(Semaphore::new(100));
    let mut tasks = Vec::new();

    for deck_path in deck_paths {
        let sem = Arc::clone(&semaphore);
        let db = card_db.clone_handle();

        let task = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            // Load deck file
            let deck = DeckLoader::load_from_file(&deck_path)
                .map_err(|e| format!("Failed to parse deck {}: {}", deck_path.display(), e))?;

            // Verify all cards in deck can be resolved
            let all_cards = deck.unique_card_names();
            for card_name in &all_cards {
                match db.get_card(card_name).await {
                    Ok(Some(_)) => {} // Card found - success
                    Ok(None) => {
                        return Err(format!(
                            "Card '{}' in deck {} not found in database",
                            card_name,
                            deck_path.display()
                        ));
                    }
                    Err(e) => {
                        return Err(format!(
                            "Error loading card '{}' in deck {}: {}",
                            card_name,
                            deck_path.display(),
                            e
                        ));
                    }
                }
            }

            Ok::<_, String>((deck_path, deck.total_cards()))
        });

        tasks.push(task);
    }

    // Collect results
    let mut loaded_decks = 0;
    let mut failed_decks = Vec::new();

    for task in tasks {
        match task.await {
            Ok(Ok((_path, _cards))) => {
                loaded_decks += 1;
            }
            Ok(Err(e)) => {
                failed_decks.push(e);
            }
            Err(e) => {
                failed_decks.push(format!("Task join error: {}", e));
            }
        }
    }

    let duration = start.elapsed();

    println!(
        "Successfully loaded and verified {} decks in {:?}",
        loaded_decks, duration
    );

    if !failed_decks.is_empty() {
        println!("\n=== Failed Decks ({}) ===", failed_decks.len());
        for (i, error) in failed_decks.iter().take(10).enumerate() {
            println!("{}. {}", i + 1, error);
        }
        if failed_decks.len() > 10 {
            println!("... and {} more", failed_decks.len() - 10);
        }

        // Extract unique missing card names for analysis
        let missing_cards: std::collections::HashSet<String> = failed_decks
            .iter()
            .filter_map(|e| {
                if e.contains("not found in database") {
                    let start = e.find("Card '")? + 6;
                    let end = e[start..].find('\'')?;
                    Some(e[start..start + end].to_string())
                } else {
                    None
                }
            })
            .collect();

        println!("\n=== Missing Cards Analysis ===");
        println!("Unique cards not found: {}", missing_cards.len());
        println!("Sample missing cards:");
        for (i, card) in missing_cards.iter().take(20).enumerate() {
            println!("  {}. {}", i + 1, card);
        }

        println!(
            "\nSuccess rate: {}/{} decks ({:.1}%)",
            loaded_decks,
            deck_count,
            (loaded_decks as f64 / deck_count as f64) * 100.0
        );

        // Regression test: ensure we don't get worse than the current baseline
        assert!(
            failed_decks.len() <= max_allowed_failures,
            "Deck loading regressed in {}! Expected <= {} failures, got {}. This means card name \
             normalization or database loading got worse.",
            directory_name,
            max_allowed_failures,
            failed_decks.len()
        );

        println!(
            "\n✓ Regression test passed: {} failures <= {} allowed",
            failed_decks.len(),
            max_allowed_failures
        );
    } else {
        println!("\n✓ All {} decks loaded successfully!", deck_count);
    }

    Ok(())
}

/// Test that the full card database and all forge-java decks can be loaded
/// This is the ONLY test in the entire test suite that should call eager_load()
/// Also serves as a regression test to ensure deck loading doesn't get worse
#[tokio::test]
async fn test_load_cards_and_decks() -> Result<()> {
    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        // Skip test if cardsfolder doesn't exist
        println!("Skipping full database load test - cardsfolder not present");
        return Ok(());
    }

    println!("Loading full card database from cardsfolder...");
    let card_db = CardDatabase::new(cardsfolder);
    let (loaded, duration) = card_db.eager_load().await?;

    println!("Successfully loaded {} cards in {:?}", loaded, duration);

    // Verify we loaded a reasonable number of cards (should be 30,000+)
    assert!(
        loaded > 30000,
        "Expected to load full database (30,000+ cards), but only loaded {}",
        loaded
    );

    // Verify some known cards can be retrieved
    let mountain = card_db.get_card("Mountain").await?;
    assert!(mountain.is_some(), "Mountain should be in database");
    assert_eq!(mountain.unwrap().name.as_str(), "Mountain");

    let lightning_bolt = card_db.get_card("Lightning Bolt").await?;
    assert!(lightning_bolt.is_some(), "Lightning Bolt should be in database");
    assert_eq!(lightning_bolt.unwrap().name.as_str(), "Lightning Bolt");

    let grizzly_bears = card_db.get_card("Grizzly Bears").await?;
    assert!(grizzly_bears.is_some(), "Grizzly Bears should be in database");
    assert_eq!(grizzly_bears.unwrap().name.as_str(), "Grizzly Bears");

    // Test loading decks from forge-java
    // Known limitation: Double-faced cards (DFCs) and modal double-faced cards (MDFCs)
    // are stored in files with both face names combined, but decks reference only one face.
    // Example: 'Ludevic, Necrogenius' is in 'ludevic_necrogenius_olag_ludevics_hubris.txt'
    // This requires building a card name index during database load.
    // Current baseline: 1730 failures out of 6000+ decks (~71% success rate)
    let forge_java = PathBuf::from("forge-java");
    test_deck_directory(&card_db, &forge_java, 1730, "forge-java").await?;

    // Test loading decks from ./decks directory
    // These are our test decks - mostly should load successfully
    // Known failure: monored.dck contains "Ojer Axonil, Deepest Might" (double-faced card)
    // Current baseline: 1 failure (out of 29 decks, ~96.6% success rate)
    let local_decks = PathBuf::from("decks");
    test_deck_directory(&card_db, &local_decks, 1, "decks").await?;

    Ok(())
}
