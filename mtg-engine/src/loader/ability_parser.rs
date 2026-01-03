//! Ability script parser for MTG Forge card format
//!
//! This module provides safe, tokenized parsing of ability scripts from card files.
//! Replaces unsafe substring matching with proper parameter extraction and validation.
//!
//! # Format
//!
//! Card abilities use pipe-delimited key-value pairs:
//! ```text
//! A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ Deal 3 damage
//! ```
//!
//! Structure:
//! - Prefix: `A:`, `S:`, `T:` (Activated, Static, Triggered)
//! - Record type: `SP$`, `AB$`, `DB$`, `ST$` (Spell, Ability, Database ref, Static)
//! - Parameters: `Key$ Value` separated by `|`
//!
//! # Safety
//!
//! This parser addresses safety concerns identified in ai_docs/ability_parsing_comparison.md:
//! 1. Tokenization: Splits by `|` before matching to avoid substring false positives
//! 2. Validation: Returns Result types with clear error messages
//! 3. Type safety: Uses enums for API types instead of raw strings
//! 4. Performance: Parse once, query many times (O(n + k) instead of O(nk))

use std::collections::HashMap;

/// Ability record type (indicates how the ability functions)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityRecordType {
    /// Spell ability (SP$) - cast from hand/stack
    Spell,
    /// Activated ability (AB$) - activated from battlefield
    Ability,
    /// Sub-ability reference (DB$) - references an SVar
    SubAbility,
    /// Static ability (ST$) - continuous effect
    StaticAbility,
}

impl AbilityRecordType {
    /// Get the prefix string for this record type (for lookups)
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Spell => "SP",
            Self::Ability => "AB",
            Self::SubAbility => "DB",
            Self::StaticAbility => "ST",
        }
    }
}

/// API type - what the ability does
///
/// This enum lists all recognized ability types from Java Forge's ApiType enum.
/// Based on forge-java/forge-game/src/main/java/forge/game/ability/ApiType.java
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ApiType {
    // === Damage & Life ===
    DealDamage,
    GainLife,
    LoseLife,
    SetLife,

    // === Card Draw & Mill ===
    Draw,
    DrawReplace,
    Mill,

    // === Mana ===
    Mana,
    ManaReflected,
    StoreMana,

    // === Creatures & Combat ===
    Pump,
    PumpAll,
    UntilEOT,
    Animate,
    BecomesCreature,

    // === Removal ===
    Destroy,
    DestroyAll,
    Sacrifice,
    SacrificeAll,
    Exile,
    ExileAll,

    // === Tap/Untap ===
    Tap,
    TapAll,
    TapOrUntap,
    TapOrUntapAll,
    Untap,
    UntapAll,

    // === Tokens & Counters ===
    Token,
    PutCounter,
    RemoveCounter,
    MoveCounter,
    MultiplyCounter,

    // === Zone Changes ===
    ChangeZone,
    ChangeZoneAll,

    // === Stack Interaction ===
    Counter,
    CounterAll,

    // === Search & Reveal ===
    DigMultiple,
    Reveal,
    RevealHand,

    // === Targeting ===
    ChooseCard,
    ChoosePlayer,
    ChooseType,
    ChooseColor,
    ChooseDirection,
    ChooseNumber,
    ChooseSource,

    // === Game Actions ===
    Play,
    PlayLandVariant,
    Effect,
    DelayedTrigger,

    // === Information ===
    Scry,
    Surveil,

    // === Protection ===
    Protection,
    ProtectionAll,

    // === Special ===
    Clash,
    Planeswalk,
    RollDice,
    FlipACoin,

    // === Balance/Equalize Effects ===
    Balance,

    // === Avatar Set Mechanics ===
    /// Airbend: Exile target, owner may cast it for {2} from exile.
    /// CR 701.65b
    Airbend,

    /// Earthbend: Target land becomes 0/0 creature with haste, put N +1/+1 counters.
    /// When it dies or is exiled, return it to battlefield tapped.
    /// CR 701.65a
    Earthbend,

    // === Catch-all for unknown types ===
    Unknown(String),
}

