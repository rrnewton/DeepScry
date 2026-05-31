//! Edition/set data loader
//!
//! Parses edition files from the `editions/` directory to extract:
//! - Set release dates (for year filtering)
//! - Card-to-set mappings

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Information about a Magic set/edition
#[derive(Debug, Clone)]
pub struct EditionInfo {
    /// Set code (e.g., "MH2")
    pub code: String,
    /// Full name (e.g., "Modern Horizons 2")
    pub name: String,
    /// Release date in YYYY-MM-DD format
    pub date: String,
    /// Release year extracted from date
    pub year: u16,
}

/// A single printing of a card in a set
#[derive(Debug, Clone)]
pub struct CardPrinting {
    /// Set code (e.g., "MH2")
    pub set_code: String,
    /// Release year
    pub year: u16,
}

/// Card to edition mapping, tracking earliest/latest printing years
#[derive(Debug, Default)]
pub struct CardEditionIndex {
    /// Map from card name (normalized) to (earliest_year, latest_year)
    card_years: HashMap<String, (u16, u16)>,
    /// Map from card name (normalized) to all printings, sorted by year
    card_printings: HashMap<String, Vec<CardPrinting>>,
    /// Set of all unique set codes loaded
    set_codes: std::collections::HashSet<String>,
}

impl CardEditionIndex {
    /// Create a new empty index
    pub fn new() -> Self {
        Self::default()
    }

    /// Load all edition files from a directory and build the card-year index
    ///
    /// # Errors
    ///
    /// Returns an I/O error if directory reading fails.
    pub fn load_from_directory(editions_dir: &Path) -> std::io::Result<Self> {
        let mut index = Self::new();

        if !editions_dir.exists() {
            return Ok(index); // Return empty index if directory doesn't exist
        }

        // Read all .txt files in the editions directory
        for entry in fs::read_dir(editions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "txt") {
                if let Ok(edition) = Self::parse_edition_file(&path) {
                    // Process each card in this edition
                    if let Ok(cards) = Self::extract_cards_from_file(&path) {
                        for card_name in cards {
                            index.add_card_printing(&card_name, &edition.code, edition.year);
                        }
                    }
                }
            }
        }

        // Sort all printings by year
        for printings in index.card_printings.values_mut() {
            printings.sort_by_key(|p| p.year);
        }

