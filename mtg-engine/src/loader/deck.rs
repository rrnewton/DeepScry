//! Deck file loader (.dck format)

use crate::{MtgError, Result};
#[cfg(feature = "native")]
use std::fs;
#[cfg(feature = "native")]
use std::path::Path;

/// Import problem type for deck loading issues
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportProblemKind {
    /// Line could not be parsed (invalid format)
    ParseFail,
    /// Card name not found in database
    CardMissing,
}

/// Represents a problem encountered during deck import
#[derive(Debug, Clone)]
pub struct ImportProblem {
    /// The kind of problem
    pub kind: ImportProblemKind,
    /// The original line text
    pub line_text: String,
    /// The card name if it was extracted (for CardMissing problems)
    pub card_name: Option<String>,
}

impl ImportProblem {
    /// Create a parse failure problem
    pub fn parse_fail(line_text: &str) -> Self {
        Self {
            kind: ImportProblemKind::ParseFail,
            line_text: line_text.to_string(),
            card_name: None,
        }
    }

    /// Create a card missing problem
    pub fn card_missing(card_name: &str, line_text: &str) -> Self {
        Self {
            kind: ImportProblemKind::CardMissing,
            line_text: line_text.to_string(),
            card_name: Some(card_name.to_string()),
        }
    }

    /// Get a display label for the problem
    pub fn label(&self) -> String {
        match self.kind {
            ImportProblemKind::ParseFail => format!("[PARSE_FAIL] {}", self.line_text),
            ImportProblemKind::CardMissing => format!("[CARD_MISSING] {}", self.line_text),
        }
    }
}

/// Result of parsing a deck with tracked problems
#[derive(Debug)]
pub struct DeckParseResult {
    /// Successfully parsed deck entries
    pub deck_list: DeckList,
    /// Problems encountered during parsing
    pub problems: Vec<ImportProblem>,
}

/// Which section of the deck file we're currently parsing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeckSection {
    Main,
    Sideboard,
    Commander,
}

/// Deck loader for .dck files
pub struct DeckLoader;

impl DeckLoader {
    /// Load a deck from a .dck file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    #[cfg(feature = "native")]
    pub fn load_from_file(path: &Path) -> Result<DeckList> {
        let content = fs::read_to_string(path).map_err(MtgError::IoError)?;
        Self::parse(&content)
    }

    /// Load a deck from file with problem tracking
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    #[cfg(feature = "native")]
    pub fn load_from_file_with_problems(path: &Path) -> Result<DeckParseResult> {
        let content = fs::read_to_string(path).map_err(MtgError::IoError)?;
        Ok(Self::parse_with_problems(&content))
    }

    /// Parse a deck from its text content
    ///
    /// # Errors
    ///
    /// Returns an error if the deck is empty or cannot be parsed.
    pub fn parse(content: &str) -> Result<DeckList> {
        let result = Self::parse_with_problems(content);
        if result.deck_list.main_deck.is_empty() && result.problems.is_empty() {
            return Err(MtgError::InvalidDeckFormat("Empty deck".to_string()));
        }
        Ok(result.deck_list)
    }

