//! Cost system for activated abilities
//!
//! Represents the various costs players can pay to activate abilities,
//! such as tapping, paying mana, sacrificing permanents, etc.

use crate::core::{CardId, ManaCost};
use serde::{Deserialize, Serialize};

/// A cost that must be paid to activate an ability
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cost {
    /// Tap the permanent (T)
    Tap,

    /// Untap the permanent (Q or Untap)
    Untap,

    /// Pay mana cost
    Mana(ManaCost),

    /// Tap and pay mana (combined cost like "2 T")
    TapAndMana(ManaCost),

    /// Sacrifice a permanent
    Sacrifice { card_id: CardId },

    /// Sacrifice a permanent matching a pattern (e.g., "Sac<1/Land>", "Sac<1/Creature.Other>")
    /// This is used during parsing and will be resolved to specific card choices at activation time
    SacrificePattern {
        count: u8,
        card_type: String, // e.g., "Land", "Creature.Other", "CARDNAME"
    },

    /// Pay life
    PayLife { amount: i32 },

    /// Discard a card
    Discard { card_id: CardId },

    /// Composite cost (multiple costs combined)
    Composite(Vec<Cost>),

    /// Waterbend cost (Avatar set mechanic - like Convoke)
    /// While paying a waterbend cost, you can tap your artifacts and creatures to help.
    /// Each one pays for {1}. For now, treated as generic mana cost of `amount`.
    Waterbend { amount: u8 },
}

impl Cost {
    /// Parse a cost string from card data (e.g., "T", "2 T", "PayLife<2>", "T Sac<1/Land>")
    pub fn parse(cost_str: &str) -> Option<Self> {
        let trimmed = cost_str.trim();

        // Check for composite costs (space-separated, but not mana symbols like "2 T" which we handle specially)
        // Look for patterns like "T Sac<1/Land>" or "1 Sac<1/CARDNAME>" or "PayLife<1> T"
        let has_sac = trimmed.contains("Sac<");
        let has_pay_life = trimmed.contains("PayLife<");
        let has_tap = trimmed.contains(" T")
            || trimmed.contains(" Tap")
            || trimmed.starts_with("T ")
            || trimmed.starts_with("Tap ");

        // If we have multiple cost components, parse as composite
        if (has_sac || has_pay_life) && (has_tap || trimmed.chars().any(|c| c.is_ascii_digit() || "WUBRG".contains(c)))
        {
            // Parse each component separately
            let mut components = Vec::new();

            // Split by space but keep Sac<...> and PayLife<...> together
            let mut current_token = String::new();
            let mut in_angle_brackets = false;

            for ch in trimmed.chars() {
                if ch == '<' {
                    in_angle_brackets = true;
                    current_token.push(ch);
                } else if ch == '>' {
                    in_angle_brackets = false;
                    current_token.push(ch);
                } else if ch.is_whitespace() && !in_angle_brackets {
                    if !current_token.is_empty() {
                        components.push(current_token.clone());
                        current_token.clear();
                    }
                } else {
                    current_token.push(ch);
                }
            }
            if !current_token.is_empty() {
                components.push(current_token);
            }

            // Parse each component
            let mut costs = Vec::new();
            let mut mana_parts = Vec::new();

            for comp in components {
                if comp == "T" || comp == "Tap" {
                    // We'll handle tap specially with mana if present
                    continue;
                } else if let Some(parsed) = Self::parse_single(&comp) {
                    match parsed {
                        Cost::Mana(_) => mana_parts.push(comp),
                        Cost::Tap
                        | Cost::Untap
                        | Cost::TapAndMana(_)
                        | Cost::Sacrifice { .. }
                        | Cost::SacrificePattern { .. }
                        | Cost::PayLife { .. }
                        | Cost::Discard { .. }
                        | Cost::Composite(_)
                        | Cost::Waterbend { .. } => costs.push(parsed),
                    }
                } else {
                    // Might be mana symbol
                    if comp.chars().any(|c| c.is_ascii_digit() || "WUBRG".contains(c)) {
                        mana_parts.push(comp);
                    }
                }
            }

            // Combine mana parts and add tap if needed
            if !mana_parts.is_empty() || has_tap {
                let mana_cost = if mana_parts.is_empty() {
                    ManaCost::new()
                } else {
                    ManaCost::from_string(&mana_parts.join(" "))
                };

                if has_tap {
                    if mana_cost.cmc() > 0 {
                        costs.push(Cost::TapAndMana(mana_cost));
                    } else {
                        costs.push(Cost::Tap);
                    }
                } else if mana_cost.cmc() > 0 {
                    costs.push(Cost::Mana(mana_cost));
                }
            }

            if costs.len() > 1 {
                return Some(Cost::Composite(costs));
            } else if costs.len() == 1 {
                return Some(costs.into_iter().next().unwrap());
            }
        }

        // Simple single costs
        Self::parse_single(trimmed)
    }

