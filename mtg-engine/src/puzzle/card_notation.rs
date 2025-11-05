//! Card notation parsing
//!
//! Parses card strings like "Mountain|Tapped" or "Goblin Guide|Id:50|Counters:P1P1=3"

use crate::{core::CounterType, MtgError, Result};
use std::collections::HashMap;

/// A card modifier from pipe-delimited notation
#[derive(Debug, Clone, PartialEq)]
pub enum CardModifier {
    /// Card has a specific ID for referencing
    Id(u32),
    /// Card is from a specific set
    Set(String),
    /// Card uses specific art variant
    Art(u32),
    /// Card is tapped
    Tapped,
    /// Card has summoning sickness
    SummonSick,
    /// Card has damage marked
    Damage(i32),
    /// Card has counters
    Counters(HashMap<CounterType, i32>),
    /// Card is attached to another card by ID
    AttachedTo(u32),
    /// Card is enchanting a player
    EnchantingPlayer(usize), // player index
    /// Card is transformed (DFC back side)
    Transformed,
    /// Card is flipped
    Flipped,
    /// Card is face down
    FaceDown,
    /// Card is manifested
    Manifested,
    /// Card is renowned
    Renowned,
    /// Card is monstrous
    Monstrous,
    /// Card is attacking (optional target planeswalker ID)
    Attacking(Option<u32>),
    /// Card's owner (if different from zone owner)
    Owner(usize), // player index
    /// Chosen colors
    ChosenColor(Vec<String>),
    /// Chosen type
    ChosenType(String),
    /// Named cards
    NamedCard(Vec<String>),
    /// Remembered cards by ID
    RememberedCards(Vec<u32>),
    /// Imprinted cards by ID
    Imprinting(Vec<u32>),
    /// Exiled with card ID
    ExiledWith(u32),
    /// Is a commander
    IsCommander,
    /// Is the ring bearer
    IsRingBearer,
    /// Don't trigger ETB effects
    NoETBTrigs,
    /// Token indicator (for future token support)
    Token(String),
}