    /// Parse a deck from text content with problem tracking
    ///
    /// Unlike `parse()`, this method tracks lines that fail to parse instead
    /// of silently ignoring them. Missing cards are NOT detected at this stage;
    /// call `validate_cards()` after loading the card database to detect those.
    pub fn parse_with_problems(content: &str) -> DeckParseResult {
        let mut main_deck = Vec::new();
        let mut sideboard = Vec::new();
        let mut commanders = Vec::new();
        let mut problems = Vec::new();
        let mut current_section = DeckSection::Main;

        for line in content.lines() {
            let original_line = line;
            let line = line.trim();

            // Skip empty lines, comments, and metadata
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Check for section markers
            if line.starts_with('[') {
                if line.contains("Sideboard") {
                    current_section = DeckSection::Sideboard;
                } else if line.contains("Commander") {
                    current_section = DeckSection::Commander;
                } else if line.contains("Main") {
                    current_section = DeckSection::Main;
                }
                continue;
            }

            // Skip metadata lines (e.g., "Name=Deck Name")
            if line.contains('=') && !line.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }

            // Format: "1 Card Name" or "1 Card Name|SET" or "1 Card Name+|SET"
            let parsed = if let Some((count_str, rest)) = line.split_once(' ') {
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

                    if !card_name.is_empty() {
                        Some(DeckEntry { card_name, count })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match parsed {
                Some(entry) => match current_section {
                    DeckSection::Main => main_deck.push(entry),
                    DeckSection::Sideboard => sideboard.push(entry),
                    DeckSection::Commander => commanders.push(entry),
                },
                None => {
                    // Line failed to parse - track it as a problem
                    problems.push(ImportProblem::parse_fail(original_line));
                }
            }
        }

        DeckParseResult {
            deck_list: DeckList {
                main_deck,
                sideboard,
                commanders,
            },
            problems,
        }
    }

    /// Validate parsed deck entries against known card names
    ///
    /// Returns additional CardMissing problems for cards not in the database.
    pub fn validate_cards(deck_list: &DeckList, known_cards: &std::collections::HashSet<&str>) -> Vec<ImportProblem> {
        let mut problems = Vec::new();

        for entry in &deck_list.main_deck {
            if !known_cards.contains(entry.card_name.as_str()) {
                let line_text = format!("{} {}", entry.count, entry.card_name);
                problems.push(ImportProblem::card_missing(&entry.card_name, &line_text));
            }
        }

        for entry in &deck_list.sideboard {
            if !known_cards.contains(entry.card_name.as_str()) {
                let line_text = format!("{} {}", entry.count, entry.card_name);
                problems.push(ImportProblem::card_missing(&entry.card_name, &line_text));
            }
        }

        problems
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
    /// Commander card(s) - populated from [Commander] section in .dck files.
    /// When non-empty, the game should use Commander format rules (40 life, command zone, etc.)
    pub commanders: Vec<DeckEntry>,
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

    /// Returns true if this deck has a commander (uses Commander format)
    pub fn is_commander(&self) -> bool {
        !self.commanders.is_empty()
    }

    /// Get unique card names from main deck, sideboard, and commanders
    pub fn unique_card_names(&self) -> Vec<String> {
        let mut names = std::collections::HashSet::new();
        for entry in &self.main_deck {
            names.insert(entry.card_name.clone());
        }
        for entry in &self.sideboard {
            names.insert(entry.card_name.clone());
        }
        for entry in &self.commanders {
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

        // Commander section (if any)
        if !self.commanders.is_empty() {
            content.push_str("\n[Commander]\n");
            for entry in &self.commanders {
                content.push_str(&format!("{} {}\n", entry.count, entry.card_name));
            }
        }

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
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
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
            commanders: vec![],
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

    #[test]
    fn test_parse_with_problems() {
        let content = r#"
[metadata]
Name=Problem Deck

[Main]
4 Lightning Bolt
invalid line
3 Mountain
abc def ghi
"#;

        let result = DeckLoader::parse_with_problems(content);

        // Should parse 2 valid entries
        assert_eq!(result.deck_list.main_deck.len(), 2);
        assert_eq!(result.deck_list.main_deck[0].card_name, "Lightning Bolt");
        assert_eq!(result.deck_list.main_deck[1].card_name, "Mountain");

        // Should have 2 parse problems
        assert_eq!(result.problems.len(), 2);
        assert_eq!(result.problems[0].kind, ImportProblemKind::ParseFail);
        assert!(result.problems[0].line_text.contains("invalid line"));
        assert_eq!(result.problems[1].kind, ImportProblemKind::ParseFail);
        assert!(result.problems[1].line_text.contains("abc def ghi"));
    }

    #[test]
    fn test_validate_cards() {
        use std::collections::HashSet;

        let deck = DeckList {
            main_deck: vec![
                DeckEntry {
                    card_name: "Lightning Bolt".to_string(),
                    count: 4,
                },
                DeckEntry {
                    card_name: "Fake Card".to_string(),
                    count: 2,
                },
                DeckEntry {
                    card_name: "Mountain".to_string(),
                    count: 20,
                },
            ],
            sideboard: vec![DeckEntry {
                card_name: "Another Fake".to_string(),
                count: 3,
            }],
            commanders: vec![],
        };

        let known_cards: HashSet<&str> = ["Lightning Bolt", "Mountain", "Forest"].iter().cloned().collect();
        let problems = DeckLoader::validate_cards(&deck, &known_cards);

        // Should find 2 missing cards
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].kind, ImportProblemKind::CardMissing);
        assert_eq!(problems[0].card_name, Some("Fake Card".to_string()));
        assert_eq!(problems[1].kind, ImportProblemKind::CardMissing);
        assert_eq!(problems[1].card_name, Some("Another Fake".to_string()));
    }

    #[test]
    fn test_import_problem_label() {
        let parse_fail = ImportProblem::parse_fail("bad line");
        assert!(parse_fail.label().contains("[PARSE_FAIL]"));
        assert!(parse_fail.label().contains("bad line"));

        let card_missing = ImportProblem::card_missing("Fake Card", "4 Fake Card");
        assert!(card_missing.label().contains("[CARD_MISSING]"));
        assert!(card_missing.label().contains("4 Fake Card"));
    }

    #[test]
    fn test_parse_commander_deck() {
        let content = r#"
[metadata]
Name=Commander Test

[Commander]
1 Chandra, Torch of Defiance

[Main]
18 Mountain
1 Lightning Bolt

[Sideboard]
"#;

        let deck = DeckLoader::parse(content).unwrap();
        assert_eq!(deck.commanders.len(), 1);
        assert_eq!(deck.commanders[0].card_name, "Chandra, Torch of Defiance");
        assert_eq!(deck.commanders[0].count, 1);
        assert_eq!(deck.main_deck.len(), 2);
        assert!(deck.is_commander());
    }
}