    /// Parse a single (non-composite) cost component
    fn parse_single(trimmed: &str) -> Option<Self> {
        // Simple tap cost
        if trimmed == "T" || trimmed == "Tap" {
            return Some(Cost::Tap);
        }

        // Simple untap cost
        if trimmed == "Q" || trimmed == "Untap" {
            return Some(Cost::Untap);
        }

        // Sacrifice cost (e.g., "Sac<1/Land>", "Sac<1/Creature.Other>", "Sac<1/CARDNAME>")
        if trimmed.starts_with("Sac<") && trimmed.ends_with('>') {
            if let Some(sac_spec) = trimmed.strip_prefix("Sac<").and_then(|s| s.strip_suffix('>')) {
                // Parse format: "N/Type" or "N/Type/description"
                let parts: Vec<&str> = sac_spec.split('/').collect();
                if parts.len() >= 2 {
                    if let Ok(count) = parts[0].parse::<u8>() {
                        let card_type = parts[1].to_string();
                        return Some(Cost::SacrificePattern { count, card_type });
                    }
                }
            }
        }

        // PayLife cost (e.g., "PayLife<2>") - check before mana parsing
        if trimmed.starts_with("PayLife<") && trimmed.ends_with('>') {
            if let Some(amount_str) = trimmed.strip_prefix("PayLife<").and_then(|s| s.strip_suffix('>')) {
                if let Ok(amount) = amount_str.parse::<i32>() {
                    return Some(Cost::PayLife { amount });
                }
            }
        }

        // Waterbend cost (e.g., "Waterbend<3>") - Avatar set mechanic
        // While paying a waterbend cost, you can tap your artifacts and creatures to help.
        // Each one pays for {1}. For now, treat as generic mana.
        if trimmed.starts_with("Waterbend<") && trimmed.ends_with('>') {
            if let Some(amount_str) = trimmed.strip_prefix("Waterbend<").and_then(|s| s.strip_suffix('>')) {
                if let Ok(amount) = amount_str.parse::<u8>() {
                    return Some(Cost::Waterbend { amount });
                }
            }
        }

        // Check for tap + mana combo (e.g., "2 T", "1 R T", "W T")
        if trimmed.contains(" T") || trimmed.contains(" Tap") {
            // Split and parse the mana part
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let mana_parts: Vec<&str> = parts.iter().filter(|&&p| p != "T" && p != "Tap").copied().collect();

            if !mana_parts.is_empty() {
                let mana_str = mana_parts.join(" ");
                let mana_cost = ManaCost::from_string(&mana_str);
                return Some(Cost::TapAndMana(mana_cost));
            }
        }

        // Pure mana cost (no tap)
        // Try to parse as mana cost if it contains numbers or color letters
        if trimmed.chars().any(|c| c.is_ascii_digit() || "WUBRG".contains(c)) {
            let mana_cost = ManaCost::from_string(trimmed);
            // Only treat as mana cost if it's not empty
            if mana_cost.cmc() > 0 {
                return Some(Cost::Mana(mana_cost));
            }
        }

        // If we can't parse it, return None
        None
    }

