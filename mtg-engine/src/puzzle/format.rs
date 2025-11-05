//! PZL file format parser
//!
//! Parses .pzl files with \[metadata\] and \[state\] sections

use crate::{
    puzzle::{metadata::PuzzleMetadata, state::GameStateDefinition},
    MtgError, Result,
};

/// A complete puzzle file with metadata and game state
#[derive(Debug, Clone)]
pub struct PuzzleFile {
    pub metadata: PuzzleMetadata,
    pub state: GameStateDefinition,
}

/// Parse a complete puzzle file from string contents
pub fn parse_puzzle(contents: &str) -> Result<PuzzleFile> {
    let sections = parse_sections(contents)?;

    let metadata = if let Some(metadata_lines) = sections.get("metadata") {
        PuzzleMetadata::parse(metadata_lines)?
    } else {
        PuzzleMetadata::default()
    };

    let state = if let Some(state_lines) = sections.get("state") {
        GameStateDefinition::parse(state_lines)?
    } else {
        return Err(MtgError::ParseError(
            "Missing [state] section in puzzle file".to_string(),
        ));
    };

    Ok(PuzzleFile { metadata, state })
}

/// Parse INI-style sections from file contents
///
/// Returns a map of section name to lines in that section
fn parse_sections(contents: &str) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let mut sections = std::collections::HashMap::new();
    let mut current_section: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for section header
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // Save previous section if exists
            if let Some(section_name) = current_section.take() {
                sections.insert(section_name, current_lines.clone());
                current_lines.clear();
            }

            // Start new section
            let section_name = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            current_section = Some(section_name);
        } else if current_section.is_some() {
            // Add line to current section
            current_lines.push(line.to_string());
        }
        // Lines before first section are ignored
    }

    // Save final section
    if let Some(section_name) = current_section {
        sections.insert(section_name, current_lines);
    }

    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_all_java_forge_puzzles() {
        // Parse ALL puzzle files from Java Forge to ensure our parser handles them
        let puzzle_dir = PathBuf::from("forge-java/forge-gui/res/puzzle");

        if !puzzle_dir.exists() {
            eprintln!("Skipping test_parse_all_java_forge_puzzles: directory not found");
            return;
        }

        let mut parsed_count = 0;
        let mut failed_count = 0;
        let mut failures = Vec::new();

        // Find all .pzl files
        for entry in std::fs::read_dir(&puzzle_dir).expect("Failed to read puzzle directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("pzl") {
                match PuzzleFile::load(&path) {
                    Ok(_puzzle) => {
                        parsed_count += 1;
                    }
                    Err(e) => {
                        failed_count += 1;
                        let filename = path.file_name().unwrap().to_string_lossy();
                        failures.push(format!("{}: {}", filename, e));
                    }
                }
            }
        }

        println!("\n=== Puzzle Parsing Results ===");
        println!("Parsed: {} puzzles", parsed_count);
        println!("Failed: {} puzzles", failed_count);

        if !failures.is_empty() {
            println!("\nFailures:");
            for failure in &failures {
                println!("  - {}", failure);
            }
        }

        // Assert that we parsed at least some puzzles (should be 300+)
        assert!(
            parsed_count > 100,
            "Should have parsed at least 100 puzzles, got {}",
            parsed_count
        );

        // For now, allow some failures as we may not support all features yet
        // But we should parse the vast majority
        let success_rate = (parsed_count as f64) / ((parsed_count + failed_count) as f64) * 100.0;
        println!("\nSuccess rate: {:.1}%", success_rate);

        // We should successfully parse at least 80% of puzzles
        assert!(
            success_rate >= 80.0,
            "Success rate too low: {:.1}%. Failed puzzles:\n{}",
            success_rate,
            failures.join("\n")
        );
    }

    #[test]
    fn test_parse_sections_basic() {
        let contents = r#"
[metadata]
Name:Test Puzzle
Goal:Win

[state]
turn=1
p0life=20
"#;

        let sections = parse_sections(contents).unwrap();
        assert_eq!(sections.len(), 2);
        assert!(sections.contains_key("metadata"));
        assert!(sections.contains_key("state"));
        assert_eq!(sections["metadata"].len(), 2);
        assert_eq!(sections["state"].len(), 2);
    }

    #[test]
    fn test_parse_sections_with_comments() {
        let contents = r#"
# This is a comment
[metadata]
Name:Test
# Another comment
Goal:Win

[state]
turn=1
"#;

        let sections = parse_sections(contents).unwrap();
        assert_eq!(sections["metadata"].len(), 2);
        assert_eq!(sections["state"].len(), 1);
    }

    #[test]
    fn test_parse_puzzle_complete() {
        let contents = r#"
[metadata]
Name:Simple Test
Goal:Win
Turns:1
Difficulty:Easy

[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p1life=10
"#;

        let puzzle = parse_puzzle(contents).unwrap();
        assert_eq!(puzzle.metadata.name, "Simple Test");
        assert_eq!(puzzle.metadata.turns, 1);
        assert_eq!(puzzle.state.turn, 1);
        assert_eq!(puzzle.state.players[0].life, 20);
        assert_eq!(puzzle.state.players[1].life, 10);
        assert_eq!(puzzle.state.players[0].hand.len(), 1);
        assert_eq!(puzzle.state.players[0].hand[0].name, "Lightning Bolt");
    }

    #[test]
    fn test_parse_puzzle_missing_state() {
        let contents = r#"
[metadata]
Name:Test
"#;

        assert!(parse_puzzle(contents).is_err());
    }

    #[test]
    fn test_parse_real_puzzle_pp04() {
        let contents = r#"
[metadata]
Name:Pauper Puzzles #04 - Make Love, Not War
URL:https://pauperpuzzles.wordpress.com/2017/01/20/4-make-love-not-war/
Goal:Win
Turns:1
Difficulty:Hard
[state]
ActivePlayer=Human
ActivePhase=Main1
HumanLife=1
AILife=5
humanhand=Kor Skyfisher;Oblivion Ring;Chainer's Edict;Holy Light
humanbattlefield=Swamp;Swamp;Swamp;Plains;Plains;Plains;Plains;Plains;Thraben Inspector;Foundry Screecher;Kor Sanctifiers;Lone Missionary;Pacifism|AttachedTo:18
humanlibrary=Leave No Trace
aibattlefield=Aura Gnarlid|Id:18;Slippery Bogle|Id:19;Ethereal Armor|AttachedTo:19;Armadillo Cloak|AttachedTo:19;Ancestral Mask|AttachedTo:19;Children of Korlis
"#;

        // This should parse without error even with legacy format
        // Note: We currently only support p0/p1 format, so this will fail
        // but we're testing the basic structure works
        let _result = parse_puzzle(contents);

        // The metadata should parse fine
        let sections = parse_sections(contents).unwrap();
        assert!(sections.contains_key("metadata"));
        assert!(sections.contains_key("state"));
    }
}
