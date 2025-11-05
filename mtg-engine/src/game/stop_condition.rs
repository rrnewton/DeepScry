//! Stop conditions for game snapshots
//!
//! Defines when to stop a game and save a snapshot based on player choices.

use crate::core::PlayerId;

/// Which player's choices to track for stop condition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPlayer {
    /// Only count Player 1's choices
    P1,
    /// Only count Player 2's choices
    P2,
    /// Count both players' choices
    Both,
}

/// Stop condition for game snapshots
#[derive(Debug, Clone)]
pub struct StopCondition {
    /// Which player's choices to count
    pub player: StopPlayer,
    /// Number of choices before stopping
    pub choice_count: usize,
}

impl StopCondition {
    /// Create a new stop condition
    pub fn new(player: StopPlayer, choice_count: usize) -> Self {
        StopCondition { player, choice_count }
    }

    /// Parse a stop condition from a string
    ///
    /// Format: <NUM>[:[p1|p2]]
    /// Examples: "3" (both players), "1:p1" (only p1), "5:p2" (only p2)
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();

        match parts.len() {
            1 => {
                // Simple format: just a number means "both"
                let choice_count = parts[0]
                    .parse::<usize>()
                    .map_err(|_| format!("invalid choice count '{}'", parts[0]))?;
                Ok(StopCondition {
                    player: StopPlayer::Both,
                    choice_count,
                })
            }
            2 => {
                // Extended format: <NUM>:<player>
                let choice_count = parts[0]
                    .parse::<usize>()
                    .map_err(|_| format!("invalid choice count '{}'", parts[0]))?;

                let player = match parts[1] {
                    "p1" => StopPlayer::P1,
                    "p2" => StopPlayer::P2,
                    _ => return Err(format!("invalid player '{}' (expected: p1 or p2)", parts[1])),
                };

                Ok(StopCondition { player, choice_count })
            }
            _ => Err(format!("invalid format '{}' (expected: <NUM> or <NUM>:[p1|p2])", s)),
        }
    }

    /// Check if this condition applies to a given player's choice
    ///
    /// # Arguments
    /// * `p1_id` - The ID of Player 1
    /// * `player_id` - The ID of the player making the choice
    ///
    /// # Returns
    /// `true` if this player's choices should be counted toward the limit
    pub fn applies_to(&self, p1_id: PlayerId, player_id: PlayerId) -> bool {
        match self.player {
            StopPlayer::P1 => player_id == p1_id,
            StopPlayer::P2 => player_id != p1_id,
            StopPlayer::Both => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;

    #[test]
    fn test_parse_stop_condition() {
        // Simple format - just a number means "both"
        let cond = StopCondition::parse("5").unwrap();
        assert!(matches!(cond.player, StopPlayer::Both));
        assert_eq!(cond.choice_count, 5);

        // Extended format - with player specification
        let cond = StopCondition::parse("1:p1").unwrap();
        assert!(matches!(cond.player, StopPlayer::P1));
        assert_eq!(cond.choice_count, 1);

        let cond = StopCondition::parse("10:p2").unwrap();
        assert!(matches!(cond.player, StopPlayer::P2));
        assert_eq!(cond.choice_count, 10);
    }

    #[test]
    fn test_parse_invalid_format() {
        assert!(StopCondition::parse("invalid").is_err());
        assert!(StopCondition::parse("5:both").is_err()); // "both" is not supported, use just "5"
        assert!(StopCondition::parse("5:p3").is_err());
        assert!(StopCondition::parse("5:p1:extra").is_err());
    }

    #[test]
    fn test_applies_to() {
        let p1_id = EntityId::new(1);
        let p2_id = EntityId::new(2);

        let cond_p1 = StopCondition::new(StopPlayer::P1, 5);
        assert!(cond_p1.applies_to(p1_id, p1_id));
        assert!(!cond_p1.applies_to(p1_id, p2_id));

        let cond_p2 = StopCondition::new(StopPlayer::P2, 5);
        assert!(!cond_p2.applies_to(p1_id, p1_id));
        assert!(cond_p2.applies_to(p1_id, p2_id));

        let cond_both = StopCondition::new(StopPlayer::Both, 5);
        assert!(cond_both.applies_to(p1_id, p1_id));
        assert!(cond_both.applies_to(p1_id, p2_id));
    }
}