impl ApiType {
    /// Parse an API type string into an enum variant
    ///
    /// Note: Not implementing FromStr trait because this never fails (returns Unknown for unrecognized types)
    pub fn parse(s: &str) -> Self {
        match s {
            // Damage & Life
            "DealDamage" => Self::DealDamage,
            "GainLife" => Self::GainLife,
            "LoseLife" => Self::LoseLife,
            "SetLife" => Self::SetLife,

            // Card Draw & Mill
            "Draw" => Self::Draw,
            "DrawReplace" => Self::DrawReplace,
            "Mill" => Self::Mill,
            "Dig" => Self::DigMultiple, // Note: "Dig" maps to DigMultiple in Java

            // Mana
            "Mana" => Self::Mana,
            "ManaReflected" => Self::ManaReflected,
            "StoreMana" => Self::StoreMana,

            // Creatures & Combat
            "Pump" => Self::Pump,
            "PumpAll" => Self::PumpAll,
            "UntilEOT" => Self::UntilEOT,
            "Animate" => Self::Animate,
            "BecomesCreature" => Self::BecomesCreature,

            // Removal
            "Destroy" => Self::Destroy,
            "DestroyAll" => Self::DestroyAll,
            "Sacrifice" => Self::Sacrifice,
            "SacrificeAll" => Self::SacrificeAll,
            "Exile" => Self::Exile,
            "ExileAll" => Self::ExileAll,

            // Tap/Untap
            "Tap" => Self::Tap,
            "TapAll" => Self::TapAll,
            "TapOrUntap" => Self::TapOrUntap,
            "TapOrUntapAll" => Self::TapOrUntapAll,
            "Untap" => Self::Untap,
            "UntapAll" => Self::UntapAll,

            // Tokens & Counters
            "Token" => Self::Token,
            "PutCounter" => Self::PutCounter,
            "RemoveCounter" => Self::RemoveCounter,
            "MoveCounter" => Self::MoveCounter,
            "MultiplyCounter" => Self::MultiplyCounter,

            // Zone Changes
            "ChangeZone" => Self::ChangeZone,
            "ChangeZoneAll" => Self::ChangeZoneAll,

            // Stack Interaction
            "Counter" => Self::Counter,
            "CounterAll" => Self::CounterAll,

            // Search & Reveal
            "DigMultiple" => Self::DigMultiple,
            "Reveal" => Self::Reveal,
            "RevealHand" => Self::RevealHand,

            // Targeting
            "ChooseCard" => Self::ChooseCard,
            "ChoosePlayer" => Self::ChoosePlayer,
            "ChooseType" => Self::ChooseType,
            "ChooseColor" => Self::ChooseColor,
            "ChooseDirection" => Self::ChooseDirection,
            "ChooseNumber" => Self::ChooseNumber,
            "ChooseSource" => Self::ChooseSource,

            // Game Actions
            "Play" => Self::Play,
            "PlayLandVariant" => Self::PlayLandVariant,
            "Effect" => Self::Effect,
            "DelayedTrigger" => Self::DelayedTrigger,

            // Information
            "Scry" => Self::Scry,
            "Surveil" => Self::Surveil,

            // Protection
            "Protection" => Self::Protection,
            "ProtectionAll" => Self::ProtectionAll,

            // Special
            "Clash" => Self::Clash,
            "Planeswalk" => Self::Planeswalk,
            "RollDice" => Self::RollDice,
            "FlipACoin" => Self::FlipACoin,

            // Balance/Equalize
            "Balance" => Self::Balance,

            // Avatar Set Mechanics
            "Airbend" => Self::Airbend,
            "Earthbend" => Self::Earthbend,

            // Unknown type - preserve original string for debugging
            unknown => Self::Unknown(unknown.to_string()),
        }
    }

    /// Convert back to string (for debugging/logging)
    pub fn as_str(&self) -> &str {
        match self {
            Self::DealDamage => "DealDamage",
            Self::GainLife => "GainLife",
            Self::LoseLife => "LoseLife",
            Self::SetLife => "SetLife",
            Self::Draw => "Draw",
            Self::DrawReplace => "DrawReplace",
            Self::Mill => "Mill",
            Self::Mana => "Mana",
            Self::ManaReflected => "ManaReflected",
            Self::StoreMana => "StoreMana",
            Self::Pump => "Pump",
            Self::PumpAll => "PumpAll",
            Self::UntilEOT => "UntilEOT",
            Self::Animate => "Animate",
            Self::BecomesCreature => "BecomesCreature",
            Self::Destroy => "Destroy",
            Self::DestroyAll => "DestroyAll",
            Self::Sacrifice => "Sacrifice",
            Self::SacrificeAll => "SacrificeAll",
            Self::Exile => "Exile",
            Self::ExileAll => "ExileAll",
            Self::Tap => "Tap",
            Self::TapAll => "TapAll",
            Self::TapOrUntap => "TapOrUntap",
            Self::TapOrUntapAll => "TapOrUntapAll",
            Self::Untap => "Untap",
            Self::UntapAll => "UntapAll",
            Self::Token => "Token",
            Self::PutCounter => "PutCounter",
            Self::RemoveCounter => "RemoveCounter",
            Self::MoveCounter => "MoveCounter",
            Self::MultiplyCounter => "MultiplyCounter",
            Self::ChangeZone => "ChangeZone",
            Self::ChangeZoneAll => "ChangeZoneAll",
            Self::Counter => "Counter",
            Self::CounterAll => "CounterAll",
            Self::DigMultiple => "DigMultiple",
            Self::Reveal => "Reveal",
            Self::RevealHand => "RevealHand",
            Self::ChooseCard => "ChooseCard",
            Self::ChoosePlayer => "ChoosePlayer",
            Self::ChooseType => "ChooseType",
            Self::ChooseColor => "ChooseColor",
            Self::ChooseDirection => "ChooseDirection",
            Self::ChooseNumber => "ChooseNumber",
            Self::ChooseSource => "ChooseSource",
            Self::Play => "Play",
            Self::PlayLandVariant => "PlayLandVariant",
            Self::Effect => "Effect",
            Self::DelayedTrigger => "DelayedTrigger",
            Self::Scry => "Scry",
            Self::Surveil => "Surveil",
            Self::Protection => "Protection",
            Self::ProtectionAll => "ProtectionAll",
            Self::Clash => "Clash",
            Self::Planeswalk => "Planeswalk",
            Self::RollDice => "RollDice",
            Self::FlipACoin => "FlipACoin",
            Self::Balance => "Balance",
            Self::Airbend => "Airbend",
            Self::Earthbend => "Earthbend",
            Self::Unknown(s) => s.as_str(),
        }
    }
}