    /// Check if this cost includes a tap
    pub fn includes_tap(&self) -> bool {
        match self {
            Cost::Tap | Cost::TapAndMana(_) => true,
            Cost::Composite(costs) => costs.iter().any(|c| c.includes_tap()),
            Cost::Untap
            | Cost::Mana(_)
            | Cost::Sacrifice { .. }
            | Cost::SacrificePattern { .. }
            | Cost::PayLife { .. }
            | Cost::Discard { .. }
            | Cost::Waterbend { .. } => false,
        }
    }

    /// Check if this cost includes mana payment
    pub fn includes_mana(&self) -> bool {
        match self {
            Cost::Mana(_) | Cost::TapAndMana(_) | Cost::Waterbend { .. } => true,
            Cost::Composite(costs) => costs.iter().any(|c| c.includes_mana()),
            Cost::Tap
            | Cost::Untap
            | Cost::Sacrifice { .. }
            | Cost::SacrificePattern { .. }
            | Cost::PayLife { .. }
            | Cost::Discard { .. } => false,
        }
    }

    /// Get the mana cost component if present
    pub fn get_mana_cost(&self) -> Option<&ManaCost> {
        match self {
            Cost::Mana(mana) | Cost::TapAndMana(mana) => Some(mana),
            Cost::Composite(costs) => costs.iter().find_map(|c| c.get_mana_cost()),
            Cost::Tap
            | Cost::Untap
            | Cost::Sacrifice { .. }
            | Cost::SacrificePattern { .. }
            | Cost::PayLife { .. }
            | Cost::Discard { .. }
            | Cost::Waterbend { .. } => None,
        }
    }

    /// Get the life cost component if present
    pub fn get_life_cost(&self) -> Option<i32> {
        match self {
            Cost::PayLife { amount } => Some(*amount),
            Cost::Composite(costs) => costs.iter().find_map(|c| c.get_life_cost()),
            Cost::Tap
            | Cost::Untap
            | Cost::Mana(_)
            | Cost::TapAndMana(_)
            | Cost::Sacrifice { .. }
            | Cost::SacrificePattern { .. }
            | Cost::Discard { .. }
            | Cost::Waterbend { .. } => None,
        }
    }

    /// Check if this cost includes sacrificing a permanent
    pub fn requires_sacrifice(&self) -> bool {
        match self {
            Cost::Sacrifice { .. } | Cost::SacrificePattern { .. } => true,
            Cost::Composite(costs) => costs.iter().any(|c| c.requires_sacrifice()),
            Cost::Tap
            | Cost::Untap
            | Cost::Mana(_)
            | Cost::TapAndMana(_)
            | Cost::PayLife { .. }
            | Cost::Discard { .. }
            | Cost::Waterbend { .. } => false,
        }
    }

    /// Get the Waterbend amount if this is a Waterbend cost
    pub fn get_waterbend_amount(&self) -> Option<u8> {
        match self {
            Cost::Waterbend { amount } => Some(*amount),
            Cost::Composite(costs) => costs.iter().find_map(|c| c.get_waterbend_amount()),
            Cost::Tap
            | Cost::Untap
            | Cost::Mana(_)
            | Cost::TapAndMana(_)
            | Cost::Sacrifice { .. }
            | Cost::SacrificePattern { .. }
            | Cost::PayLife { .. }
            | Cost::Discard { .. } => None,
        }
    }

