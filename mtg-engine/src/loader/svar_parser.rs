//! SVar (Script Variable) parsing for Java Forge card format.
//!
//! SVars are a core mechanism in Java Forge for:
//! 1. Defining reusable ability definitions (DB$, AB$ blocks)
//! 2. Defining static abilities (Mode$ CantBlockBy, Mode$ Continuous, etc.)
//! 3. Holding computed values (X variables, counters, etc.)
//! 4. Chaining SubAbility effects
//!
//! # Format
//!
//! SVar lines have the format: `SVar:NAME:body`
//!
//! Where body can be:
//! - `DB$ ApiType | Param$ Value | ...` - Delayed/triggered effect
//! - `AB$ ApiType | Param$ Value | ...` - Activated ability
//! - `Mode$ StaticType | Param$ Value | ...` - Static ability definition
//! - `TRUE` / `FALSE` - Boolean flag
//! - `Count$...` or `Sacrificed$...` - Computed value expression
//!
//! # Examples
//!
//! ```text
//! SVar:Unblockable:Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered
//! SVar:TrigDraw:DB$ Draw | NumCards$ 1
//! SVar:MayPlay:Mode$ Continuous | Affected$ Card.IsRemembered | MayPlay$ True
//! SVar:X:Sacrificed$CardPower
//! SVar:HasAttackEffect:TRUE
//! ```

use super::ability_parser::{AbilityParams, ApiType};
use std::collections::HashMap;

/// A parsed SVar definition.
///
/// SVars can represent different things depending on their content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSVar {
    /// A static ability definition (Mode$ ...)
    /// Examples: CantBlockBy, Continuous, Attacks, ChangesZone
    StaticAbility(StaticAbilityDef),

    /// An effect definition (DB$ ... or AB$ ...)
    /// Can be referenced by Execute$ or SubAbility$
    Effect(EffectDef),

    /// A boolean flag (TRUE, FALSE)
    BooleanFlag(bool),

    /// A computed value expression (Count$, Sacrificed$, etc.)
    ComputedValue(ComputedValueExpr),

    /// Raw/unparsed content (for patterns we don't yet support)
    Raw(String),
}

/// A static ability definition parsed from Mode$ SVar.
///
/// Static abilities apply continuous effects to the game state
/// without requiring activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticAbilityDef {
    /// The mode/type of static ability
    pub mode: StaticAbilityMode,

    /// All parameters from the definition
    pub params: HashMap<String, String>,

    /// Description text (for display)
    pub description: Option<String>,
}

/// Types of static abilities (from Mode$ parameter).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StaticAbilityMode {
    /// Mode$ CantBlockBy - Prevents certain creatures from blocking
    /// Params: ValidAttacker$, ValidBlocker$, etc.
    CantBlockBy,

    /// Mode$ CantAttack - Prevents creatures from attacking
    CantAttack,

    /// Mode$ CantBlock - Prevents creatures from blocking
    CantBlock,

    /// Mode$ Continuous - General continuous effect
    /// Params: Affected$, AffectedZone$, MayPlay$, etc.
    Continuous,

    /// Mode$ Attacks - Triggers when a creature attacks
    Attacks,

    /// Mode$ ChangesZone - Triggers on zone changes (ETB, LTB, etc.)
    ChangesZone,

    /// Mode$ Phase - Triggers at specific phases
    Phase,

    /// Mode$ SpellCast - Triggers when spells are cast
    SpellCast,

    /// Mode$ LandPlayed - Triggers when lands are played
    LandPlayed,

    /// Mode$ Sacrificed - Triggers when permanents are sacrificed
    Sacrificed,

    /// Mode$ MustAttack - the affected creature attacks each combat if able
    /// (CR 508.1a). Juggernaut: `Mode$ MustAttack | ValidCreature$ Card.Self`.
    MustAttack,

    /// Unknown mode (for forward compatibility)
    Unknown(String),
}