/// Parse a single card definition from notation like "CardName|Mod1|Mod2"
pub fn parse_card_notation(notation: &str) -> Result<(String, Vec<CardModifier>)> {
    let parts: Vec<&str> = notation.split('|').collect();

    if parts.is_empty() {
        return Err(MtgError::ParseError("Empty card notation".to_string()));
    }

    // First part is the card name (or token indicator)
    let card_name = parts[0].trim();

    // Check if it's a token
    if card_name.starts_with("t:") || card_name.starts_with("T:") {
        return Ok((
            card_name[2..].to_string(),
            vec![CardModifier::Token(card_name[2..].to_string())],
        ));
    }

    let mut modifiers = Vec::new();

    // Parse remaining parts as modifiers
    for part in &parts[1..] {
        let part = part.trim();

        if part.is_empty() {
            continue;
        }

        // Parse key:value modifiers
        if let Some((key, value)) = part.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim();

            match key.as_str() {
                "id" => {
                    let id = value
                        .parse()
                        .map_err(|_| MtgError::ParseError(format!("Invalid Id value: {}", value)))?;
                    modifiers.push(CardModifier::Id(id));
                }
                "set" => {
                    modifiers.push(CardModifier::Set(value.to_string()));
                }
                "art" => {
                    let art = value
                        .parse()
                        .map_err(|_| MtgError::ParseError(format!("Invalid Art value: {}", value)))?;
                    modifiers.push(CardModifier::Art(art));
                }
                "damage" => {
                    let damage = value
                        .parse()
                        .map_err(|_| MtgError::ParseError(format!("Invalid Damage value: {}", value)))?;
                    modifiers.push(CardModifier::Damage(damage));
                }
                "counters" => {
                    modifiers.push(CardModifier::Counters(parse_counters(value)?));
                }
                "attachedto" => {
                    let id = value
                        .parse()
                        .map_err(|_| MtgError::ParseError(format!("Invalid AttachedTo value: {}", value)))?;
                    modifiers.push(CardModifier::AttachedTo(id));
                }
                "enchantingplayer" => {
                    let player_idx = parse_player_ref(value)?;
                    modifiers.push(CardModifier::EnchantingPlayer(player_idx));
                }
                "attacking" => {
                    let target = if value.is_empty() {
                        None
                    } else {
                        Some(
                            value
                                .parse()
                                .map_err(|_| MtgError::ParseError(format!("Invalid Attacking value: {}", value)))?,
                        )
                    };
                    modifiers.push(CardModifier::Attacking(target));
                }
                "owner" => {
                    let player_idx = parse_player_ref(value)?;
                    modifiers.push(CardModifier::Owner(player_idx));
                }
                "chosencolor" => {
                    let colors = value.split(',').map(|s| s.trim().to_string()).collect();
                    modifiers.push(CardModifier::ChosenColor(colors));
                }
                "chosentype" => {
                    modifiers.push(CardModifier::ChosenType(value.to_string()));
                }
                "namedcard" => {
                    let cards = value.split(',').map(|s| s.trim().to_string()).collect();
                    modifiers.push(CardModifier::NamedCard(cards));
                }
                "rememberedcards" => {
                    let ids = parse_id_list(value)?;
                    modifiers.push(CardModifier::RememberedCards(ids));
                }
                "imprinting" => {
                    let ids = parse_id_list(value)?;
                    modifiers.push(CardModifier::Imprinting(ids));
                }
                "exiledwith" => {
                    let id = value
                        .parse()
                        .map_err(|_| MtgError::ParseError(format!("Invalid ExiledWith value: {}", value)))?;
                    modifiers.push(CardModifier::ExiledWith(id));
                }
                _ => {
                    // Unknown modifier, skip for forward compatibility
                }
            }
        } else {
            // Parse boolean flags
            match part.to_lowercase().as_str() {
                "tapped" => modifiers.push(CardModifier::Tapped),
                "summonsick" => modifiers.push(CardModifier::SummonSick),
                "transformed" => modifiers.push(CardModifier::Transformed),
                "flipped" => modifiers.push(CardModifier::Flipped),
                "facedown" => modifiers.push(CardModifier::FaceDown),
                "manifested" => modifiers.push(CardModifier::Manifested),
                "renowned" => modifiers.push(CardModifier::Renowned),
                "monstrous" => modifiers.push(CardModifier::Monstrous),
                "iscommander" => modifiers.push(CardModifier::IsCommander),
                "isringbearer" => modifiers.push(CardModifier::IsRingBearer),
                "noetbtrigs" => modifiers.push(CardModifier::NoETBTrigs),
                _ => {
                    // Unknown flag, skip for forward compatibility
                }
            }
        }
    }

    Ok((card_name.to_string(), modifiers))
}

/// Parse counter string like "P1P1=3,LOYALTY=5"
fn parse_counters(s: &str) -> Result<HashMap<CounterType, i32>> {
    let mut counters = HashMap::new();

    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((counter_name, count_str)) = part.split_once('=') {
            let counter_type = parse_counter_type(counter_name.trim())?;
            let count = count_str
                .trim()
                .parse()
                .map_err(|_| MtgError::ParseError(format!("Invalid counter count: {}", count_str)))?;
            counters.insert(counter_type, count);
        }
    }

    Ok(counters)
}

/// Parse counter type from string
fn parse_counter_type(s: &str) -> Result<CounterType> {
    // Parse common counter types found in puzzle files
    match s.trim().to_uppercase().as_str() {
        "P1P1" | "+1/+1" => Ok(CounterType::P1P1),
        "M1M1" | "-1/-1" => Ok(CounterType::M1M1),
        "LOYALTY" => Ok(CounterType::Loyalty),
        "POISON" => Ok(CounterType::Poison),
        "ENERGY" => Ok(CounterType::Energy),
        "CHARGE" => Ok(CounterType::Charge),
        "AGE" => Ok(CounterType::Age),
        "STORAGE" => Ok(CounterType::Storage),
        "REPR" | "REPRIEVE" => Ok(CounterType::Reprieve),
        "LORE" => Ok(CounterType::Lore),
        "OIL" => Ok(CounterType::Oil),
        "STASH" => Ok(CounterType::Stash),
        "DEF" | "DEFENSE" => Ok(CounterType::Defense),
        "REV" => Ok(CounterType::Rev),
        _ => Err(MtgError::ParseError(format!(
            "Unknown counter type: {}. Supported types: P1P1, M1M1, LOYALTY, POISON, ENERGY, CHARGE, AGE, STORAGE, REPR, LORE, OIL, STASH",
            s
        ))),
    }
}