/// Error types for ability parsing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbilityParseError {
    /// Ability string missing ':' prefix separator (expected "A:" or "T:" or "S:")
    MissingPrefix,

    /// No record type found (expected AB$, SP$, DB$, or ST$)
    MissingRecordType,

    /// Unknown API type encountered
    UnknownApiType(String),

    /// Missing required parameter for this API type
    MissingParameter { api_type: String, parameter: String },

    /// Invalid value for a parameter
    InvalidParameter {
        parameter: String,
        value: String,
        expected: String,
    },
}

impl std::fmt::Display for AbilityParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPrefix => write!(f, "Ability string missing ':' prefix separator"),
            Self::MissingRecordType => write!(f, "No record type found (expected AB$, SP$, DB$, or ST$)"),
            Self::UnknownApiType(s) => write!(f, "Unknown API type: {}", s),
            Self::MissingParameter { api_type, parameter } => {
                write!(
                    f,
                    "Missing required parameter '{}' for API type '{}'",
                    parameter, api_type
                )
            }
            Self::InvalidParameter {
                parameter,
                value,
                expected,
            } => {
                write!(
                    f,
                    "Invalid value '{}' for parameter '{}' (expected {})",
                    value, parameter, expected
                )
            }
        }
    }
}

impl std::error::Error for AbilityParseError {}

/// Parsed ability parameters (tokenized and structured)
///
/// This is the core data structure that replaces unsafe `contains()` checks.
/// Equivalent to Java Forge's FileSection.parseToMap()
#[derive(Debug, Clone)]
pub struct AbilityParams {
    /// Record type prefix (A:, S:, T:)
    pub prefix: String,

    /// Record type (SP, AB, DB, ST)
    pub record_type: AbilityRecordType,

    /// API type (what the ability does)
    pub api_type: ApiType,

    /// All parameters as key-value pairs
    params: HashMap<String, String>,
}