impl StaticAbilityMode {
    /// Parse a Mode$ value into a StaticAbilityMode.
    pub fn parse(s: &str) -> Self {
        match s {
            "CantBlockBy" => Self::CantBlockBy,
            "CantAttack" => Self::CantAttack,
            "CantBlock" => Self::CantBlock,
            "Continuous" => Self::Continuous,
            "Attacks" => Self::Attacks,
            "ChangesZone" => Self::ChangesZone,
            "Phase" => Self::Phase,
            "SpellCast" => Self::SpellCast,
            "LandPlayed" => Self::LandPlayed,
            "Sacrificed" => Self::Sacrificed,
            "MustAttack" => Self::MustAttack,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &str {
        match self {
            Self::CantBlockBy => "CantBlockBy",
            Self::CantAttack => "CantAttack",
            Self::CantBlock => "CantBlock",
            Self::Continuous => "Continuous",
            Self::Attacks => "Attacks",
            Self::ChangesZone => "ChangesZone",
            Self::Phase => "Phase",
            Self::SpellCast => "SpellCast",
            Self::LandPlayed => "LandPlayed",
            Self::Sacrificed => "Sacrificed",
            Self::MustAttack => "MustAttack",
            Self::Unknown(s) => s,
        }
    }
}

/// An effect definition parsed from DB$ or AB$ SVar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDef {
    /// Whether this is AB$ (activated) or DB$ (delayed/triggered)
    pub is_activated: bool,

    /// The API type (Draw, DealDamage, Pump, etc.)
    pub api_type: ApiType,

    /// All parameters from the definition
    pub params: HashMap<String, String>,

    /// SubAbility$ reference (if any)
    pub sub_ability: Option<String>,
}

/// A computed value expression.
///
/// These are used for dynamic values like X costs, power references, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputedValueExpr {
    /// Sacrificed$CardPower - Power of sacrificed card
    SacrificedCardPower,

    /// Sacrificed$CardToughness - Toughness of sacrificed card
    SacrificedCardToughness,

    /// Count$... - Count of matching objects
    Count { expression: String },

    /// Compare expression - conditional value
    Compare { expression: String },

    /// Raw expression we don't parse
    Raw(String),
}

/// Parse a single SVar body into a ParsedSVar.
///
/// # Arguments
///
/// * `body` - The SVar body (everything after `SVar:NAME:`)
///
/// # Returns
///
/// A parsed SVar representation.
pub fn parse_svar(body: &str) -> ParsedSVar {
    let body = body.trim();

    // Check for boolean flags
    if body.eq_ignore_ascii_case("TRUE") {
        return ParsedSVar::BooleanFlag(true);
    }
    if body.eq_ignore_ascii_case("FALSE") {
        return ParsedSVar::BooleanFlag(false);
    }

    // Check for computed value expressions
    if body.starts_with("Sacrificed$") {
        return parse_sacrificed_expr(body);
    }
    if body.starts_with("Count$") {
        return ParsedSVar::ComputedValue(ComputedValueExpr::Count {
            expression: body.to_string(),
        });
    }

    // Check for static ability (Mode$ ...)
    if body.starts_with("Mode$") {
        return parse_static_ability(body);
    }

    // Check for effect definitions (DB$ or AB$)
    if body.starts_with("DB$") || body.starts_with("AB$") {
        return parse_effect_def(body);
    }

    // Unknown format - store as raw
    ParsedSVar::Raw(body.to_string())
}

/// Parse a Sacrificed$ expression.
fn parse_sacrificed_expr(body: &str) -> ParsedSVar {
    let expr = body.strip_prefix("Sacrificed$").unwrap_or(body);
    let computed = match expr {
        "CardPower" => ComputedValueExpr::SacrificedCardPower,
        "CardToughness" => ComputedValueExpr::SacrificedCardToughness,
        _ => ComputedValueExpr::Raw(body.to_string()),
    };
    ParsedSVar::ComputedValue(computed)
}

/// Parse a static ability definition (Mode$ ...).
fn parse_static_ability(body: &str) -> ParsedSVar {
    let mut params = HashMap::new();
    let mut mode = StaticAbilityMode::Unknown("".to_string());
    let mut description = None;

    // Parse pipe-separated key-value pairs
    for part in body.split('|') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('$') {
            let key = key.trim();
            let value = value.trim();

            if key == "Mode" {
                mode = StaticAbilityMode::parse(value);
            } else if key == "Description" {
                description = Some(value.to_string());
            } else {
                params.insert(key.to_string(), value.to_string());
            }
        }
    }

    ParsedSVar::StaticAbility(StaticAbilityDef {
        mode,
        params,
        description,
    })
}

/// Parse an effect definition (DB$ or AB$).
fn parse_effect_def(body: &str) -> ParsedSVar {
    let is_activated = body.starts_with("AB$");

    // Parse using AbilityParams for consistency
    // We need to add a prefix to make it parseable
    // Both AB$ and DB$ use the same A: prefix for parsing
    let prefixed = format!("A:{}", body);

    if let Ok(ability_params) = AbilityParams::parse(&prefixed) {
        let mut params = HashMap::new();
        let mut sub_ability: Option<String> = None;

        // Extract all parameters using the public iter() method
        for (key, value) in ability_params.iter() {
            if key == "SubAbility" {
                sub_ability = Some(value.to_string());
            } else {
                params.insert(key.to_string(), value.to_string());
            }
        }

        ParsedSVar::Effect(EffectDef {
            is_activated,
            api_type: ability_params.api_type,
            params,
            sub_ability,
        })
    } else {
        // Fallback to raw if parsing fails
        ParsedSVar::Raw(body.to_string())
    }
}

