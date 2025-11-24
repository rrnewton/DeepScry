//! Async card database for on-demand and eager loading
//!
//! Supports two loading modes:
//! 1. Lazy loading: Load cards on-demand when requested (parallel I/O)
//! 2. Eager loading: Load all cards upfront from cardsfolder (parallel I/O)

use crate::loader::card::{CardDefinition, CardLoader};
use crate::{MtgError, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Instant;

/// Convert card name to file path
/// "Lightning Bolt" -> "cardsfolder/l/lightning_bolt.txt"
/// "All Hallow's Eve" -> "cardsfolder/a/all_hallows_eve.txt"
/// "Juzám Djinn" -> "cardsfolder/j/juzam_djinn.txt" (Unicode normalized to ASCII)
/// "Spiked Corridor // Torture Pit" -> "cardsfolder/s/spiked_corridor_torture_pit.txt" (split cards)
/// "Minsc & Boo, Timeless Heroes" -> "cardsfolder/m/minsc_boo_timeless_heroes.txt" (ampersand removed)
/// "Summon: Choco/Mog" -> "cardsfolder/s/summon_choco_mog.txt" (forward slash becomes underscore)
/// Removes apostrophes, ampersands, and other special characters to match Java Forge convention
fn card_name_to_path(cardsfolder: &Path, card_name: &str) -> PathBuf {
    // First normalize Unicode characters to ASCII (e.g., "á" -> "a", "ñ" -> "n")
    // This is necessary because cardsfolder uses ASCII-only filenames
    let ascii = deunicode::deunicode(card_name);

    // Then normalize for filesystem: lowercase, replace/remove special chars
    // Using iterator-based approach for efficiency
    // Special handling for multi-character patterns
    let normalized: String = ascii
        .replace(" // ", "_") // Split cards: "Spiked Corridor // Torture Pit" -> "Spiked Corridor_Torture Pit"
        .replace(" & ", "_") // Ampersand: "Minsc & Boo, Timeless Heroes" -> "Minsc_Boo, Timeless Heroes"
        .to_lowercase()
        .chars()
        .map(|c| match c {
            ' ' | '-' | '/' => '_',                     // Spaces, hyphens, and slashes become underscores
            '\'' | ',' | ':' | '!' | '?' | '&' => '\0', // Remove these characters
            _ => c,
        })
        .filter(|&c| c != '\0') // Remove marked characters
        .collect();

    let first_char = normalized.chars().next().unwrap_or('_');

    cardsfolder
        .join(first_char.to_string())
        .join(format!("{normalized}.txt"))
}

/// Async card database with lazy and eager loading support
pub struct CardDatabase {
    /// Base directory containing card files
    cardsfolder: PathBuf,
    /// Cache of loaded cards (shared, thread-safe)
    /// Using Arc<CardDefinition> to avoid cloning on every access
    cards: Arc<RwLock<HashMap<String, Arc<CardDefinition>>>>,
}

impl CardDatabase {
    /// Create a new database pointing at a cardsfolder
    pub fn new(cardsfolder: PathBuf) -> Self {
        CardDatabase {
            cardsfolder,
            cards: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load a single card by name (async, with caching)
    /// Returns None if card file doesn't exist
    /// Returns Arc<CardDefinition> to avoid cloning
    pub async fn get_card(&self, name: &str) -> Result<Option<Arc<CardDefinition>>> {
        let name_lower = name.to_lowercase();

        // Check cache first
        {
            let cards = self.cards.read().await;
            if let Some(card) = cards.get(&name_lower) {
                return Ok(Some(Arc::clone(card)));
            }
        }

        // Not in cache, try to load from disk
        let path = card_name_to_path(&self.cardsfolder, name);

        // Try exact match first
        let actual_path = if path.exists() {
            path
        } else {
            // Double faced cards file naming convention currently has both names - so try to prefix match.
            // This matches Java Forge behavior in CardStorageReader.findFileForCard()
            match Self::find_file_by_prefix(&path).await {
                Some(p) => p,
                None => return Ok(None),
            }
        };

        // Load asynchronously
        match Self::load_card_async(actual_path).await {
            Ok(card_def) => {
                // Cache the loaded card in an Arc
                let card_arc = Arc::new(card_def);
                let mut cards = self.cards.write().await;
                cards.insert(name_lower, Arc::clone(&card_arc));
                Ok(Some(card_arc))
            }
            Err(e) => {
                // Card file exists but failed to parse - this is a fatal error
                Err(e)
            }
        }
    }

    /// Find a card file by prefix matching (for double-faced cards)
    /// Example: "ojer_axonil_deepest_might" matches "ojer_axonil_deepest_might_temple_of_power.txt"
    async fn find_file_by_prefix(expected_path: &Path) -> Option<PathBuf> {
        let parent = expected_path.parent()?;
        let file_stem = expected_path.file_stem()?.to_str()?;

        // Read directory entries
        let mut read_dir = tokio::fs::read_dir(parent).await.ok()?;

        while let Some(entry) = read_dir.next_entry().await.ok()? {
            let entry_path = entry.path();
            if let Some(entry_stem) = entry_path.file_stem().and_then(|s| s.to_str()) {
                if entry_stem.starts_with(file_stem) && entry_path.extension().and_then(|s| s.to_str()) == Some("txt") {
                    return Some(entry_path);
                }
            }
        }

        None
    }

    /// Load multiple cards in parallel
    /// Returns timing information
    pub async fn load_cards(&self, names: &[String]) -> Result<(usize, std::time::Duration)> {
        let start = Instant::now();

        // Spawn tasks for all cards in parallel - track names for error reporting
        let mut tasks = Vec::new();
        for name in names {
            let name = name.clone();
            let db = self.clone_handle();
            let task_name = name.clone(); // Keep name for error reporting
            tasks.push((task_name, tokio::spawn(async move { db.get_card(&name).await })));
        }

        // Wait for all to complete - fail fast on any error
        let mut loaded = 0;
        for (card_name, task) in tasks {
            match task.await {
                Ok(Ok(Some(_))) => loaded += 1,
                Ok(Ok(None)) => {
                    return Err(crate::MtgError::InvalidCardFormat(format!(
                        "Card file not found: '{}' (expected path: {})",
                        card_name,
                        card_name_to_path(&self.cardsfolder, &card_name).display()
                    )))
                }
                Ok(Err(e)) => return Err(e),
                Err(e) => {
                    return Err(crate::MtgError::InvalidCardFormat(format!(
                        "Task join error for card '{}': {e}",
                        card_name
                    )))
                }
            }
        }

        let duration = start.elapsed();
        Ok((loaded, duration))
    }

    /// Eagerly load all cards from cardsfolder (parallel)
    /// Uses streaming discovery - starts loading cards while still walking directory tree
    /// Returns (cards_loaded, duration)
    pub async fn eager_load(&self) -> Result<(usize, std::time::Duration)> {
        let start = Instant::now();

        // Stream card file paths using parallel directory walking (jwalk + rayon)
        // Key optimization: spawn loading tasks AS paths are discovered, not after
        let cardsfolder = self.cardsfolder.clone();

        let (path_tx, mut path_rx) = tokio::sync::mpsc::unbounded_channel();
        let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel::<Result<CardDefinition>>();

        // Spawn directory walking in a blocking task (jwalk uses rayon internally)
        tokio::task::spawn_blocking(move || {
            for entry in jwalk::WalkDir::new(&cardsfolder).skip_hidden(false).into_iter() {
                match entry {
                    Ok(entry) => {
                        if entry.file_type().is_file() {
                            if let Some(ext) = entry.path().extension() {
                                if ext == "txt" {
                                    // Fail fast: if we can't send, the receiver is gone
                                    if path_tx.send(entry.path()).is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Fail fast: directory walking errors are fatal
                        eprintln!("Fatal error walking directory: {e}");
                        return;
                    }
                }
            }
        });

        // Spawn task consumer - starts loading cards immediately as paths arrive
        tokio::spawn(async move {
            let mut count = 0;
            while let Some(path) = path_rx.recv().await {
                count += 1;
                let result_tx = result_tx.clone();
                tokio::spawn(async move {
                    // Send the result (success or error) - don't filter
                    let result = Self::load_card_async(path.clone()).await;
                    if let Err(e) = &result {
                        eprintln!("Fatal error loading card from {path:?}: {e}");
                    }
                    let _ = result_tx.send(result);
                });
            }
            count
        });

        // Collect loaded cards as they complete - fail fast on any error
        let mut cards_map = HashMap::new();
        while let Some(card_result) = result_rx.recv().await {
            let card_def = card_result?; // Fail fast: propagate card loading errors
            let name_lower = card_def.name.to_lowercase();
            cards_map.insert(name_lower, Arc::new(card_def));
        }

        let loaded = cards_map.len();
        println!("Loaded {loaded} cards via streaming discovery");

        // Update cache
        let mut cards = self.cards.write().await;
        *cards = cards_map;

        let duration = start.elapsed();
        Ok((loaded, duration))
    }

    /// Load a card from a file asynchronously
    async fn load_card_async(path: PathBuf) -> Result<CardDefinition> {
        let contents = tokio::fs::read_to_string(&path).await.map_err(MtgError::IoError)?;

        CardLoader::parse(&contents).map_err(|e| {
            // Enhance error message with file path for easier debugging
            MtgError::InvalidCardFormat(format!("Failed to parse card file '{}': {}", path.display(), e))
        })
    }

    /// Get a clone of the database handle (shares the cache)
    pub fn clone_handle(&self) -> Self {
        CardDatabase {
            cardsfolder: self.cardsfolder.clone(),
            cards: Arc::clone(&self.cards),
        }
    }

    /// Synchronous check if card exists in cache
    pub async fn contains(&self, name: &str) -> bool {
        let name_lower = name.to_lowercase();
        let cards = self.cards.read().await;
        cards.contains_key(&name_lower)
    }

    /// Get number of cards currently loaded
    pub async fn len(&self) -> usize {
        let cards = self.cards.read().await;
        cards.len()
    }

    /// Check if database is empty
    pub async fn is_empty(&self) -> bool {
        let cards = self.cards.read().await;
        cards.is_empty()
    }

    /// Get all cards as an iterator (for statistics and analysis)
    /// Returns an owned collection to iterate over (guards can't escape async)
    pub async fn all_cards(&self) -> Vec<Arc<CardDefinition>> {
        let cards = self.cards.read().await;
        cards.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_card_name_to_path() {
        let cardsfolder = PathBuf::from("cardsfolder");

        let path = card_name_to_path(&cardsfolder, "Lightning Bolt");
        assert_eq!(path, PathBuf::from("cardsfolder/l/lightning_bolt.txt"));

        let path = card_name_to_path(&cardsfolder, "Black Lotus");
        assert_eq!(path, PathBuf::from("cardsfolder/b/black_lotus.txt"));

        // Test special character handling
        let path = card_name_to_path(&cardsfolder, "All Hallow's Eve");
        assert_eq!(path, PathBuf::from("cardsfolder/a/all_hallows_eve.txt"));

        let path = card_name_to_path(&cardsfolder, "Nevinyrral's Disk");
        assert_eq!(path, PathBuf::from("cardsfolder/n/nevinyrrals_disk.txt"));

        let path = card_name_to_path(&cardsfolder, "Mishra's Factory");
        assert_eq!(path, PathBuf::from("cardsfolder/m/mishras_factory.txt"));

        // Test Unicode normalization
        let path = card_name_to_path(&cardsfolder, "Juzám Djinn");
        assert_eq!(path, PathBuf::from("cardsfolder/j/juzam_djinn.txt"));

        let path = card_name_to_path(&cardsfolder, "Dandan");
        assert_eq!(path, PathBuf::from("cardsfolder/d/dandan.txt"));

        // Test split cards
        let path = card_name_to_path(&cardsfolder, "Spiked Corridor // Torture Pit");
        assert_eq!(path, PathBuf::from("cardsfolder/s/spiked_corridor_torture_pit.txt"));

        // Test ampersand removal
        let path = card_name_to_path(&cardsfolder, "Minsc & Boo, Timeless Heroes");
        assert_eq!(path, PathBuf::from("cardsfolder/m/minsc_boo_timeless_heroes.txt"));

        // Test forward slash
        let path = card_name_to_path(&cardsfolder, "Summon: Choco/Mog");
        assert_eq!(path, PathBuf::from("cardsfolder/s/summon_choco_mog.txt"));
    }

    #[tokio::test]
    async fn test_lazy_loading() {
        let cardsfolder = PathBuf::from("cardsfolder");
        if !cardsfolder.exists() {
            return;
        }

        let db = CardDatabase::new(cardsfolder);

        // Should start empty
        assert!(db.is_empty().await);

        // Load a card
        let card = db.get_card("Lightning Bolt").await.unwrap();
        assert!(card.is_some());

        // Should now have 1 card
        assert_eq!(db.len().await, 1);

        // Loading again should hit cache
        let card2 = db.get_card("Lightning Bolt").await.unwrap();
        assert!(card2.is_some());

        // Still only 1 card (hit cache)
        assert_eq!(db.len().await, 1);
    }

    #[tokio::test]
    async fn test_parallel_loading() {
        let cardsfolder = PathBuf::from("cardsfolder");
        if !cardsfolder.exists() {
            return;
        }

        let db = CardDatabase::new(cardsfolder);

        let cards = vec![
            "Lightning Bolt".to_string(),
            "Mountain".to_string(),
            "Forest".to_string(),
        ];

        let (loaded, duration) = db.load_cards(&cards).await.unwrap();
        assert_eq!(loaded, 3);
        println!("Loaded {loaded} cards in {duration:?}");
    }
}