/// Parse player reference like "P0", "P1", "HUMAN", "AI"
fn parse_player_ref(s: &str) -> Result<usize> {
    let s = s.trim().to_uppercase();
    match s.as_str() {
        "HUMAN" | "P0" => Ok(0),
        "AI" | "P1" => Ok(1),
        _ => {
            if s.starts_with('P') && s.len() == 2 {
                s[1..]
                    .parse()
                    .map_err(|_| MtgError::ParseError(format!("Invalid player ref: {}", s)))
            } else {
                Err(MtgError::ParseError(format!("Invalid player ref: {}", s)))
            }
        }
    }
}

/// Parse comma-separated list of IDs
fn parse_id_list(s: &str) -> Result<Vec<u32>> {
    s.split(',')
        .map(|id_str| {
            id_str
                .trim()
                .parse()
                .map_err(|_| MtgError::ParseError(format!("Invalid ID in list: {}", id_str)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_card() {
        let (name, mods) = parse_card_notation("Mountain").unwrap();
        assert_eq!(name, "Mountain");
        assert!(mods.is_empty());
    }

    #[test]
    fn test_parse_tapped_card() {
        let (name, mods) = parse_card_notation("Forest|Tapped").unwrap();
        assert_eq!(name, "Forest");
        assert_eq!(mods.len(), 1);
        assert!(matches!(mods[0], CardModifier::Tapped));
    }

    #[test]
    fn test_parse_card_with_id() {
        let (name, mods) = parse_card_notation("Goblin Guide|Id:50").unwrap();
        assert_eq!(name, "Goblin Guide");
        assert_eq!(mods.len(), 1);
        assert!(matches!(mods[0], CardModifier::Id(50)));
    }

    #[test]
    fn test_parse_card_with_counters() {
        let (name, mods) = parse_card_notation("Tarmogoyf|Counters:P1P1=3").unwrap();
        assert_eq!(name, "Tarmogoyf");
        assert_eq!(mods.len(), 1);
        if let CardModifier::Counters(counters) = &mods[0] {
            assert_eq!(counters.get(&CounterType::P1P1), Some(&3));
        } else {
            panic!("Expected Counters modifier");
        }
    }

    #[test]
    fn test_parse_card_multiple_modifiers() {
        let (name, mods) = parse_card_notation("Serra Angel|Id:10|Tapped|Damage:3|SummonSick").unwrap();
        assert_eq!(name, "Serra Angel");
        assert_eq!(mods.len(), 4);
    }

    #[test]
    fn test_parse_attached_card() {
        let (name, mods) = parse_card_notation("Pacifism|AttachedTo:18").unwrap();
        assert_eq!(name, "Pacifism");
        assert!(matches!(mods[0], CardModifier::AttachedTo(18)));
    }

    #[test]
    fn test_parse_token() {
        let (name, mods) = parse_card_notation("t:1/1 G Saproling").unwrap();
        assert_eq!(name, "1/1 G Saproling");
        assert!(matches!(&mods[0], CardModifier::Token(s) if s == "1/1 G Saproling"));
    }

    #[test]
    fn test_parse_player_ref() {
        assert_eq!(parse_player_ref("HUMAN").unwrap(), 0);
        assert_eq!(parse_player_ref("AI").unwrap(), 1);
        assert_eq!(parse_player_ref("P0").unwrap(), 0);
        assert_eq!(parse_player_ref("P1").unwrap(), 1);
    }

    #[test]
    fn test_parse_counters_multiple() {
        let counters = parse_counters("P1P1=3,LOYALTY=5").unwrap();
        assert_eq!(counters.get(&CounterType::P1P1), Some(&3));
        assert_eq!(counters.get(&CounterType::Loyalty), Some(&5));
    }
}
