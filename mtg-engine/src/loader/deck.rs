//! Deck file loader (.dck format)

use crate::{MtgError, Result};
use std::fs;
use std::path::Path;

/// Deck loader for .dck files
pub struct DeckLoader;

impl DeckLoader {
    /// Load a deck from a .dck file
    pub fn load_from_file(path: &Path) -> Result<DeckList> {
        let content = fs::read_to_string(path).map_err(MtgError::IoError)?;
        Self::parse(&content)
    }

    /// Parse a deck from its text content
    pub fn parse(content: &str) -> Result<DeckList> {
        let mut main_deck = Vec::new();
        let mut sideboard = Vec::new();
        let mut in_sideboard = false;

        for line in content.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                if line.contains("Sideboard") {
                    in_sideboard = true;
                }
                continue;
            }

            // Format: "1 Card Name" or "1 Card Name|SET"
            if let Some((count_str, rest)) = line.split_once(' ') {
                if let Ok(count) = count_str.parse::<u8>() {
                    // Extract card name (before pipe if present)
                    let card_name = if let Some((name, _set)) = rest.split_once('|') {
                        name.trim().to_string()
                    } else {
                        rest.trim().to_string()
                    };

                    let entry = DeckEntry { card_name, count };

                    if in_sideboard {
                        sideboard.push(entry);
                    } else {
                        main_deck.push(entry);
                    }
                }
            }
        }

        if main_deck.is_empty() {
            return Err(MtgError::InvalidDeckFormat("Empty deck".to_string()));
        }

        Ok(DeckList { main_deck, sideboard })
    }
}

/// Represents a deck entry (card name and count)
#[derive(Debug, Clone)]
pub struct DeckEntry {
    pub card_name: String,
    pub count: u8,
}

/// Represents a complete deck list
#[derive(Debug, Clone)]
pub struct DeckList {
    pub main_deck: Vec<DeckEntry>,
    pub sideboard: Vec<DeckEntry>,
}

impl DeckList {
    /// Total cards in main deck
    pub fn total_cards(&self) -> usize {
        self.main_deck.iter().map(|e| e.count as usize).sum()
    }

    /// Total cards in sideboard
    pub fn sideboard_size(&self) -> usize {
        self.sideboard.iter().map(|e| e.count as usize).sum()
    }

    /// Get unique card names from main deck and sideboard
    pub fn unique_card_names(&self) -> Vec<String> {
        let mut names = std::collections::HashSet::new();
        for entry in &self.main_deck {
            names.insert(entry.card_name.clone());
        }
        for entry in &self.sideboard {
            names.insert(entry.card_name.clone());
        }
        names.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_deck() {
        let content = r#"
[metadata]
Name=Test Deck

[Main]
20 Mountain
40 Lightning Bolt

[Sideboard]
15 Shock
"#;

        let deck = DeckLoader::parse(content).unwrap();
        assert_eq!(deck.main_deck.len(), 2);
        assert_eq!(deck.total_cards(), 60);

        assert_eq!(deck.main_deck[0].card_name, "Mountain");
        assert_eq!(deck.main_deck[0].count, 20);

        assert_eq!(deck.main_deck[1].card_name, "Lightning Bolt");
        assert_eq!(deck.main_deck[1].count, 40);

        assert_eq!(deck.sideboard.len(), 1);
        assert_eq!(deck.sideboard[0].card_name, "Shock");
        assert_eq!(deck.sideboard[0].count, 15);
    }
}