/// Parse all SVars from a card's svar HashMap.
///
/// Returns a new HashMap with parsed SVars.
pub fn parse_all_svars(raw_svars: &HashMap<String, String>) -> HashMap<String, ParsedSVar> {
    raw_svars
        .iter()
        .map(|(name, body)| (name.clone(), parse_svar(body)))
        .collect()
}

/// Get a StaticAbilityDef from parsed SVars by name.
pub fn get_static_ability<'a>(
    parsed_svars: &'a HashMap<String, ParsedSVar>,
    name: &str,
) -> Option<&'a StaticAbilityDef> {
    match parsed_svars.get(name) {
        Some(ParsedSVar::StaticAbility(def)) => Some(def),
        _ => None,
    }
}

/// Get an EffectDef from parsed SVars by name.
pub fn get_effect_def<'a>(parsed_svars: &'a HashMap<String, ParsedSVar>, name: &str) -> Option<&'a EffectDef> {
    match parsed_svars.get(name) {
        Some(ParsedSVar::Effect(def)) => Some(def),
        _ => None,
    }
}

/// Check if a static ability's ValidAttacker/Affected uses Card.IsRemembered.
pub fn uses_remembered(def: &StaticAbilityDef) -> bool {
    def.params
        .values()
        .any(|v| v.contains("IsRemembered") || v.contains(".IsRemembered"))
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)] // Tests use wildcards in panic branches
mod tests {
    use super::*;

    #[test]
    fn test_parse_boolean_flag() {
        assert_eq!(parse_svar("TRUE"), ParsedSVar::BooleanFlag(true));
        assert_eq!(parse_svar("true"), ParsedSVar::BooleanFlag(true));
        assert_eq!(parse_svar("FALSE"), ParsedSVar::BooleanFlag(false));
    }

    #[test]
    fn test_parse_sacrificed_expr() {
        match parse_svar("Sacrificed$CardPower") {
            ParsedSVar::ComputedValue(ComputedValueExpr::SacrificedCardPower) => {}
            other => panic!("Expected SacrificedCardPower, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_static_ability_cant_block_by() {
        let svar =
            "Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered | Description$ This creature can't be blocked.";
        match parse_svar(svar) {
            ParsedSVar::StaticAbility(def) => {
                assert_eq!(def.mode, StaticAbilityMode::CantBlockBy);
                assert_eq!(def.params.get("ValidAttacker"), Some(&"Card.IsRemembered".to_string()));
                assert!(def.description.is_some());
            }
            other => panic!("Expected StaticAbility, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_static_ability_continuous() {
        let svar = "Mode$ Continuous | Affected$ Card.IsRemembered | AffectedZone$ Exile | MayPlay$ True | MayPlayWithoutManaCost$ True";
        match parse_svar(svar) {
            ParsedSVar::StaticAbility(def) => {
                assert_eq!(def.mode, StaticAbilityMode::Continuous);
                assert_eq!(def.params.get("Affected"), Some(&"Card.IsRemembered".to_string()));
                assert_eq!(def.params.get("AffectedZone"), Some(&"Exile".to_string()));
                assert_eq!(def.params.get("MayPlay"), Some(&"True".to_string()));
            }
            other => panic!("Expected StaticAbility, got {:?}", other),
        }
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // Test panic branch
    fn test_parse_effect_def_db() {
        let svar = "DB$ Draw | NumCards$ 1";
        match parse_svar(svar) {
            ParsedSVar::Effect(def) => {
                assert!(!def.is_activated);
                assert_eq!(def.api_type, ApiType::Draw);
            }
            other => panic!("Expected Effect, got {:?}", other),
        }
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // Test panic branch
    fn test_parse_effect_def_with_subability() {
        let svar = "DB$ Pump | Defined$ Self | KW$ Flying | SubAbility$ DBGainLife";
        match parse_svar(svar) {
            ParsedSVar::Effect(def) => {
                assert!(!def.is_activated);
                assert_eq!(def.api_type, ApiType::Pump);
                assert_eq!(def.sub_ability, Some("DBGainLife".to_string()));
            }
            other => panic!("Expected Effect, got {:?}", other),
        }
    }

    #[test]
    fn test_uses_remembered() {
        let svar = "Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered";
        if let ParsedSVar::StaticAbility(def) = parse_svar(svar) {
            assert!(uses_remembered(&def));
        } else {
            panic!("Expected StaticAbility");
        }
    }
}
