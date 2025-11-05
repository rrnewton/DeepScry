//! Puzzle metadata parsing
//!
//! Handles the \[metadata\] section of .pzl files

use crate::{MtgError, Result};

/// Puzzle difficulty level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    VeryHard,
    // MTG rarity-based difficulties (used in newer puzzle sets)
    Uncommon,
    Rare,
    Mythic,
    Special,
}

impl std::str::FromStr for Difficulty {
    type Err = MtgError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "easy" => Ok(Difficulty::Easy),
            "medium" => Ok(Difficulty::Medium),
            "hard" => Ok(Difficulty::Hard),
            "very hard" => Ok(Difficulty::VeryHard),
            "uncommon" => Ok(Difficulty::Uncommon),
            "rare" => Ok(Difficulty::Rare),
            "mythic" => Ok(Difficulty::Mythic),
            "special" => Ok(Difficulty::Special),
            _ => Err(MtgError::ParseError(format!("Invalid difficulty: {}", s))),
        }
    }
}

/// Goal type for puzzle completion
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalType {
    /// Win the game within turn limit
    Win,
    /// Survive until turn limit + 1
    Survive,
    /// Destroy all specified permanents
    DestroySpecifiedPermanents { targets: String },
    /// Remove specified permanents from battlefield
    RemoveSpecifiedPermanents { targets: String },
    /// Kill specified creatures
    KillSpecifiedCreatures { targets: String },
    /// Put specified permanent on battlefield
    PlaySpecifiedPermanent { targets: String, count: usize },
    /// Gain control of specified permanents
    GainControlOfPermanents { targets: String },
    /// Win before opponent's next turn
    WinBeforeOpponentTurn,
}

impl GoalType {
    /// Parse goal type from string, with optional targets
    pub fn parse(goal_str: &str, targets: Option<&str>, target_count: usize) -> Result<Self> {
        let goal_lower = goal_str.trim().to_lowercase();

        Ok(match goal_lower.as_str() {
            "win" => GoalType::Win,
            "survive" => GoalType::Survive,
            "destroy specified permanents" => GoalType::DestroySpecifiedPermanents {
                targets: targets.unwrap_or("Creature.OppCtrl").to_string(),
            },
            "destroy specified creatures" => GoalType::DestroySpecifiedPermanents {
                targets: targets.unwrap_or("Creature.OppCtrl").to_string(),
            },
            "remove specified permanents from the battlefield" => GoalType::RemoveSpecifiedPermanents {
                targets: targets.unwrap_or("Creature.OppCtrl").to_string(),
            },
            "kill specified creatures" => GoalType::KillSpecifiedCreatures {
                targets: targets.unwrap_or("Creature.OppCtrl").to_string(),
            },
            "put the specified permanent on the battlefield" | "play the specified permanent" => {
                GoalType::PlaySpecifiedPermanent {
                    targets: targets
                        .ok_or_else(|| {
                            MtgError::ParseError("PlaySpecifiedPermanent goal requires Targets field".to_string())
                        })?
                        .to_string(),
                    count: target_count,
                }
            }
            "gain control of specified permanents" => GoalType::GainControlOfPermanents {
                targets: targets.unwrap_or("Card.inZoneBattlefield+OppCtrl").to_string(),
            },
            "win before opponent's next turn" => GoalType::WinBeforeOpponentTurn,
            _ => return Err(MtgError::ParseError(format!("Unknown goal type: {}", goal_str))),
        })
    }
}

/// Puzzle metadata from \[metadata\] section
#[derive(Debug, Clone)]
pub struct PuzzleMetadata {
    pub name: String,
    pub url: Option<String>,
    pub goal: GoalType,
    pub turns: u32,
    pub difficulty: Difficulty,
    pub description: Option<String>,
    pub targets: Option<String>,
    pub target_count: usize,
    pub human_control: bool,
}

impl Default for PuzzleMetadata {
    fn default() -> Self {
        Self {
            name: "Unnamed Puzzle".to_string(),
            url: None,
            goal: GoalType::Win,
            turns: 1,
            difficulty: Difficulty::Easy,
            description: None,
            targets: None,
            target_count: 1,
            human_control: false,
        }
    }
}

