//! Deck file loader (.dck format)

use crate::{MtgError, Result};
#[cfg(feature = "native")]
use std::fs;
#[cfg(feature = "native")]
use std::path::Path;

/// Deck loader for .dck files
pub struct DeckLoader;

impl DeckLoader {
    /// Load a deck from a .dck file
    #[cfg(feature = "native")]
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

            // Format: "1 Card Name" or "1 Card Name|SET" or "1 Card Name+|SET" (+ indicates foil)
            if let Some((count_str, rest)) = line.split_once(' ') {
                if let Ok(count) = count_str.parse::<u8>() {
                    // Extract card name (before pipe if present)
                    let mut card_name = if let Some((name, _set)) = rest.split_once('|') {
                        name.trim().to_string()
                    } else {
                        rest.trim().to_string()
                    };

                    // Strip trailing '+' which indicates foil/premium in Forge deck files
                    if card_name.ends_with('+') {
                        card_name.pop();
                        card_name = card_name.trim().to_string();
                    }

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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeckEntry {
    pub card_name: String,
    pub count: u8,
}

/// Represents a complete deck list
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

    /// Format deck as .dck file content
    pub fn to_dck_format(&self, name: Option<&str>) -> String {
        let mut content = String::new();

        // Metadata section
        content.push_str("[metadata]\n");
        content.push_str(&format!("Name={}\n", name.unwrap_or("Deck")));

        // Main deck section
        content.push_str("\n[Main]\n");
        for entry in &self.main_deck {
            content.push_str(&format!("{} {}\n", entry.count, entry.card_name));
        }

        // Sideboard section (if any)
        if !self.sideboard.is_empty() {
            content.push_str("\n[Sideboard]\n");
            for entry in &self.sideboard {
                content.push_str(&format!("{} {}\n", entry.count, entry.card_name));
            }
        }

        content
    }

    /// Save deck to a .dck file
    #[cfg(feature = "native")]
    pub fn save_to_file(&self, path: &Path, name: Option<&str>) -> Result<()> {
        let content = self.to_dck_format(name);
        fs::write(path, content).map_err(MtgError::IoError)
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

    #[test]
    fn test_parse_foil_cards() {
        let content = r#"
[metadata]
Name=Foil Test

[Main]
1 Master of Etherium+|ALA
2 Lightning Bolt+|M10
1 Grizzly Bears
"#;

        let deck = DeckLoader::parse(content).unwrap();
        assert_eq!(deck.main_deck.len(), 3);

        // The '+' suffix should be stripped from foil cards
        assert_eq!(deck.main_deck[0].card_name, "Master of Etherium");
        assert_eq!(deck.main_deck[0].count, 1);

        assert_eq!(deck.main_deck[1].card_name, "Lightning Bolt");
        assert_eq!(deck.main_deck[1].count, 2);

        assert_eq!(deck.main_deck[2].card_name, "Grizzly Bears");
        assert_eq!(deck.main_deck[2].count, 1);
    }

    #[test]
    fn test_to_dck_format_roundtrip() {
        // Create a deck
        let deck = DeckList {
            main_deck: vec![
                DeckEntry {
                    card_name: "Lightning Bolt".to_string(),
                    count: 4,
                },
                DeckEntry {
                    card_name: "Mountain".to_string(),
                    count: 20,
                },
            ],
            sideboard: vec![DeckEntry {
                card_name: "Shock".to_string(),
                count: 2,
            }],
        };

        // Convert to .dck format
        let content = deck.to_dck_format(Some("Test Deck"));

        // Parse it back
        let parsed = DeckLoader::parse(&content).unwrap();

        // Verify roundtrip
        assert_eq!(parsed.main_deck.len(), 2);
        assert_eq!(parsed.main_deck[0].card_name, "Lightning Bolt");
        assert_eq!(parsed.main_deck[0].count, 4);
        assert_eq!(parsed.main_deck[1].card_name, "Mountain");
        assert_eq!(parsed.main_deck[1].count, 20);
        assert_eq!(parsed.sideboard.len(), 1);
        assert_eq!(parsed.sideboard[0].card_name, "Shock");
        assert_eq!(parsed.sideboard[0].count, 2);
    }
}