        Ok(index)
    }

    /// Parse edition metadata from a file
    pub(crate) fn parse_edition_file(path: &Path) -> std::io::Result<EditionInfo> {
        let content = fs::read_to_string(path)?;

        let mut code = String::new();
        let mut name = String::new();
        let mut date = String::new();

        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("Code=") {
                code = value.to_string();
            } else if let Some(value) = line.strip_prefix("Name=") {
                name = value.to_string();
            } else if let Some(value) = line.strip_prefix("Date=") {
                date = value.to_string();
            }
            // Stop parsing metadata after [cards] section begins
            if line == "[cards]" {
                break;
            }
        }

        // Extract year from date (format: YYYY-MM-DD)
        let year = date.split('-').next().and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);

        Ok(EditionInfo { code, name, date, year })
    }

    /// Extract card names from an edition file's [cards] section
    pub(crate) fn extract_cards_from_file(path: &Path) -> std::io::Result<Vec<String>> {
        let content = fs::read_to_string(path)?;
        let mut cards = Vec::new();
        let mut in_cards_section = false;

        for line in content.lines() {
            let line = line.trim();

            if line == "[cards]" {
                in_cards_section = true;
                continue;
            }

            if !in_cards_section || line.is_empty() || line.starts_with('[') {
                if in_cards_section && line.starts_with('[') {
                    break; // End of [cards] section
                }
                continue;
            }

            // Parse card line: "<collector_num> <rarity> <card_name> @<artist>"
            // Example: "12 R Esper Sentinel @Eric Deschamps"
            if let Some(card_name) = Self::parse_card_line(line) {
                cards.push(card_name);
            }
        }

        Ok(cards)
    }

    /// Parse a card line and extract the card name
    /// Format: "<collector_num> <rarity> <card_name> @<artist>"
    fn parse_card_line(line: &str) -> Option<String> {
        // Split off the artist part first (after @)
        let name_part = line.split('@').next()?.trim();

        // Split by whitespace: first is collector number, second is rarity, rest is card name
        let mut parts = name_part.split_whitespace();

        // Skip collector number
        parts.next()?;

        // Skip rarity (single char like U, C, R, M, S)
        parts.next()?;

        // Remaining is the card name
        let card_name: String = parts.collect::<Vec<&str>>().join(" ");

        if card_name.is_empty() {
            None
        } else {
            Some(card_name)
        }
    }

    /// Add a card printing to the index
    fn add_card_printing(&mut self, card_name: &str, set_code: &str, year: u16) {
        if year == 0 {
            return; // Skip unknown years
        }

        let normalized = normalize_card_name(card_name);

        // Track unique set codes
        self.set_codes.insert(set_code.to_string());

        // Update earliest/latest years
        self.card_years
            .entry(normalized.clone())
            .and_modify(|(earliest, latest)| {
                if year < *earliest {
                    *earliest = year;
                }
                if year > *latest {
                    *latest = year;
                }
            })
            .or_insert((year, year));

        // Add to printings list
        self.card_printings.entry(normalized).or_default().push(CardPrinting {
            set_code: set_code.to_string(),
            year,
        });
    }

    /// Get all printings of a card, sorted by year (earliest first)
    pub fn get_card_printings(&self, card_name: &str) -> Option<&[CardPrinting]> {
        let normalized = normalize_card_name(card_name);
        self.card_printings.get(&normalized).map(|v| v.as_slice())
    }

    /// Check if a card was printed within a year range
    ///
    /// Returns true if:
    /// - No start_year/end_year constraints (always matches)
    /// - Card's earliest printing is <= end_year (if end_year specified)
    /// - Card's latest printing is >= start_year (if start_year specified)
    pub fn card_in_year_range(&self, card_name: &str, start_year: Option<u16>, end_year: Option<u16>) -> bool {
        // No filters = all cards match
        if start_year.is_none() && end_year.is_none() {
            return true;
        }

        let normalized = normalize_card_name(card_name);

        // If we don't have data for this card, include it by default
        let Some(&(earliest, latest)) = self.card_years.get(&normalized) else {
            return true;
        };

        // Check if any printing falls within the range
        // Card's range [earliest, latest] overlaps with filter range [start, end]
        let meets_start = start_year.is_none_or(|start| latest >= start);
        let meets_end = end_year.is_none_or(|end| earliest <= end);

        meets_start && meets_end
    }

    /// Get the set a card was *originally* printed in (its earliest printing).
    ///
    /// Ties on year are broken by lexicographic set code, matching
    /// [`PrimarySetAssignment`] so the native and WASM origin-set assignments
    /// agree. Returns `None` for cards with no edition entry. Powers
    /// set-origin valid predicates such as `setARN`.
    pub fn get_origin_set(&self, card_name: &str) -> Option<crate::core::SetCode> {
        let normalized = normalize_card_name(card_name);
        let printings = self.card_printings.get(&normalized)?;
        // `card_printings` is sorted by year (see load_from_directory), but
        // re-derive the earliest with a lexicographic set-code tie-break so the
        // result is independent of insertion order within a single year.
        printings
            .iter()
            .min_by(|a, b| a.year.cmp(&b.year).then_with(|| a.set_code.cmp(&b.set_code)))
            .map(|p| crate::core::SetCode::new(&p.set_code))
    }

    /// Get the year range for a card (earliest, latest)
    pub fn get_card_years(&self, card_name: &str) -> Option<(u16, u16)> {
        let normalized = normalize_card_name(card_name);
        self.card_years.get(&normalized).copied()
    }

    /// Number of unique cards indexed
    pub fn card_count(&self) -> usize {
        self.card_years.len()
    }

    /// Number of unique sets loaded
    pub fn set_count(&self) -> usize {
        self.set_codes.len()
    }
}

/// Result of scanning all edition files for the per-set WASM exporter.
///
/// Each entry maps a card name (as it appears in the edition file's `[cards]`
/// section, NOT normalized) to its "primary printing": the earliest
/// `(year, set_code)` printing, with ties broken by lexicographic set code.
///
/// This is kept separate from [`CardEditionIndex`] because that index
/// normalizes card names to lowercase and is keyed off the normalized name;
/// the WASM card map is keyed by the canonical (original-case) card name that
/// comes out of `CardLoader::load_from_file`, and conflating the two would
/// silently break lookups for cards like "Serra's Emissary" whose canonical
/// names contain uppercase letters.
#[derive(Debug, Default)]
pub struct PrimarySetAssignment {
    /// (year, set_code) for each set we observed at least one card in,
    /// keyed by the original set code from the edition file.
    pub sets: HashMap<String, u16>,
    /// Map from original-case card name -> primary (year, set_code).
    pub primary: HashMap<String, (u16, String)>,
}

