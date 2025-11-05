//! Game state parsing
//!
//! Handles the \[state\] section of .pzl files

use crate::{
    core::CounterType,
    game::Step,
    puzzle::card_notation::{parse_card_notation, CardModifier},
    MtgError, Result,
};
use std::collections::HashMap;

/// A card definition from puzzle notation
#[derive(Debug, Clone)]
pub struct CardDefinition {
    pub name: String,
    pub set_code: Option<String>,
    pub art_id: Option<u32>,
    pub id: Option<u32>,
    pub modifiers: Vec<CardModifier>,
}

impl CardDefinition {
    /// Parse a card from notation string
    pub fn parse(notation: &str) -> Result<Self> {
        let (name, modifiers) = parse_card_notation(notation)?;

        let mut card = CardDefinition {
            name,
            set_code: None,
            art_id: None,
            id: None,
            modifiers: Vec::new(),
        };

        // Extract certain modifiers to top level, keep rest in modifiers list
        for modifier in modifiers {
            match modifier {
                CardModifier::Set(ref set) => card.set_code = Some(set.clone()),
                CardModifier::Art(art) => card.art_id = Some(art),
                CardModifier::Id(id) => card.id = Some(id),
                _ => card.modifiers.push(modifier),
            }
        }

        Ok(card)
    }

    /// Check if this is a token
    pub fn is_token(&self) -> bool {
        self.modifiers.iter().any(|m| matches!(m, CardModifier::Token(_)))
    }
}

/// Player state definition
#[derive(Debug, Clone)]
pub struct PlayerStateDefinition {
    pub life: i32,
    pub lands_played: u32,
    pub lands_played_last_turn: u32,
    pub counters: HashMap<CounterType, i32>,
    pub mana_pool: Vec<String>, // Simplified for Phase 1, will parse properly later
    pub persistent_mana: Vec<String>,
    pub hand: Vec<CardDefinition>,
    pub battlefield: Vec<CardDefinition>,
    pub graveyard: Vec<CardDefinition>,
    pub library: Vec<CardDefinition>,
    pub exile: Vec<CardDefinition>,
    pub command: Vec<CardDefinition>,
}

impl Default for PlayerStateDefinition {
    fn default() -> Self {
        Self {
            life: 20,
            lands_played: 0,
            lands_played_last_turn: 0,
            counters: HashMap::new(),
            mana_pool: Vec::new(),
            persistent_mana: Vec::new(),
            hand: Vec::new(),
            battlefield: Vec::new(),
            graveyard: Vec::new(),
            library: Vec::new(),
            exile: Vec::new(),
            command: Vec::new(),
        }
    }
}

/// Player reference in state file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerRef {
    Player0,
    Player1,
}

impl PlayerRef {
    /// Parse player reference from string (p0, p1, human, ai)
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "p0" | "human" => Ok(PlayerRef::Player0),
            "p1" | "ai" => Ok(PlayerRef::Player1),
            _ => Err(MtgError::ParseError(format!("Invalid player reference: {}", s))),
        }
    }

    /// Convert to index
    pub fn index(&self) -> usize {
        match self {
            PlayerRef::Player0 => 0,
            PlayerRef::Player1 => 1,
        }
    }
}

/// Complete game state definition
#[derive(Debug, Clone)]
pub struct GameStateDefinition {
    pub turn: u32,
    pub active_player: PlayerRef,
    pub active_phase: Step,
    pub players: Vec<PlayerStateDefinition>,
}

impl Default for GameStateDefinition {
    fn default() -> Self {
        Self {
            turn: 1,
            active_player: PlayerRef::Player0,
            active_phase: Step::Main1,
            players: vec![PlayerStateDefinition::default(), PlayerStateDefinition::default()],
        }
    }
}

impl GameStateDefinition {
    /// Parse game state from lines in \[state\] section
    pub fn parse(lines: &[String]) -> Result<Self> {
        let mut state = GameStateDefinition::default();

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Split on = sign
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_lowercase();
                let value = value.trim();

                // Parse game-level fields
                if key == "turn" {
                    state.turn = value
                        .parse()
                        .map_err(|_| MtgError::ParseError(format!("Invalid turn value: {}", value)))?;
                    continue;
                }

                if key == "activeplayer" {
                    state.active_player = PlayerRef::parse(value)?;
                    continue;
                }

                if key == "activephase" {
                    state.active_phase = parse_phase(value)?;
                    continue;
                }

                // Parse player-specific fields
                if let Some(player_idx) = extract_player_prefix(&key) {
                    if player_idx >= state.players.len() {
                        return Err(MtgError::ParseError(format!("Invalid player index: {}", player_idx)));
                    }

                    let player = &mut state.players[player_idx];
                    let field = &key[2..]; // Skip "p0" or "p1" prefix

                    match field {
                        "life" => {
                            player.life = value
                                .parse()
                                .map_err(|_| MtgError::ParseError(format!("Invalid life value: {}", value)))?;
                        }
                        "landsplayed" => {
                            player.lands_played = value
                                .parse()
                                .map_err(|_| MtgError::ParseError(format!("Invalid lands played value: {}", value)))?;
                        }
                        "landsplayedlastturn" => {
                            player.lands_played_last_turn = value.parse().map_err(|_| {
                                MtgError::ParseError(format!("Invalid lands played last turn value: {}", value))
                            })?;
                        }
                        "counters" => {
                            player.counters = parse_player_counters(value)?;
                        }
                        "manapool" => {
                            player.mana_pool = value.split_whitespace().map(String::from).collect();
                        }
                        "persistentmana" => {
                            player.persistent_mana = value.split_whitespace().map(String::from).collect();
                        }
                        "hand" => {
                            player.hand = parse_card_list(value)?;
                        }
                        "battlefield" => {
                            player.battlefield = parse_card_list(value)?;
                        }
                        "graveyard" => {
                            player.graveyard = parse_card_list(value)?;
                        }
                        "library" => {
                            player.library = parse_card_list(value)?;
                        }
                        "exile" => {
                            player.exile = parse_card_list(value)?;
                        }
                        "command" => {
                            player.command = parse_card_list(value)?;
                        }
                        _ => {
                            // Unknown field, skip for forward compatibility
                        }
                    }
                }
            }
        }

        Ok(state)
    }
}

