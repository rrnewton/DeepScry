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
    DamageAll,
    /// Multiple creatures deal damage to a single target
    /// Used by Allies at Last, Band Together, Tandem Takedown
    /// Parameters:
    ///   DefinedDamagers$ ParentTarget - damagers come from parent ability's targets
    ///   ValidTgts$ Creature - the target receiving damage
    ///   NumDmg$ Count$CardPower - damage equals each damager's power
    EachDamage,
    GainLife,
    LoseLife,
    SetLife,

    // === Card Draw & Mill ===
    Draw,
    DrawReplace,
    Mill,
    Discard,

    // === Mana ===
    Mana,
    ManaReflected,
    StoreMana,

    // === Creatures & Combat ===
    Pump,
    PumpAll,
    /// Remove keywords from a creature (inverse of Pump's keyword granting)
    /// Parameters:
    ///   Keywords$ - keywords to remove, separated by " & "
    ///   Defined$ Self - target is self (e.g., lose Defender)
    ///   ValidTgts$ Creature - target restriction for opponent's creatures
    ///   Duration$ Permanent - if present, keyword removal is permanent
    Debuff,
    UntilEOT,
    Animate,
    /// Mass animate: set base P/T and/or grant keywords to all matching permanents
    /// Parameters:
    ///   ValidCards$ - filter for which cards to animate (e.g., "Creature.YouCtrl")
    ///   Power$ / Toughness$ - base P/T to set (optional)
    ///   Keywords$ - keywords to grant, separated by " & " (optional)
    AnimateAll,
    BecomesCreature,

    // === Removal ===
    Destroy,
    DestroyAll,
    Sacrifice,
    SacrificeAll,
    Exile,
    ExileAll,

    // === Control ===
    /// Gain control of target permanent
    /// Parameters:
    ///   ValidTgts$ - target restriction (e.g., "Creature", "Artifact")
    ///   LoseControl$ - when control is lost (EOT, LeavesPlay, Untap)
    ///   Untap$ True - also untap the stolen permanent
    ///   AddKWs$ - keywords to grant (e.g., "Haste")
    GainControl,

    // === Combat ===
    /// Fight - two creatures deal damage equal to their power to each other (CR 701.12)
    ///   Defined$ - the creature that initiates the fight (Self, ParentTarget, etc.)
    ///   ValidTgts$ - target creature to fight against
    Fight,

    // === Tap/Untap ===
    Tap,
    TapAll,
    TapOrUntap,
    TapOrUntapAll,
    Untap,
    UntapAll,

    // === Tokens & Counters ===
    Token,

    // === Equipment & Auras ===
    /// Attach Equipment or Aura to target
    /// Example: DB$ Attach | ValidTgts$ Creature.YouCtrl
    /// Used by equipment ETB triggers like Twin Blades
    Attach,
    /// Create a token that's a copy of a permanent
    /// Parameters:
    ///   ValidTgts$ / Defined$ - target permanent to copy
    ///   NonLegendary$ True - remove Legendary supertype
    ///   SetPower$ N - override power
    ///   SetToughness$ N - override toughness
    ///   AddTypes$ Type1 & Type2 - add creature types
    ///   SetColor$ Color - override color
    ///   AddKeywords$ Keyword - add keywords
    ///   NumCopies$ N - create multiple copies
    CopyPermanent,
    PutCounter,
    PutCounterAll,
    RemoveCounter,
    MoveCounter,
    MultiplyCounter,
    /// Proliferate: choose any number of permanents and/or players with counters,
    /// then give each one additional counter of each kind already there (CR 701.34a)
    Proliferate,

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
    /// Take an extra turn after this one
    /// Example: "SP$ AddTurn | NumTurns$ 1" (Time Walk)
    AddTurn,
    /// Copy a spell on the stack
    /// Used by Jeong Jeong: "copy it and you may choose new targets"
    /// Parameters:
    ///   Defined$ - what to copy (TriggeredSpellAbility = the triggering spell)
    ///   MayChooseTarget$ - can choose new targets for the copy
    CopySpellAbility,

    /// Conditional sub-effect execution based on remembered cards
    /// Corresponds to: DB$ ImmediateTrigger | ConditionDefined$ Remembered | ConditionPresent$ Card.nonLand | Execute$ SVar
    /// Used by Teo to put counter when discarding nonland
    ImmediateTrigger,

    /// Clear remembered cards storage
    /// Corresponds to: DB$ Cleanup | ClearRemembered$ True
    Cleanup,

    // === Information ===
    Scry,
    Surveil,

    // === Protection & Regeneration ===
    Protection,
    ProtectionAll,
    /// Regenerate: Create a regeneration shield on target permanent (CR 701.15)
    /// Example: AB$ Regenerate | Cost$ B | SpellDescription$ Regenerate CARDNAME.
    Regenerate,

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

    // === Modal Spells ===
    /// Charm: Modal spell with multiple choices (e.g., "Choose one —")
    /// Parameters:
    ///   Choices$ - comma-separated SVar references (e.g., "DBDestroy,DBDraw")
    ///   CharmNum$ - number of modes to choose (default "1")
    ///   MinCharmNum$ - minimum modes required (default = CharmNum$)
    ///   CanRepeatModes$ - if present, same mode can be chosen twice
    /// Example: A:SP$ Charm | Choices$ Destroy,Remove
    Charm,

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
            "DamageAll" => Self::DamageAll,
            "EachDamage" => Self::EachDamage,
            "GainLife" => Self::GainLife,
            "LoseLife" => Self::LoseLife,
            "SetLife" => Self::SetLife,

            // Card Draw & Mill
            "Draw" => Self::Draw,
            "DrawReplace" => Self::DrawReplace,
            "Mill" => Self::Mill,
            "Discard" => Self::Discard,
            "Dig" => Self::DigMultiple, // Note: "Dig" maps to DigMultiple in Java

            // Mana
            "Mana" => Self::Mana,
            "ManaReflected" => Self::ManaReflected,
            "StoreMana" => Self::StoreMana,

            // Creatures & Combat
            "Pump" => Self::Pump,
            "PumpAll" => Self::PumpAll,
            "Debuff" => Self::Debuff,
            "UntilEOT" => Self::UntilEOT,
            "Animate" => Self::Animate,
            "AnimateAll" => Self::AnimateAll,
            "BecomesCreature" => Self::BecomesCreature,

            // Removal
            "Destroy" => Self::Destroy,
            "DestroyAll" => Self::DestroyAll,
            "Sacrifice" => Self::Sacrifice,
            "SacrificeAll" => Self::SacrificeAll,
            "Exile" => Self::Exile,
            "ExileAll" => Self::ExileAll,

            // Control
            "GainControl" => Self::GainControl,

            // Combat
            "Fight" => Self::Fight,

            // Tap/Untap
            "Tap" => Self::Tap,
            "TapAll" => Self::TapAll,
            "TapOrUntap" => Self::TapOrUntap,
            "TapOrUntapAll" => Self::TapOrUntapAll,
            "Untap" => Self::Untap,
            "UntapAll" => Self::UntapAll,

            // Tokens & Counters
            "Token" => Self::Token,
            "Attach" => Self::Attach,
            "CopyPermanent" => Self::CopyPermanent,
            "PutCounter" => Self::PutCounter,
            "PutCounterAll" => Self::PutCounterAll,
            "RemoveCounter" => Self::RemoveCounter,
            "MoveCounter" => Self::MoveCounter,
            "MultiplyCounter" => Self::MultiplyCounter,
            "Proliferate" => Self::Proliferate,

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
            "AddTurn" => Self::AddTurn,
            "CopySpellAbility" => Self::CopySpellAbility,
            "ImmediateTrigger" => Self::ImmediateTrigger,
            "Cleanup" => Self::Cleanup,

            // Information
            "Scry" => Self::Scry,
            "Surveil" => Self::Surveil,

            // Protection & Regeneration
            "Protection" => Self::Protection,
            "ProtectionAll" => Self::ProtectionAll,
            "Regenerate" => Self::Regenerate,

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

            // Modal Spells
            "Charm" => Self::Charm,

            // Unknown type - preserve original string for debugging
            unknown => Self::Unknown(unknown.to_string()),
        }
    }

    /// Convert back to string (for debugging/logging)
    pub fn as_str(&self) -> &str {
        match self {
            Self::DealDamage => "DealDamage",
            Self::DamageAll => "DamageAll",
            Self::EachDamage => "EachDamage",
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
            Self::Debuff => "Debuff",
            Self::UntilEOT => "UntilEOT",
            Self::Animate => "Animate",
            Self::AnimateAll => "AnimateAll",
            Self::BecomesCreature => "BecomesCreature",
            Self::Destroy => "Destroy",
            Self::DestroyAll => "DestroyAll",
            Self::Sacrifice => "Sacrifice",
            Self::SacrificeAll => "SacrificeAll",
            Self::Exile => "Exile",
            Self::ExileAll => "ExileAll",
            Self::GainControl => "GainControl",
            Self::Fight => "Fight",
            Self::Tap => "Tap",
            Self::TapAll => "TapAll",
            Self::TapOrUntap => "TapOrUntap",
            Self::TapOrUntapAll => "TapOrUntapAll",
            Self::Untap => "Untap",
            Self::UntapAll => "UntapAll",
            Self::Token => "Token",
            Self::Attach => "Attach",
            Self::CopyPermanent => "CopyPermanent",
            Self::PutCounter => "PutCounter",
            Self::PutCounterAll => "PutCounterAll",
            Self::RemoveCounter => "RemoveCounter",
            Self::MoveCounter => "MoveCounter",
            Self::MultiplyCounter => "MultiplyCounter",
            Self::Proliferate => "Proliferate",
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
            Self::AddTurn => "AddTurn",
            Self::CopySpellAbility => "CopySpellAbility",
            Self::Scry => "Scry",
            Self::Surveil => "Surveil",
            Self::Protection => "Protection",
            Self::ProtectionAll => "ProtectionAll",
            Self::Regenerate => "Regenerate",
            Self::Clash => "Clash",
            Self::Planeswalk => "Planeswalk",
            Self::RollDice => "RollDice",
            Self::FlipACoin => "FlipACoin",
            Self::Balance => "Balance",
            Self::Airbend => "Airbend",
            Self::Earthbend => "Earthbend",
            Self::Charm => "Charm",
            Self::Discard => "Discard",
            Self::ImmediateTrigger => "ImmediateTrigger",
            Self::Cleanup => "Cleanup",
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
    ///
    /// # Errors
    ///
    /// Returns `AbilityParseError::MissingParameter` if key not found,
    /// or `AbilityParseError::InvalidParameter` if value cannot be parsed.
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
    ///
    /// # Errors
    ///
    /// Returns `AbilityParseError::MissingParameter` if key not found,
    /// or `AbilityParseError::InvalidParameter` if value cannot be parsed.
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

    /// Parse an SVar body (without prefix) and return the params if successful
    ///
    /// This is a convenience function for parsing SVar bodies like:
    /// `"DB$ Draw | NumCards$ 1"` instead of needing `"A:DB$ Draw | NumCards$ 1"`
    ///
    /// # Example
    /// ```ignore
    /// // OLD (hacky): body.contains("DB$ Draw")
    /// // NEW (safe):
    /// if let Some(params) = AbilityParams::parse_svar_body(body) {
    ///     if params.api_type == ApiType::Draw { ... }
    /// }
    /// ```
    pub fn parse_svar_body(body: &str) -> Option<Self> {
        // Add a dummy prefix since parse() expects "PREFIX:BODY" format
        let prefixed = format!("A:{}", body);
        Self::parse(&prefixed).ok()
    }

    /// Check if an SVar body has a specific API type
    ///
    /// This is the safe replacement for `body.contains("DB$ Draw")` patterns.
    ///
    /// # Example
    /// ```ignore
    /// // OLD (hacky): body.contains("DB$ Draw")
    /// // NEW (safe): AbilityParams::is_api_type(body, ApiType::Draw)
    /// ```
    pub fn is_api_type(body: &str, expected: ApiType) -> bool {
        Self::parse_svar_body(body)
            .map(|params| params.api_type == expected)
            .unwrap_or(false)
    }

    /// Check if an SVar body has any of the specified API types
    ///
    /// # Example
    /// ```ignore
    /// // OLD (hacky): body.contains("AB$ Draw") || body.contains("DB$ Draw")
    /// // NEW (safe): AbilityParams::is_any_api_type(body, &[ApiType::Draw])
    /// ```
    pub fn is_any_api_type(body: &str, expected: &[ApiType]) -> bool {
        Self::parse_svar_body(body)
            .map(|params| expected.contains(&params.api_type))
            .unwrap_or(false)
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

    #[test]
    fn test_parse_svar_body() {
        // Test parsing SVar body without prefix
        let body = "DB$ Draw | NumCards$ 1";
        let params = AbilityParams::parse_svar_body(body).unwrap();

        assert_eq!(params.api_type, ApiType::Draw);
        assert_eq!(params.get("NumCards"), Some("1"));
    }

    #[test]
    fn test_is_api_type() {
        // Test the is_api_type helper
        let body = "DB$ Draw | NumCards$ 1";
        assert!(AbilityParams::is_api_type(body, ApiType::Draw));
        assert!(!AbilityParams::is_api_type(body, ApiType::DealDamage));

        // Test with different ability types
        let mana_body = "AB$ Mana | Produced$ R | Amount$ 1";
        assert!(AbilityParams::is_api_type(mana_body, ApiType::Mana));
        assert!(!AbilityParams::is_api_type(mana_body, ApiType::Draw));
    }

    #[test]
    fn test_is_any_api_type() {
        // Test checking multiple API types at once
        let body = "DB$ Draw | NumCards$ 1";

        // Should match Draw
        assert!(AbilityParams::is_any_api_type(body, &[ApiType::Draw, ApiType::Mill]));

        // Should not match
        assert!(!AbilityParams::is_any_api_type(
            body,
            &[ApiType::DealDamage, ApiType::Mana]
        ));
    }

    #[test]
    fn test_is_api_type_replaces_contains() {
        // Demonstrate that is_api_type is safer than .contains()
        let body = "DB$ Draw | NumCards$ 1";

        // OLD (hacky - would have false positives):
        // body.contains("AB$ Draw") || body.contains("DB$ Draw")

        // NEW (safe - tokenized parsing):
        assert!(AbilityParams::is_api_type(body, ApiType::Draw));

        // This body has "Draw" as part of a different context - wouldn't match
        let misleading_body = "DB$ PutCounter | CounterType$ Drawing";
        assert!(!AbilityParams::is_api_type(misleading_body, ApiType::Draw));
    }
}