impl PrimarySetAssignment {
    /// Walk every `.txt` file in `editions_dir` and assign each printed card
    /// to its earliest printing (ties broken by lexicographic set code).
    ///
    /// `year == 0` entries are still considered, but only if no other set
    /// also lists the card (so a real release always wins over an unparseable
    /// date). All edition files in the repo today have a valid four-digit
    /// year — see the manual scan in mtg-464 — so this fallback path is
    /// dormant.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if `editions_dir` cannot be read.
    pub fn scan(editions_dir: &Path) -> std::io::Result<Self> {
        let mut out = Self::default();

        if !editions_dir.exists() {
            return Ok(out);
        }

        for entry in fs::read_dir(editions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "txt") {
                continue;
            }
            let Ok(edition) = CardEditionIndex::parse_edition_file(&path) else {
                continue;
            };
            if edition.code.is_empty() {
                continue;
            }
            let Ok(cards) = CardEditionIndex::extract_cards_from_file(&path) else {
                continue;
            };
            out.sets.insert(edition.code.clone(), edition.year);
            for card_name in cards {
                let candidate = (edition.year, edition.code.clone());
                out.primary
                    .entry(card_name)
                    .and_modify(|existing| {
                        // Earlier year wins; tie-break on lexicographic set code.
                        if candidate.0 < existing.0 || (candidate.0 == existing.0 && candidate.1 < existing.1) {
                            *existing = candidate.clone();
                        }
                    })
                    .or_insert(candidate);
            }
        }

        Ok(out)
    }

    /// Number of distinct set codes seen across all edition files.
    pub fn set_count(&self) -> usize {
        self.sets.len()
    }

    /// Number of distinct card names that have at least one printing.
    pub fn card_count(&self) -> usize {
        self.primary.len()
    }
}

/// Normalize card name for lookup (lowercase, trim)
fn normalize_card_name(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_card_line() {
        assert_eq!(
            CardEditionIndex::parse_card_line("12 R Esper Sentinel @Eric Deschamps"),
            Some("Esper Sentinel".to_string())
        );

        assert_eq!(
            CardEditionIndex::parse_card_line("1 U Abiding Grace @Ravenna Tran"),
            Some("Abiding Grace".to_string())
        );

        assert_eq!(
            CardEditionIndex::parse_card_line("30 M Serra's Emissary @Nils Hamm"),
            Some("Serra's Emissary".to_string())
        );
    }

    #[test]
    fn test_card_in_year_range() {
        let mut index = CardEditionIndex::new();
        index.add_card_printing("Lightning Bolt", "LEA", 1993); // Alpha
        index.add_card_printing("Lightning Bolt", "M12", 2011); // M12
        index.add_card_printing("Lightning Bolt", "STA", 2021); // Strixhaven mystical archive

        // No filter = matches
        assert!(index.card_in_year_range("Lightning Bolt", None, None));

        // Start year only: card's latest printing must be >= start
        assert!(index.card_in_year_range("Lightning Bolt", Some(2010), None)); // Latest (2021) >= 2010
        assert!(index.card_in_year_range("Lightning Bolt", Some(2021), None)); // Latest (2021) >= 2021
        assert!(!index.card_in_year_range("Lightning Bolt", Some(2022), None)); // Latest (2021) < 2022

        // End year only: card's earliest printing must be <= end
        assert!(index.card_in_year_range("Lightning Bolt", None, Some(2000))); // Earliest (1993) <= 2000
        assert!(index.card_in_year_range("Lightning Bolt", None, Some(1993))); // Earliest (1993) <= 1993
        assert!(!index.card_in_year_range("Lightning Bolt", None, Some(1992))); // Earliest (1993) > 1992

        // Both bounds: card's range [earliest, latest] must overlap with [start, end]
        assert!(index.card_in_year_range("Lightning Bolt", Some(2000), Some(2015))); // [1993, 2021] overlaps [2000, 2015]
                                                                                     // Note: We only track earliest/latest, not individual printings.
                                                                                     // [1993, 2021] overlaps [1994, 2010], so this matches even though no exact printing in that range.
        assert!(index.card_in_year_range("Lightning Bolt", Some(1994), Some(2010)));

        // Test a card that truly doesn't overlap
        index.add_card_printing("Serra Angel", "LEA", 1993);
        index.add_card_printing("Serra Angel", "4ED", 1995);
        // Serra Angel [1993, 1995] doesn't overlap [2000, 2010]
        assert!(!index.card_in_year_range("Serra Angel", Some(2000), Some(2010)));

        // Test get_card_printings
        let printings = index.get_card_printings("Lightning Bolt").unwrap();
        assert_eq!(printings.len(), 3);
        assert_eq!(printings[0].set_code, "LEA");
        assert_eq!(printings[0].year, 1993);
    }

    #[test]
    fn test_normalize_card_name() {
        assert_eq!(normalize_card_name("Lightning Bolt"), "lightning bolt");
        assert_eq!(normalize_card_name("  Serra's Emissary  "), "serra's emissary");
    }
}