/// Extract player index from field like "p0life" or "p1hand"
fn extract_player_prefix(field: &str) -> Option<usize> {
    if field.len() >= 2 && field.starts_with('p') {
        let idx_char = field.chars().nth(1)?;
        idx_char.to_digit(10).map(|d| d as usize)
    } else {
        None
    }
}

/// Parse phase string to Step enum
fn parse_phase(s: &str) -> Result<Step> {
    // Map common phase names to Step enum
    match s.trim().to_uppercase().as_str() {
        "UNTAP" => Ok(Step::Untap),
        "UPKEEP" => Ok(Step::Upkeep),
        "DRAW" => Ok(Step::Draw),
        "MAIN1" | "PRECOMBAT" | "PRECOMBATMAIN" => Ok(Step::Main1),
        "COMBAT_BEGIN" | "BEGINNINGOFCOMBAT" => Ok(Step::BeginCombat),
        "COMBAT_DECLARE_ATTACKERS" | "DECLAREATTACKERS" => Ok(Step::DeclareAttackers),
        "COMBAT_DECLARE_BLOCKERS" | "DECLAREBLOCKERS" => Ok(Step::DeclareBlockers),
        "COMBAT_DAMAGE" | "COMBATDAMAGE" => Ok(Step::CombatDamage),
        "COMBAT_END" | "ENDOFCOMBAT" => Ok(Step::EndCombat),
        "MAIN2" | "POSTCOMBAT" | "POSTCOMBATMAIN" => Ok(Step::Main2),
        "END" | "ENDSTEP" | "END_OF_TURN" => Ok(Step::End),
        "CLEANUP" => Ok(Step::Cleanup),
        _ => Err(MtgError::ParseError(format!("Unknown phase: {}", s))),
    }
}

/// Parse player counters like "POISON=3,ENERGY=5"
fn parse_player_counters(s: &str) -> Result<HashMap<CounterType, i32>> {
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

/// Parse semicolon-separated list of cards
fn parse_card_list(s: &str) -> Result<Vec<CardDefinition>> {
    if s.is_empty() {
        return Ok(Vec::new());
    }

    s.split(';')
        .filter(|card| !card.trim().is_empty())
        .map(CardDefinition::parse)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_player_ref() {
        assert_eq!(PlayerRef::parse("p0").unwrap(), PlayerRef::Player0);
        assert_eq!(PlayerRef::parse("P1").unwrap(), PlayerRef::Player1);
        assert_eq!(PlayerRef::parse("human").unwrap(), PlayerRef::Player0);
        assert_eq!(PlayerRef::parse("AI").unwrap(), PlayerRef::Player1);
    }

    #[test]
    fn test_parse_phase() {
        assert!(matches!(parse_phase("UPKEEP").unwrap(), Step::Upkeep));
        assert!(matches!(parse_phase("Main1").unwrap(), Step::Main1));
        assert!(matches!(
            parse_phase("DeclareAttackers").unwrap(),
            Step::DeclareAttackers
        ));
    }

    #[test]
    fn test_parse_card_definition() {
        let card = CardDefinition::parse("Mountain|Tapped").unwrap();
        assert_eq!(card.name, "Mountain");
        assert!(card.modifiers.iter().any(|m| matches!(m, CardModifier::Tapped)));
    }

    #[test]
    fn test_parse_card_list() {
        let cards = parse_card_list("Mountain;Forest|Tapped;Island").unwrap();
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0].name, "Mountain");
        assert_eq!(cards[1].name, "Forest");
        assert_eq!(cards[2].name, "Island");
    }

    #[test]
    fn test_parse_game_state_basic() {
        let lines = vec![
            "turn=5".to_string(),
            "activeplayer=p0".to_string(),
            "activephase=MAIN1".to_string(),
            "p0life=15".to_string(),
            "p1life=18".to_string(),
        ];

        let state = GameStateDefinition::parse(&lines).unwrap();
        assert_eq!(state.turn, 5);
        assert_eq!(state.active_player, PlayerRef::Player0);
        assert_eq!(state.players[0].life, 15);
        assert_eq!(state.players[1].life, 18);
    }

    #[test]
    fn test_parse_game_state_with_zones() {
        let lines = vec![
            "turn=1".to_string(),
            "activeplayer=p0".to_string(),
            "activephase=UPKEEP".to_string(),
            "p0life=20".to_string(),
            "p0hand=Lightning Bolt;Mountain".to_string(),
            "p0battlefield=Forest|Tapped".to_string(),
            "p1life=20".to_string(),
        ];

        let state = GameStateDefinition::parse(&lines).unwrap();
        assert_eq!(state.players[0].hand.len(), 2);
        assert_eq!(state.players[0].hand[0].name, "Lightning Bolt");
        assert_eq!(state.players[0].battlefield.len(), 1);
        assert_eq!(state.players[0].battlefield[0].name, "Forest");
    }

    #[test]
    fn test_extract_player_prefix() {
        assert_eq!(extract_player_prefix("p0life"), Some(0));
        assert_eq!(extract_player_prefix("p1hand"), Some(1));
        assert_eq!(extract_player_prefix("turn"), None);
    }
}