    /// Get the sacrifice pattern if this is a SacrificePattern cost
    /// Returns (count, card_type) where card_type is the pattern string
    pub fn get_sacrifice_pattern(&self) -> Option<(u8, &str)> {
        match self {
            Cost::SacrificePattern { count, card_type } => Some((*count, card_type.as_str())),
            Cost::Composite(costs) => costs.iter().find_map(|c| c.get_sacrifice_pattern()),
            Cost::Tap
            | Cost::Untap
            | Cost::Mana(_)
            | Cost::TapAndMana(_)
            | Cost::Sacrifice { .. }
            | Cost::PayLife { .. }
            | Cost::Discard { .. }
            | Cost::Waterbend { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tap_cost() {
        let cost = Cost::parse("T").unwrap();
        assert_eq!(cost, Cost::Tap);
        assert!(cost.includes_tap());
        assert!(!cost.includes_mana());
    }

    #[test]
    fn test_parse_tap_and_mana() {
        let cost = Cost::parse("2 T").unwrap();
        let Cost::TapAndMana(mana) = &cost else {
            panic!("Expected TapAndMana cost, got {cost:?}");
        };
        assert_eq!(mana.generic, 2);
        assert!(mana.white == 0);
        assert!(cost.includes_tap());
        assert!(cost.includes_mana());
    }

    #[test]
    fn test_parse_colored_mana_tap() {
        let cost = Cost::parse("1 R T").unwrap();
        let Cost::TapAndMana(mana) = &cost else {
            panic!("Expected TapAndMana cost, got {cost:?}");
        };
        assert_eq!(mana.generic, 1);
        assert_eq!(mana.red, 1);
    }

    #[test]
    fn test_parse_pure_mana() {
        let cost = Cost::parse("3").unwrap();
        let Cost::Mana(mana) = &cost else {
            panic!("Expected Mana cost, got {cost:?}");
        };
        assert_eq!(mana.generic, 3);
        assert!(!cost.includes_tap());
        assert!(cost.includes_mana());
    }

    #[test]
    fn test_parse_pay_life() {
        let cost = Cost::parse("PayLife<2>").unwrap();
        assert_eq!(cost, Cost::PayLife { amount: 2 });
    }

    #[test]
    fn test_parse_sacrifice_land() {
        let cost = Cost::parse("Sac<1/Land>").unwrap();
        let Cost::SacrificePattern { count, card_type } = cost else {
            panic!("Expected SacrificePattern cost, got {cost:?}");
        };
        assert_eq!(count, 1);
        assert_eq!(card_type, "Land");
    }

    #[test]
    fn test_parse_sacrifice_creature_other() {
        let cost = Cost::parse("Sac<1/Creature.Other>").unwrap();
        let Cost::SacrificePattern { count, card_type } = cost else {
            panic!("Expected SacrificePattern cost, got {cost:?}");
        };
        assert_eq!(count, 1);
        assert_eq!(card_type, "Creature.Other");
    }

    #[test]
    fn test_parse_composite_tap_sac() {
        let cost = Cost::parse("T Sac<1/Creature.Other>").unwrap();
        let Cost::Composite(costs) = cost else {
            panic!("Expected Composite cost, got {cost:?}");
        };
        assert_eq!(costs.len(), 2);
        assert!(matches!(costs[0], Cost::SacrificePattern { .. }) || matches!(costs[0], Cost::Tap));
        assert!(matches!(costs[1], Cost::SacrificePattern { .. }) || matches!(costs[1], Cost::Tap));
    }

    #[test]
    fn test_parse_waterbend() {
        // Test parsing Waterbend<3> (Flexible Waterbender)
        let cost = Cost::parse("Waterbend<3>").unwrap();
        assert_eq!(cost, Cost::Waterbend { amount: 3 });
        assert!(cost.includes_mana()); // Waterbend requires mana-like payment
        assert!(!cost.includes_tap());
        assert_eq!(cost.get_waterbend_amount(), Some(3));

        // Test parsing Waterbend<5> (Foggy Swamp Vinebender)
        let cost = Cost::parse("Waterbend<5>").unwrap();
        assert_eq!(cost, Cost::Waterbend { amount: 5 });
        assert_eq!(cost.get_waterbend_amount(), Some(5));
    }
}