impl AbilityParams {
    /// Parse an ability string into structured parameters
    ///
    /// # Format
    ///
    /// ```text
    /// PREFIX:RECORD$ APIType | Param1$ Value1 | Param2$ Value2 | ...
    /// ```
    ///
    /// # Example
    ///
    /// ```text
    /// A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ Deal 3 damage
    /// ```
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Missing prefix (`:`)
    /// - Missing record type (`SP$`, `AB$`, etc.)
    /// - Unknown API type
    pub fn parse(ability: &str) -> Result<Self, AbilityParseError> {
        // Split by prefix separator
        let (prefix, body) = ability.split_once(':').ok_or(AbilityParseError::MissingPrefix)?;

        let prefix = prefix.trim().to_string();

        // Parse parameters by splitting on |
        let mut params = HashMap::new();

        for param in body.split('|') {
            let param = param.trim();
            if param.is_empty() {
                continue;
            }

            // Split by $ separator
            if let Some((key, value)) = param.split_once('$') {
                params.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        // Determine record type
        let record_type = if params.contains_key("AB") {
            AbilityRecordType::Ability
        } else if params.contains_key("SP") {
            AbilityRecordType::Spell
        } else if params.contains_key("DB") {
            AbilityRecordType::SubAbility
        } else if params.contains_key("ST") {
            AbilityRecordType::StaticAbility
        } else {
            return Err(AbilityParseError::MissingRecordType);
        };

        // Get API type string
        let api_type_str = params
            .get(record_type.prefix())
            .ok_or(AbilityParseError::MissingRecordType)?;

        // Parse API type
        let api_type = ApiType::parse(api_type_str);

        Ok(Self {
            prefix,
            record_type,
            api_type,
            params,
        })
    }

    /// Get a parameter value by key
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Check if a parameter exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.params.contains_key(key)
    }

    /// Get a parameter and parse it as an integer
    pub fn get_i32(&self, key: &str) -> Result<i32, AbilityParseError> {
        let value = self.get(key).ok_or_else(|| AbilityParseError::MissingParameter {
            api_type: self.api_type.as_str().to_string(),
            parameter: key.to_string(),
        })?;

        value.parse::<i32>().map_err(|_| AbilityParseError::InvalidParameter {
            parameter: key.to_string(),
            value: value.to_string(),
            expected: "integer".to_string(),
        })
    }

    /// Get a parameter and parse it as an unsigned integer
    pub fn get_u8(&self, key: &str) -> Result<u8, AbilityParseError> {
        let value = self.get(key).ok_or_else(|| AbilityParseError::MissingParameter {
            api_type: self.api_type.as_str().to_string(),
            parameter: key.to_string(),
        })?;

        value.parse::<u8>().map_err(|_| AbilityParseError::InvalidParameter {
            parameter: key.to_string(),
            value: value.to_string(),
            expected: "unsigned integer".to_string(),
        })
    }

    /// Get a parameter with a default value if missing
    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    /// Get all parameter keys (for debugging)
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.params.keys().map(|s| s.as_str())
    }

    /// Iterate over all parameters
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_damage_spell() {
        let ability = "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ Deal 3 damage";
        let params = AbilityParams::parse(ability).unwrap();

        assert_eq!(params.prefix, "A");
        assert_eq!(params.record_type, AbilityRecordType::Spell);
        assert_eq!(params.api_type, ApiType::DealDamage);
        assert_eq!(params.get("ValidTgts"), Some("Any"));
        assert_eq!(params.get_i32("NumDmg").unwrap(), 3);
    }

    #[test]
    fn test_parse_mana_ability() {
        let ability = "A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}";
        let params = AbilityParams::parse(ability).unwrap();

        assert_eq!(params.prefix, "A");
        assert_eq!(params.record_type, AbilityRecordType::Ability);
        assert_eq!(params.api_type, ApiType::Mana);
        assert_eq!(params.get("Cost"), Some("T"));
        assert_eq!(params.get("Produced"), Some("G"));
    }

    #[test]
    fn test_parse_draw_effect() {
        let ability = "A:SP$ Draw | NumCards$ 3 | ValidTgts$ Player | SpellDescription$ Target player draws 3 cards";
        let params = AbilityParams::parse(ability).unwrap();

        assert_eq!(params.api_type, ApiType::Draw);
        assert_eq!(params.get_u8("NumCards").unwrap(), 3);
    }

    #[test]
    fn test_missing_prefix_error() {
        let ability = "SP$ DealDamage | NumDmg$ 3";
        let result = AbilityParams::parse(ability);

        assert!(matches!(result, Err(AbilityParseError::MissingPrefix)));
    }

    #[test]
    fn test_missing_record_type_error() {
        let ability = "A:DealDamage | NumDmg$ 3"; // No SP$ or AB$
        let result = AbilityParams::parse(ability);

        assert!(matches!(result, Err(AbilityParseError::MissingRecordType)));
    }

    #[test]
    fn test_no_false_positive_substring_match() {
        // "Madden" should NOT match "add" when using tokenized parsing
        let ability = "A:SP$ Madden | Cost$ T";
        let params = AbilityParams::parse(ability).unwrap();

        // API type should be Unknown("Madden"), not parsed as "add"
        assert!(matches!(params.api_type, ApiType::Unknown(_)));
        if let ApiType::Unknown(ref s) = params.api_type {
            assert_eq!(s, "Madden");
        }
    }

    #[test]
    fn test_tokenized_damage_vs_deal_damage() {
        // "Damage" should NOT match "DealDamage" - they're separate API types
        let ability1 = "A:SP$ DealDamage | NumDmg$ 3";
        let ability2 = "A:SP$ PreventDamage | Amount$ 5";

        let params1 = AbilityParams::parse(ability1).unwrap();
        let params2 = AbilityParams::parse(ability2).unwrap();

        assert_eq!(params1.api_type, ApiType::DealDamage);
        assert!(matches!(params2.api_type, ApiType::Unknown(_))); // PreventDamage not in enum yet
    }
}