impl PuzzleMetadata {
    /// Parse metadata from lines in \[metadata\] section
    pub fn parse(lines: &[String]) -> Result<Self> {
        let mut meta = PuzzleMetadata::default();

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Split on first colon
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value.trim();

                match key.as_str() {
                    "name" => meta.name = value.to_string(),
                    "url" => meta.url = Some(value.to_string()),
                    "goal" => {
                        // Don't parse goal yet, need targets first
                        // Will be parsed in finalize()
                    }
                    "turns" => {
                        meta.turns = value
                            .parse()
                            .map_err(|_| MtgError::ParseError(format!("Invalid turns value: {}", value)))?;
                    }
                    "difficulty" => {
                        meta.difficulty = value.parse()?;
                    }
                    "description" => meta.description = Some(value.to_string()),
                    "targets" => meta.targets = Some(value.to_string()),
                    "targetcount" => {
                        meta.target_count = value
                            .parse()
                            .map_err(|_| MtgError::ParseError(format!("Invalid target count: {}", value)))?;
                    }
                    "humancontrol" => {
                        meta.human_control = value.trim().to_lowercase() == "true";
                    }
                    _ => {
                        // Unknown metadata field, ignore for forward compatibility
                    }
                }
            }
        }

        // Parse goal after all fields are set (needs targets)
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim().to_lowercase() == "goal" {
                    meta.goal = GoalType::parse(value.trim(), meta.targets.as_deref(), meta.target_count)?;
                    break;
                }
            }
        }

        Ok(meta)
    }

    /// Get a human-readable description of the puzzle goal
    pub fn goal_description(&self) -> String {
        let mut desc = String::new();

        if self.human_control {
            desc.push_str(
                "WARNING: This puzzle is human-controlled (no AI player). \
                 You will make all decisions for your opponent.\n\n",
            );
        }

        desc.push_str(&self.name);
        desc.push_str("\nDifficulty: ");
        desc.push_str(match self.difficulty {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
            Difficulty::VeryHard => "Very Hard",
            Difficulty::Uncommon => "Uncommon",
            Difficulty::Rare => "Rare",
            Difficulty::Mythic => "Mythic",
            Difficulty::Special => "Special",
        });

        desc.push_str("\n\nGoal: ");
        desc.push_str(match &self.goal {
            GoalType::Win => "Win the game",
            GoalType::Survive => "Survive",
            GoalType::DestroySpecifiedPermanents { .. } => "Destroy specified permanents",
            GoalType::RemoveSpecifiedPermanents { .. } => "Remove specified permanents",
            GoalType::KillSpecifiedCreatures { .. } => "Kill specified creatures",
            GoalType::PlaySpecifiedPermanent { .. } => "Play the specified permanent",
            GoalType::GainControlOfPermanents { .. } => "Gain control of specified permanents",
            GoalType::WinBeforeOpponentTurn => "Win before opponent's next turn",
        });

        desc.push_str(&format!("\nTurns Limit: {}", self.turns));

        if let Some(description) = &self.description {
            desc.push_str("\n\n");
            desc.push_str(&description.replace("\\n", "\n"));
        }

        desc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_difficulty() {
        assert_eq!("easy".parse::<Difficulty>().unwrap(), Difficulty::Easy);
        assert_eq!("Medium".parse::<Difficulty>().unwrap(), Difficulty::Medium);
        assert_eq!("HARD".parse::<Difficulty>().unwrap(), Difficulty::Hard);
        assert!("invalid".parse::<Difficulty>().is_err());
    }

    #[test]
    fn test_parse_goal_win() {
        let goal = GoalType::parse("Win", None, 1).unwrap();
        assert_eq!(goal, GoalType::Win);
    }

    #[test]
    fn test_parse_goal_survive() {
        let goal = GoalType::parse("Survive", None, 1).unwrap();
        assert_eq!(goal, GoalType::Survive);
    }

    #[test]
    fn test_parse_metadata_basic() {
        let lines = vec![
            "Name:Test Puzzle".to_string(),
            "Goal:Win".to_string(),
            "Turns:2".to_string(),
            "Difficulty:Medium".to_string(),
        ];

        let meta = PuzzleMetadata::parse(&lines).unwrap();
        assert_eq!(meta.name, "Test Puzzle");
        assert_eq!(meta.turns, 2);
        assert_eq!(meta.difficulty, Difficulty::Medium);
        assert_eq!(meta.goal, GoalType::Win);
    }

    #[test]
    fn test_parse_metadata_with_url() {
        let lines = vec![
            "Name:Web Puzzle".to_string(),
            "URL:https://example.com/puzzle".to_string(),
            "Goal:Win".to_string(),
            "Turns:1".to_string(),
            "Difficulty:Easy".to_string(),
        ];

        let meta = PuzzleMetadata::parse(&lines).unwrap();
        assert_eq!(meta.url, Some("https://example.com/puzzle".to_string()));
    }

    #[test]
    fn test_goal_description() {
        let meta = PuzzleMetadata {
            name: "My Puzzle".to_string(),
            url: None,
            goal: GoalType::Win,
            turns: 1,
            difficulty: Difficulty::Hard,
            description: Some("Test description".to_string()),
            targets: None,
            target_count: 1,
            human_control: false,
        };

        let desc = meta.goal_description();
        assert!(desc.contains("My Puzzle"));
        assert!(desc.contains("Hard"));
        assert!(desc.contains("Win the game"));
        assert!(desc.contains("Test description"));
    }
}
