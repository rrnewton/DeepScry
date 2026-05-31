//! Card effects and ability system

use crate::core::{CardId, Color, Keyword, PlayerId};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Target reference for effects
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRef {
    /// Target a player
    Player(PlayerId),
    /// Target a creature or other permanent
    Permanent(CardId),
    /// No target (e.g., "each player", "all creatures")
    None,
}

/// Controller restriction for targeting
///
/// Used by spells like Cackling Counterpart ("target creature you control")
/// or Ember Island Production modes to restrict targets by controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControllerRestriction {
    /// Target can be controlled by anyone (no restriction)
    #[default]
    Any,
    /// Target must be controlled by the spell/ability's controller
    YouCtrl,
    /// Target must be controlled by an opponent
    OppCtrl,
    /// Target must be controlled by the active player (the player whose turn it
    /// is). Used by "each player's upkeep" triggers like The Abyss
    /// (`ValidTgts$ Creature.nonArtifact+ActivePlayerCtrl`) where the trigger
    /// fires on every player's upkeep and must affect a permanent controlled by
    /// the player whose upkeep it is — i.e. the active player.
    ActivePlayerCtrl,
}

/// Types of permanents that can be targeted
///
/// Used by spells like Disenchant (Artifact, Enchantment) or Terror (Creature)
/// to restrict what can be legally targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetType {
    /// Any permanent (no restriction)
    Any,
    /// Must be an artifact
    Artifact,
    /// Must be an enchantment
    Enchantment,
    /// Must be a creature
    Creature,
    /// Must be a land
    Land,
    /// Must be a planeswalker
    Planeswalker,
}

impl TargetType {
    /// Check if a card matches this target type restriction
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        match self {
            TargetType::Any => true,
            TargetType::Artifact => card.is_artifact(),
            TargetType::Enchantment => card.is_enchantment(),
            TargetType::Creature => card.is_creature(),
            TargetType::Land => card.is_land(),
            TargetType::Planeswalker => card.is_planeswalker(),
        }
    }
}

/// Filter for Dig effect's ChangeValid$ parameter
///
/// Specifies which card types are valid for selection when digging.
/// Parsed from comma-separated values like "Creature,Land" or "Artifact".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigFilter {
    /// Any card is valid
    Card,
    /// Only creatures
    Creature,
    /// Only lands
    Land,
    /// Only artifacts
    Artifact,
    /// Only enchantments
    Enchantment,
    /// Only instants
    Instant,
    /// Only sorceries
    Sorcery,
    /// Only planeswalkers
    Planeswalker,
    /// Any permanent (creature, artifact, enchantment, land, planeswalker)
    Permanent,
}

impl DigFilter {
    /// Parse a single filter token from ChangeValid$ value
    pub fn parse(s: &str) -> Option<Self> {
        // Strip modifiers like ".cmcLE3", ".Legendary", ".nonLand" etc.
        let base = s.split('.').next().unwrap_or(s);
        match base {
            "Card" => Some(DigFilter::Card),
            "Creature" => Some(DigFilter::Creature),
            "Land" => Some(DigFilter::Land),
            "Artifact" => Some(DigFilter::Artifact),
            "Enchantment" => Some(DigFilter::Enchantment),
            "Instant" => Some(DigFilter::Instant),
            "Sorcery" => Some(DigFilter::Sorcery),
            "Planeswalker" => Some(DigFilter::Planeswalker),
            "Permanent" => Some(DigFilter::Permanent),
            _ => None,
        }
    }

    /// Check if a card matches this filter
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        match self {
            DigFilter::Card => true,
            DigFilter::Creature => card.is_creature(),
            DigFilter::Land => card.is_land(),
            DigFilter::Artifact => card.is_artifact(),
            DigFilter::Enchantment => card.is_enchantment(),
            DigFilter::Instant => card.is_instant(),
            DigFilter::Sorcery => card.is_sorcery(),
            DigFilter::Planeswalker => card.is_planeswalker(),
            DigFilter::Permanent => !card.is_instant() && !card.is_sorcery(),
        }
    }
}

/// A dynamic numeric amount whose value is computed from game state at the
/// moment an effect resolves, rather than being a fixed literal.
///
/// This is the general construct behind effects such as:
/// - Swords to Plowshares — "gains life equal to its power" (`TargetPower`)
/// - Divine Offering — "gain life equal to its mana value" (`TargetManaValue`)
/// - Drain Life — "gain life equal to the damage dealt" (`DamageDealt`)
///
/// The amount is derived purely from **public** game state (a card's last-known
/// power / printed mana value, or damage already dealt this resolution), so it
/// is information-independent and produces identical results on the server and
/// every client / WASM shadow game. See `docs/NETWORK_ARCHITECTURE.md`.
///
/// Timing (CR 608.2g/2h): for `TargetPower` / `TargetManaValue` the referenced
/// card may already have left the battlefield earlier in the same resolution
/// (e.g. Swords exiles the creature, then the chained GainLife runs). The
/// engine reads the card's retained characteristics — its **last-known
/// information** — because zone moves do not reset a `Card`'s power / counters /
/// mana cost in the entity store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DynamicAmount {
    /// A fixed literal amount (degenerate case; lets a dynamic-amount effect
    /// also carry a plain number without a separate variant).
    Fixed(i32),

    /// The referenced card's power, captured via last-known information
    /// (CR 608.2g). Used by Swords to Plowshares.
    TargetPower,

    /// The referenced card's mana value (converted mana cost). Used by Divine
    /// Offering ("gain life equal to its mana value").
    TargetManaValue,

    /// The amount of damage actually dealt earlier in this same effect
    /// resolution (read from the spell's damage bookkeeping). Used by Drain
    /// Life ("you gain life equal to the damage dealt").
    DamageDealt,
}

impl DynamicAmount {
    /// Parse a `LifeAmount$ <expr>` value, resolving an `X`/`Y`/`Z` reference
    /// through the card's SVars, into a `DynamicAmount`.
    ///
    /// Recognised SVar bodies (tokenized, never substring-matched):
    /// - `Targeted$CardPower`    -> `TargetPower`
    /// - `Targeted$CardManaCost` -> `TargetManaValue`
    ///
    /// A literal integer parses to `Fixed`. Anything unrecognised returns
    /// `None` so the caller can fall back to the existing fixed-amount path.
    pub fn parse(value: &str, svars: &std::collections::HashMap<String, String>) -> Option<Self> {
        let trimmed = value.trim();
        if let Ok(n) = trimmed.parse::<i32>() {
            return Some(DynamicAmount::Fixed(n));
        }

        // Variable reference (X / Y / Z). Resolve through the card's SVars.
        let svar_body = svars.get(trimmed)?;
        // SVar bodies for these references are `Targeted$<Characteristic>`.
        let (selector, characteristic) = svar_body.split_once('$')?;
        if selector.trim() != "Targeted" {
            return None;
        }
        match characteristic.trim() {
            "CardPower" => Some(DynamicAmount::TargetPower),
            "CardManaCost" => Some(DynamicAmount::TargetManaValue),
            _ => None,
        }
    }
}

/// Count expression for variable effects
///
/// Used by effects that depend on counting game state, like:
/// - "gets +X/+X where X is the number of artifacts your opponents control"
/// - "draw cards equal to the number of creatures you control"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountExpression {
    /// Fixed value (not actually a count, just wraps a constant)
    Fixed(i32),

    /// Count permanents matching a filter (Count$Valid filter)
    /// Filter examples: "Artifact.OppCtrl", "Creature.YouCtrl", "Land.YouCtrl"
    ValidPermanents {
        /// The filter string (e.g., "Artifact.OppCtrl")
        filter: String,
    },

    /// Count cards drawn this turn (Count$YouDrewThisTurn)
    CardsDrawnThisTurn,

    /// The value of X paid when casting this spell (Count$xPaid)
    /// Resolved at effect execution time by reading Card::x_paid
    XPaid,

    /// Count spells cast this turn (Count$YouCastThisTurn)
    SpellsCastThisTurn,

    /// Compare a source count against a condition and return true/false value
    /// Pattern: Count$Compare SourceSVar Condition.TrueValue.FalseValue
    /// Example: Count$Compare Y GE1.2.1 → if Y >= 1 then 2 else 1
    Compare {
        /// The nested count expression to evaluate (resolved from SVar)
        source: Box<CountExpression>,
        /// Comparison operator and threshold (e.g., "GE1" for >= 1)
        condition: CompareCondition,
        /// Value to return if condition is true
        true_value: i32,
        /// Value to return if condition is false
        false_value: i32,
    },
}

/// Comparison conditions for Count$Compare expressions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareCondition {
    /// Greater or Equal (GE1 = >= 1)
    GreaterOrEqual(i32),
    /// Less or Equal (LE0 = <= 0)
    LessOrEqual(i32),
    /// Equal (EQ3 = == 3)
    Equal(i32),
    /// Greater Than (GT2 = > 2)
    GreaterThan(i32),
    /// Less Than (LT5 = < 5)
    LessThan(i32),
}

impl CompareCondition {
    /// Parse a condition string like "GE1", "LE0", "EQ3", "GT2", "LT5"
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(rest) = s.strip_prefix("GE") {
            rest.parse().ok().map(CompareCondition::GreaterOrEqual)
        } else if let Some(rest) = s.strip_prefix("LE") {
            rest.parse().ok().map(CompareCondition::LessOrEqual)
        } else if let Some(rest) = s.strip_prefix("EQ") {
            rest.parse().ok().map(CompareCondition::Equal)
        } else if let Some(rest) = s.strip_prefix("GT") {
            rest.parse().ok().map(CompareCondition::GreaterThan)
        } else if let Some(rest) = s.strip_prefix("LT") {
            rest.parse().ok().map(CompareCondition::LessThan)
        } else {
            None
        }
    }

    /// Evaluate the condition against a value
    pub fn evaluate(&self, value: i32) -> bool {
        match self {
            CompareCondition::GreaterOrEqual(threshold) => value >= *threshold,
            CompareCondition::LessOrEqual(threshold) => value <= *threshold,
            CompareCondition::Equal(threshold) => value == *threshold,
            CompareCondition::GreaterThan(threshold) => value > *threshold,
            CompareCondition::LessThan(threshold) => value < *threshold,
        }
    }
}

impl Default for CountExpression {
    fn default() -> Self {
        CountExpression::Fixed(0)
    }
}

impl CountExpression {
    /// Parse a count expression from a string value and optional SVars
    ///
    /// Examples:
    /// - "+3" -> Fixed(3)
    /// - "X" with SVar X = "Count$Valid Artifact.OppCtrl" -> ValidPermanents { filter: "Artifact.OppCtrl" }
    /// - "X" with SVar X = "Count$YouDrewThisTurn" -> CardsDrawnThisTurn
    /// - "X" with SVar X = "Count$Compare Y GE1.2.1" -> Compare { source, condition, true_value, false_value }
    pub fn parse(value: &str, svars: &std::collections::HashMap<String, String>) -> Self {
        Self::parse_internal(value, svars, 0)
    }

    /// Internal parse with recursion depth to prevent infinite loops
    fn parse_internal(value: &str, svars: &std::collections::HashMap<String, String>, depth: u8) -> Self {
        // Prevent infinite recursion
        if depth > 10 {
            return CountExpression::Fixed(0);
        }

        // Try parsing as fixed integer first
        let trimmed = value.trim_start_matches('+');
        if let Ok(n) = trimmed.parse::<i32>() {
            return CountExpression::Fixed(n);
        }

        // Check if it's a variable reference (X, Y, Z, -X, etc.)
        let var_name = value.trim_start_matches('+').trim_start_matches('-');

        // Look up the SVar
        if let Some(svar_value) = svars.get(var_name) {
            // Parse Count$ expressions
            if let Some(rest) = svar_value.strip_prefix("Count$") {
                if rest == "xPaid" {
                    return CountExpression::XPaid;
                } else if rest.starts_with("Valid ") {
                    // Count$Valid filter
                    let filter = rest.strip_prefix("Valid ").unwrap_or(rest).to_string();
                    return CountExpression::ValidPermanents { filter };
                } else if rest == "YouDrewThisTurn" {
                    return CountExpression::CardsDrawnThisTurn;
                } else if rest == "YouCastThisTurn" {
                    return CountExpression::SpellsCastThisTurn;
                } else if rest.starts_with("Compare ") {
                    // Count$Compare SourceSVar Condition.TrueValue.FalseValue
                    // Example: "Compare Y GE1.2.1"
                    let compare_parts: Vec<&str> = rest.strip_prefix("Compare ").unwrap().splitn(2, ' ').collect();
                    if compare_parts.len() == 2 {
                        let source_svar = compare_parts[0];
                        let cond_parts: Vec<&str> = compare_parts[1].split('.').collect();
                        if cond_parts.len() == 3 {
                            if let (Some(condition), Ok(true_val), Ok(false_val)) = (
                                CompareCondition::parse(cond_parts[0]),
                                cond_parts[1].parse::<i32>(),
                                cond_parts[2].parse::<i32>(),
                            ) {
                                // Recursively parse the source SVar
                                let source = Self::parse_internal(source_svar, svars, depth + 1);
                                return CountExpression::Compare {
                                    source: Box::new(source),
                                    condition,
                                    true_value: true_val,
                                    false_value: false_val,
                                };
                            }
                        }
                    }
                }
            }
        }

        // Unknown expression - return 0
        CountExpression::Fixed(0)
    }

    /// Check if this is a fixed value (not actually variable)
    pub fn is_fixed(&self) -> bool {
        matches!(self, CountExpression::Fixed(_))
    }
}

/// Restrictions on what types of permanents can be targeted
///
/// For spells like Disenchant ("destroy target artifact or enchantment"),
/// this would contain [Artifact, Enchantment].
/// For Terror ("destroy target creature"), this would contain [Creature].
/// An empty vec means any permanent can be targeted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRestriction {
    /// Valid target types (if empty, any permanent is valid)
    pub types: SmallVec<[TargetType; 2]>,
    /// If true, target must have no counters on it (e.g., Heartless Act mode 1)
    #[serde(default)]
    pub requires_no_counters: bool,
    /// Controller restriction (e.g., YouCtrl, OppCtrl)
    #[serde(default)]
    pub controller: ControllerRestriction,
    /// Minimum power requirement (e.g., powerGE4 means power >= 4)
    #[serde(default)]
    pub power_ge: Option<i32>,
    /// Maximum power requirement (e.g., powerLE2 means power <= 2)
    #[serde(default)]
    pub power_le: Option<i32>,
    /// If true, target must not be a token (e.g., Chaos Orb)
    #[serde(default)]
    pub requires_nontoken: bool,
    /// If true, target must be in the "remembered" set (unimplemented — always fails)
    #[serde(default)]
    pub requires_remembered: bool,
    /// If true, target must NOT be an artifact (e.g. The Abyss's
    /// `Creature.nonArtifact`). Artifact creatures are excluded.
    #[serde(default)]
    pub requires_nonartifact: bool,
    /// Required color of the target, from a color qualifier in `ValidTgts$`
    /// (e.g. `Permanent.Blue`, `Card.Red`). `None` = no color restriction.
    /// Used by Red/Blue Elemental Blast, Pyroblast, Hydroblast, and color-hosers.
    #[serde(default)]
    pub required_color: Option<crate::core::Color>,
}

impl TargetRestriction {
    /// Create a restriction allowing any permanent
    pub fn any() -> Self {
        Self {
            types: SmallVec::new(),
            requires_no_counters: false,
            controller: ControllerRestriction::Any,
            power_ge: None,
            power_le: None,
            requires_nontoken: false,
            requires_remembered: false,
            requires_nonartifact: false,
            required_color: None,
        }
    }

    /// Create a restriction from a list of target types
    pub fn from_types(types: impl IntoIterator<Item = TargetType>) -> Self {
        Self {
            types: types.into_iter().collect(),
            requires_no_counters: false,
            controller: ControllerRestriction::Any,
            power_ge: None,
            power_le: None,
            requires_nontoken: false,
            requires_remembered: false,
            requires_nonartifact: false,
            required_color: None,
        }
    }

    /// Render a short, human-readable description of this restriction for game
    /// logs (e.g. "artifacts you control", "blue permanents", "nontoken
    /// creatures"). Avoids dumping the raw `Debug` struct into the gamelog,
    /// which counts as a sentinel/BROKEN log per the compatibility skill.
    /// Used by `Effect::ChangeZoneAll` / `Effect::PutCounterAll` logging.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(color) = self.required_color {
            parts.push(format!("{color:?}").to_lowercase());
        }
        if self.requires_nontoken {
            parts.push("nontoken".to_string());
        }
        if self.requires_nonartifact {
            parts.push("nonartifact".to_string());
        }
        // Noun: the matched type(s), defaulting to "cards" (the generic filter
        // `ChangeType$ Card` / unrestricted matches any card, e.g. Timetwister
        // shuffling hand+graveyard). Callers describing battlefield-only moves
        // still read naturally ("all cards on the battlefield").
        let noun = if self.types.is_empty() {
            "cards".to_string()
        } else {
            let names: Vec<String> = self.types.iter().map(|t| format!("{t:?}").to_lowercase()).collect();
            // Pluralize the simple way (good enough for log readability).
            format!("{}s", names.join("/"))
        };
        parts.push(noun);
        let mut desc = parts.join(" ");
        // Controller / power qualifiers as a trailing clause.
        let ctrl = match self.controller {
            ControllerRestriction::YouCtrl => Some("you control"),
            ControllerRestriction::OppCtrl => Some("an opponent controls"),
            ControllerRestriction::ActivePlayerCtrl => Some("the active player controls"),
            ControllerRestriction::Any => None,
        };
        if let Some(c) = ctrl {
            desc.push_str(&format!(" {c}"));
        }
        if let Some(ge) = self.power_ge {
            desc.push_str(&format!(" with power >= {ge}"));
        }
        if let Some(le) = self.power_le {
            desc.push_str(&format!(" with power <= {le}"));
        }
        if self.requires_no_counters {
            desc.push_str(" with no counters");
        }
        desc
    }

    /// Check if a card matches this restriction (type, counter, and power checks)
    ///
    /// Returns true if:
    /// - types is empty (any permanent allowed), OR card matches at least one of the specified types
    /// - requires_no_counters is false, OR card has no counters
    /// - power_ge is None, OR card's power >= power_ge
    /// - power_le is None, OR card's power <= power_le
    ///
    /// Note: This does NOT check controller restrictions. Use `matches_with_controller`
    /// for full validation including controller checks.
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        // "Remembered" cards require FlipOntoBattlefield which is unimplemented
        if self.requires_remembered {
            return false;
        }

        // Check token restriction
        if self.requires_nontoken && card.is_token {
            return false;
        }

        // Check nonartifact restriction (e.g. The Abyss targets nonartifact creatures)
        if self.requires_nonartifact && card.is_artifact() {
            return false;
        }

        // Check color restriction (e.g. Red Elemental Blast's `Permanent.Blue`
        // destroy mode may only hit BLUE permanents). A basic Mountain is a
        // *colorless* land, so `Permanent.Red` does NOT match it (CR 105.2a:
        // a land type does not grant color); only genuinely red permanents
        // (red creatures, red artifacts, etc.) qualify.
        if let Some(color) = self.required_color {
            if !card.is_color(color) {
                return false;
            }
        }

        // Check counter restriction
        if self.requires_no_counters && card.has_counters() {
            return false;
        }

        // Check power restrictions (for creatures)
        if let Some(min_power) = self.power_ge {
            if i32::from(card.current_power()) < min_power {
                return false;
            }
        }
        if let Some(max_power) = self.power_le {
            if i32::from(card.current_power()) > max_power {
                return false;
            }
        }

        // Check type restriction
        if self.types.is_empty() {
            return true; // No type restriction
        }
        self.types.iter().any(|t| t.matches(card))
    }

    /// Check if a card matches this restriction including controller checks
    ///
    /// # Arguments
    /// * `card` - The target card to check
    /// * `spell_controller` - The controller of the spell/ability
    /// * `target_controller` - The controller of the target card
    ///
    /// Returns true if all restrictions match:
    /// - Type restriction matches
    /// - Counter restriction matches
    /// - Controller restriction matches (YouCtrl/OppCtrl/Any)
    pub fn matches_with_controller(
        &self,
        card: &crate::core::Card,
        spell_controller: PlayerId,
        target_controller: PlayerId,
    ) -> bool {
        // Check type and counter restrictions
        if !self.matches(card) {
            return false;
        }

        // Check controller restriction
        match self.controller {
            ControllerRestriction::Any => true,
            ControllerRestriction::YouCtrl => target_controller == spell_controller,
            ControllerRestriction::OppCtrl => target_controller != spell_controller,
            // ActivePlayerCtrl cannot be resolved without knowing the active
            // player. Callers that need it (the trigger auto-target site for
            // "each player's upkeep" effects) check it explicitly against the
            // active player; here we conservatively treat it as YouCtrl, which
            // is correct for the common case where the trigger fires on the
            // controller's own upkeep.
            ControllerRestriction::ActivePlayerCtrl => target_controller == spell_controller,
        }
    }

    /// Parse ValidTgts string from Java Forge format
    ///
    /// Examples:
    /// - "Artifact,Enchantment" -> [Artifact, Enchantment]
    /// - "Creature" -> [Creature]
    /// - "Creature.YouCtrl" -> [Creature] with YouCtrl controller restriction
    /// - "Creature.OppCtrl" -> [Creature] with OppCtrl controller restriction
    /// - "Creature.nonArtifact+nonBlack" -> [Creature] with requires_nonartifact=true (nonBlack ignored)
    /// - "Creature.nonArtifact+ActivePlayerCtrl" -> [Creature] nonartifact, ActivePlayerCtrl (The Abyss)
    /// - "Creature.!HasCounters" -> [Creature] with requires_no_counters=true
    /// - "Creature.powerGE4" -> [Creature] with power_ge=4
    /// - "Creature.powerLE2" -> [Creature] with power_le=2
    pub fn parse(valid_tgts: &str) -> Self {
        let mut types = SmallVec::new();
        let mut requires_no_counters = false;
        let mut requires_nontoken = false;
        let mut requires_remembered = false;
        let mut requires_nonartifact = false;
        let mut controller = ControllerRestriction::Any;
        let mut power_ge = None;
        let mut power_le = None;
        let mut required_color = None;

        for part in valid_tgts.split(',') {
            // Check for modifiers after the base type
            // Example: "Creature.YouCtrl" or "Creature.Other+YouCtrl+powerLE2"
            let parts: Vec<&str> = part.split('.').collect();
            let base_type = parts.first().map(|s| s.trim()).unwrap_or("");

            // Check for modifiers (may be combined with +)
            for modifier_part in parts.iter().skip(1) {
                // Split by + to handle combined modifiers like "Other+YouCtrl"
                for modifier in modifier_part.split('+') {
                    match modifier {
                        "!HasCounters" => requires_no_counters = true,
                        "!token" => requires_nontoken = true,
                        "IsRemembered" => requires_remembered = true,
                        "YouCtrl" => controller = ControllerRestriction::YouCtrl,
                        "OppCtrl" => controller = ControllerRestriction::OppCtrl,
                        "ActivePlayerCtrl" => controller = ControllerRestriction::ActivePlayerCtrl,
                        "nonArtifact" => requires_nonartifact = true,
                        "White" => required_color = Some(crate::core::Color::White),
                        "Blue" => required_color = Some(crate::core::Color::Blue),
                        "Black" => required_color = Some(crate::core::Color::Black),
                        "Red" => required_color = Some(crate::core::Color::Red),
                        "Green" => required_color = Some(crate::core::Color::Green),
                        m if m.starts_with("powerGE") => {
                            // Parse powerGE4 -> power_ge = 4
                            if let Ok(n) = m.trim_start_matches("powerGE").parse::<i32>() {
                                power_ge = Some(n);
                            }
                        }
                        m if m.starts_with("powerLE") => {
                            // Parse powerLE2 -> power_le = 2
                            if let Ok(n) = m.trim_start_matches("powerLE").parse::<i32>() {
                                power_le = Some(n);
                            }
                        }
                        _ => {} // Other modifiers ignored for now
                    }
                }
            }

            match base_type {
                "Artifact" => types.push(TargetType::Artifact),
                "Enchantment" => types.push(TargetType::Enchantment),
                "Creature" => types.push(TargetType::Creature),
                "Land" => types.push(TargetType::Land),
                "Planeswalker" => types.push(TargetType::Planeswalker),
                // "Any", "Permanent", or unrecognized - allow any
                _ => {}
            }
        }

        Self {
            types,
            requires_no_counters,
            controller,
            power_ge,
            power_le,
            requires_nontoken,
            requires_remembered,
            requires_nonartifact,
            required_color,
        }
    }
}

/// Basic card effects that can be executed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// Deal damage to a target
    /// Example: "Lightning Bolt deals 3 damage to any target"
    DealDamage { target: TargetRef, amount: i32 },

    /// Deal X damage to a target, where X is the value paid when casting
    /// Example: "Fireball deals X damage" (SVar:X:Count$xPaid)
    /// Amount is read from Card::x_paid at resolution time
    DealDamageXPaid { target: TargetRef },

    /// Deal a variable amount of damage to the player whose upkeep/phase the
    /// trigger fired on (the "triggered" / "chosen" / active player). The
    /// damage amount is a `CountExpression` evaluated **against that same
    /// player** at resolution time.
    ///
    /// Used by "each player's upkeep" phase triggers that punish the active
    /// player by a count of their own permanents/cards:
    /// - Karma: `T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player` with
    ///   `DB$ DealDamage | Defined$ TriggeredPlayer | NumDmg$ X` where
    ///   `SVar:X:Count$Valid Swamp.ActivePlayerCtrl` — deals damage equal to
    ///   the number of Swamps the active player controls.
    /// - The Tabernacle at Pendrell Vale and other "ActivePlayerCtrl" punishers
    ///   share the same shape.
    ///
    /// `target_self` distinguishes `Defined$ You` (controller-only upkeep
    /// punishers, e.g. cards that hurt their own controller) from the
    /// "each player" / chosen-player case. When `target_self` is true the
    /// damage and count both resolve against the trigger source's controller;
    /// otherwise both resolve against the active player whose upkeep fired.
    DealDamageToTriggeredPlayer {
        /// Amount of damage, evaluated against the resolved target player.
        count: CountExpression,
        /// If true, target/count resolve against the trigger source's
        /// controller (`Defined$ You`); if false, against the active player
        /// (`Defined$ TriggeredPlayer` / `ChosenPlayer`).
        target_self: bool,
    },

    /// Multiple creatures deal damage to a single target
    /// Example: "Up to two target creatures you control each deal damage equal to their power
    /// to target creature an opponent controls"
    /// Corresponds to: DB$ EachDamage | DefinedDamagers$ ParentTarget | NumDmg$ Count$CardPower
    ///
    /// Used by cards like Allies at Last, Band Together, Tandem Takedown
    ///
    /// At parse time: damagers is empty, receiver is placeholder
    /// At spell resolution: filled via resolve_effect_target from chosen_targets
    EachDamage {
        /// Creatures dealing damage (resolved from chosen_targets at spell resolution)
        /// Empty Vec at parse time means "use parent targets" (DefinedDamagers$ ParentTarget)
        damagers: smallvec::SmallVec<[CardId; 4]>,
        /// Target receiving damage (placeholder CardId::new(0) at parse, resolved at spell resolution)
        receiver: CardId,
        /// Whether each damager deals damage equal to its power (NumDmg$ Count$CardPower)
        /// If false, uses fixed_damage
        use_card_power: bool,
        /// Fixed damage per damager (used if !use_card_power)
        fixed_damage: i32,
    },

    /// Draw cards
    /// Example: "Draw a card"
    DrawCards { player: PlayerId, count: u8 },

    /// Draw X cards, where X is the value paid when casting
    /// Example: "Target player draws X cards" (Braingeyser, SVar:X:Count$xPaid)
    DrawCardsXPaid { player: PlayerId },

    /// Looting effect (discard then draw)
    /// Example: "Discard a card, then draw a card"
    /// Corresponds to: AB$ Draw | Cost$ Discard<N/Card> (requires discarding N cards first)
    Loot {
        player: PlayerId,
        discard_count: u8,
        draw_count: u8,
    },

    /// Discard cards
    /// Example: "Discard a card"
    /// Corresponds to: DB$ Discard | Defined$ You | NumCards$ 1 | RememberDiscarded$ True
    DiscardCards {
        player: PlayerId,
        count: u8,
        /// If true, store discarded cards in game.remembered_cards for ImmediateTrigger
        remember_discarded: bool,
        /// If true, each player may choose whether to discard (Optional$ True)
        /// Used by Raphael's Technique: "Each player may discard their hand"
        #[serde(default)]
        optional: bool,
        /// If true, store which players actually discarded in game.remembered_players
        /// Used by Raphael's Technique: draw 7 only for players who discarded
        #[serde(default)]
        remember_discarding_players: bool,
    },

    /// Discard X cards, where X is the value paid when casting
    /// Example: "Target player discards X cards at random" (Mind Twist, SVar:X:Count$xPaid)
    DiscardCardsXPaid { player: PlayerId, remember_discarded: bool },

    /// Gain life
    /// Example: "You gain 3 life"
    GainLife { player: PlayerId, amount: i32 },

    /// Gain life by a dynamic amount computed from game state at resolution.
    ///
    /// Example: Swords to Plowshares — "Its controller gains life equal to its
    /// power" (`amount = TargetPower`, `reference` = the exiled creature);
    /// Divine Offering — "you gain life equal to its mana value"
    /// (`amount = TargetManaValue`, `reference` = the destroyed artifact).
    ///
    /// `reference` names the card the amount is read from (filled in at
    /// resolution from the spell's targeted permanent). For `DamageDealt` the
    /// reference is unused and may be a placeholder. The amount is resolved by
    /// `execute_effect` reading public, last-known game state, keeping the
    /// effect information-independent (network + WASM safe).
    GainLifeDynamic {
        player: PlayerId,
        amount: DynamicAmount,
        reference: CardId,
    },

    /// Lose life
    /// Example: "Target player loses 2 life"
    LoseLife { player: PlayerId, amount: i32 },

    /// Destroy a permanent
    /// Example: "Destroy target creature" or "Destroy target artifact or enchantment"
    DestroyPermanent {
        target: CardId,
        /// Restriction on what types can be targeted (e.g., [Artifact, Enchantment] for Disenchant)
        restriction: TargetRestriction,
        /// If true, the destroyed permanent can't be regenerated (CR 701.15).
        /// Set by `NoRegen$ True` on the script (e.g. The Abyss, Terror).
        #[serde(default)]
        no_regenerate: bool,
    },

    /// Destroy all permanents matching a filter
    /// Example: "Destroy all creatures" (Wrath of God)
    DestroyAll {
        /// Filter for which permanents to destroy
        restriction: TargetRestriction,
        /// If true, destroyed permanents can't be regenerated (CR 701.15)
        no_regenerate: bool,
    },

    /// Each player sacrifices all permanents matching a filter
    /// Example: "Each player sacrifices all permanents they control that are one or more colors" (All is Dust)
    SacrificeAll {
        /// Filter for which permanents to sacrifice
        restriction: TargetRestriction,
    },

    /// Deal damage to all creatures (and optionally players) matching a filter
    /// Example: "Deal 2 damage to each creature" (Pyroclasm)
    DamageAll {
        /// Amount of damage to deal
        amount: i32,
        /// Filter for which creatures/permanents to damage
        valid_cards: TargetRestriction,
        /// Also damage players? (e.g., "each creature and each player")
        damage_players: bool,
    },

    /// Force a player to sacrifice permanents
    /// Example: "Target player sacrifices a creature" (Diabolic Edict)
    ForceSacrifice {
        /// The player who must sacrifice
        player: PlayerId,
        /// Type of permanent to sacrifice (e.g., "Creature", "Permanent")
        sac_type: String,
        /// Number of permanents to sacrifice
        count: u8,
    },

    /// Tap all permanents matching a filter
    /// Example: "Tap all creatures your opponents control" (Cryptic Command)
    TapAll { restriction: TargetRestriction },

    /// Untap all permanents matching a filter
    /// Example: "Untap all creatures you control"
    UntapAll { restriction: TargetRestriction },

    /// Set a player's life total to a specific value
    /// Example: "Target opponent's life total becomes 10" (Sorin Markov)
    SetLife { player: PlayerId, amount: i32 },

    /// Gain control of a permanent
    /// Example: "Gain control of target creature" (Control Magic, Threaten)
    /// Reference: Java Forge AB$ GainControl
    GainControl {
        /// The permanent to gain control of
        target: CardId,
        /// The new controller (resolved at cast time)
        new_controller: PlayerId,
        /// Whether to also untap the stolen permanent
        untap: bool,
        /// Duration: true = until end of turn (Threaten), false = permanent (Control Magic)
        until_eot: bool,
    },

    /// Fight - two creatures deal damage equal to their power to each other (CR 701.12)
    /// Example: "Target creature you control fights target creature you don't control"
    /// Reference: Java Forge AB$ Fight
    Fight {
        /// The creature that initiates the fight (typically "your" creature)
        fighter: CardId,
        /// The target creature to fight against (typically opponent's creature)
        target: CardId,
    },

    /// Tap a permanent
    /// Example: "Tap target creature"
    TapPermanent { target: CardId },

    /// Untap a permanent
    /// Example: "Untap target land"
    UntapPermanent { target: CardId },

    /// Tap or untap target permanent (player chooses which)
    /// Example: "You may tap or untap target creature" (Bounding Krasis ETB)
    /// AI heuristic: tap opponent's creatures, untap our own
    TapOrUntapPermanent { target: CardId },

    /// Pump (temporary stat boost and/or keyword grant) until end of turn
    /// Example: "Target creature gets +3/+3 until end of turn"
    /// Example with keyword: "Target creature gains double strike until end of turn"
    PumpCreature {
        target: CardId,
        power_bonus: i32,
        toughness_bonus: i32,
        /// Keywords to grant (e.g., Double Strike from KW$ parameter)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Debuff: Remove keywords from a creature (inverse of Pump keyword granting)
    /// Example: "CARDNAME loses defender until end of turn"
    /// Example: "Target creature loses flying until end of turn"
    DebuffCreature {
        target: CardId,
        /// Keywords to remove (from Keywords$ parameter, separated by " & ")
        keywords_removed: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Pump all creatures matching a filter until end of turn
    /// Example: "Creatures you control get +1/+0 until end of turn"
    PumpAllCreatures {
        controller: PlayerId,
        /// Filter string like "Creature.YouCtrl" or "Creature"
        filter: String,
        power_bonus: i32,
        toughness_bonus: i32,
    },

    /// Set base P/T and/or grant keywords to all matching permanents until end of turn
    /// Example: "All creatures you control become 4/4 Dragons with Flying"
    /// Combines SetBasePowerToughness + PumpAllCreatures semantics for mass animation.
    AnimateAll {
        controller: PlayerId,
        /// Filter string like "Creature.YouCtrl", "Planeswalker.YouCtrl"
        filter: String,
        /// Base power to set (None = don't change)
        power: Option<i32>,
        /// Base toughness to set (None = don't change)
        toughness: Option<i32>,
        /// Keywords to grant (e.g., Flying, Trample)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Pump with variable bonus based on counting game state
    /// Example: "This creature gets +X/+X until end of turn, where X is the number of artifacts your opponents control"
    ///
    /// Used by cards like Elephant-Mandrill where the pump amount depends on Count$Valid
    PumpCreatureVariable {
        target: CardId,
        /// Count expression for power bonus (e.g., "Artifact.OppCtrl" for Count$Valid Artifact.OppCtrl)
        power_count: CountExpression,
        /// Count expression for toughness bonus
        toughness_count: CountExpression,
        /// Keywords to grant (optional)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Mill cards from library to graveyard
    /// Example: "Target player mills 3 cards"
    Mill { player: PlayerId, count: u8 },

    /// Empty a player's unspent mana pool ("lose all unspent mana").
    /// Used by Power Sink's not-paid rider. `player` may be a sentinel
    /// (`target_controller()` / placeholder) resolved at execution time.
    DrainMana { player: PlayerId },

    /// Scry - look at top N cards and put any number on bottom
    /// Example: "Scry 1" or "Scry 2"
    /// Corresponds to: DB$ Scry | ScryNum$ N
    ///
    /// AI heuristic: Keep spells, put excess lands on bottom
    Scry { player: PlayerId, count: u8 },

    /// Take an extra turn after this one (CR 500.7)
    /// Example: "Take an extra turn after this one" (Time Walk)
    AddTurn {
        /// Player who takes the extra turn
        player: PlayerId,
        /// Number of extra turns to add
        num_turns: u8,
    },

    /// Surveil N - look at top N cards, put any number into graveyard, rest on top (CR 701.42)
    /// Example: "Surveil 2" (Thought Erasure)
    ///
    /// AI heuristic: Put non-creature, non-land cards into graveyard (fuel for graveyard strategies)
    Surveil { player: PlayerId, count: u8 },

    /// Counter a spell on the stack
    /// Example: "Counter target spell"
    ///
    /// `required_color` restricts which spells are legal targets, from a color
    /// qualifier in `ValidTgts$` (e.g. Red Elemental Blast's `Card.Blue` =
    /// "counter target blue spell"). `None` = any spell (plain Counterspell).
    CounterSpell {
        target: CardId,
        #[serde(default)]
        required_color: Option<crate::core::Color>,
    },

    /// Add mana to a player's mana pool
    /// Example: "Add {G}" or "Add {C}{C}"
    AddMana {
        player: PlayerId,
        mana: crate::core::ManaCost,
        /// If true, this ability also produces mana of the card's chosen color
        /// (for cards like Thriving lands that have "Produced$ Combo G Chosen")
        produces_chosen_color: bool,
        /// Optional variable name for amount (e.g., "X" for cards like Raucous Audience)
        /// If present, resolved via card's SVars to a CountExpression at execution time
        #[serde(default)]
        amount_var: Option<String>,
    },

    /// Put counters on a permanent
    /// Example: "Put a +1/+1 counter on target creature"
    PutCounter {
        target: CardId,
        counter_type: crate::core::CounterType,
        amount: u8,
    },

    /// Put counters on all permanents matching a filter
    /// Example: "Put a +1/+1 counter on each creature you control"
    PutCounterAll {
        restriction: TargetRestriction,
        counter_type: crate::core::CounterType,
        amount: u8,
    },

    /// Proliferate: choose any number of permanents and/or players that have
    /// a counter, then give each one additional counter of each kind already there (CR 701.34a)
    /// Example: "Martyr for the Cause: When CARDNAME dies, proliferate."
    Proliferate,

    /// Remove counters from a permanent
    /// Example: "Remove a +1/+1 counter from target creature"
    /// When counter_type is None, removes counters of any type (CounterType$ Any)
    RemoveCounter {
        target: CardId,
        /// None means "any counter type" (CounterType$ Any)
        counter_type: Option<crate::core::CounterType>,
        amount: u8,
    },

    /// Multiply (double) counters on a permanent
    /// Example: "Double the number of +1/+1 counters on target creature"
    /// If counter_type is None, doubles ALL counter types on the target
    MultiplyCounter {
        target: CardId,
        /// Counter type to multiply (None = all types)
        counter_type: Option<crate::core::CounterType>,
        /// Multiplier (default 2 = double)
        multiplier: u8,
    },

    /// Exile a permanent
    /// Example: "Exile target creature" (Swords to Plowshares)
    /// Moves a card from the battlefield to the exile zone
    ExilePermanent { target: CardId },

    /// Self-exile from the stack (override default sorcery → graveyard).
    ///
    /// Corresponds to `A:SP$ ChangeZone | Origin$ Stack | Destination$ Exile`,
    /// e.g. All Hallow's Eve ("Exile this card with two scream counters on
    /// it.") or any other "this spell goes to exile when it resolves" effect.
    ///
    /// During spell resolution this effect moves the source card from the
    /// stack to exile. `resolve_spell_finalize` then notices the card is no
    /// longer on the stack and skips its default move-to-graveyard step, so
    /// the spell ends up in exile rather than the graveyard.
    ///
    /// If `remember_changed` is true the moved card is pushed onto
    /// `GameState::remembered_cards` so chained `Defined$ Remembered`
    /// sub-abilities (e.g. "put two scream counters on it") can target it.
    ///
    /// The `source` field is filled in by `resolve_self_target` at spell
    /// resolution time (it is `CardId::self_target()` straight out of the
    /// converter, since the effect always operates on the resolving spell).
    SelfExileFromStack {
        /// The resolving spell's CardId. Stored as `self_target()` after
        /// parsing and patched to the actual source by `resolve_self_target`.
        source: CardId,
        /// Whether to push the moved card onto `remembered_cards` so chained
        /// SubAbilities with `Defined$ Remembered` can find it.
        remember_changed: bool,
    },

    /// Move all cards matching a filter from one zone to another
    /// Example: "Return all attacking creatures to their owner's hand" (Aetherize)
    /// Example: "Exile all cards from all graveyards" (Tormod's Crypt)
    ChangeZoneAll {
        /// Filter for which cards to move
        restriction: TargetRestriction,
        /// Source zone(s). Most mass-zone effects have a single origin
        /// (e.g. "return all creatures from the battlefield"), but some move
        /// cards out of TWO zones at once — e.g. Timetwister shuffles each
        /// player's Hand AND Graveyard into the library
        /// (`Origin$ Hand,Graveyard | UseAllOriginZones$ True`).
        origins: SmallVec<[crate::zones::Zone; 2]>,
        /// Destination zone
        destination: crate::zones::Zone,
        /// `Shuffle$ True` — shuffle each affected player's library after the
        /// move. Set for mass shuffle-back effects (Timetwister, Mnemonic
        /// Nexus, Midnight Clock). Left false for ordered library moves like
        /// `LibraryPosition$ -1` (bottom-of-library, e.g. Manifold Insights).
        shuffle: bool,
    },

    /// Move the source card itself between two named zones (neither of which is
    /// the stack — that case is `SelfExileFromStack`).
    ///
    /// Corresponds to `DB$ ChangeZone | Defined$ Self | Origin$ <zone> |
    /// Destination$ <zone>` executed by a triggered ability whose source lives
    /// outside the battlefield. All Hallow's Eve uses
    /// `DB$ ChangeZone | Origin$ Exile | Destination$ Graveyard | Defined$ Self`
    /// to put itself into the graveyard once its last scream counter is removed.
    ///
    /// `source` starts as `CardId::self_target()` from the converter and is
    /// patched to the resolving card by `resolve_self_target` (spells) or by the
    /// trigger placeholder resolution (triggered abilities).
    MoveSelfBetweenZones {
        /// The source card. `self_target()` until patched to the real CardId.
        source: CardId,
        /// Zone the card must currently be in to move.
        origin: crate::zones::Zone,
        /// Destination zone.
        destination: crate::zones::Zone,
    },

    /// Execute an inner effect only if the source card currently satisfies a
    /// counter-count condition (e.g. `ConditionPresent$ Card.counters_EQ0_SCREAM`).
    ///
    /// Corresponds to `DB$ ... | ConditionDefined$ Self |
    /// ConditionPresent$ Card.counters_<CMP>_<TYPE>` chains. All Hallow's Eve
    /// uses this so the move-to-graveyard and mass-resurrection only fire on the
    /// upkeep where the final scream counter was removed (counters == 0).
    ConditionalSelfCounter {
        /// The card whose counters are inspected. `self_target()` until patched
        /// to the real source CardId by the trigger/spell resolver.
        source: CardId,
        /// Counter condition evaluated against `source`.
        condition: SelfCounterCondition,
        /// Effect to run when the condition holds.
        inner: Box<Effect>,
    },

    /// Search library for a card and put it into a zone
    /// Example: "Search your library for a basic land card, put it onto the battlefield tapped, then shuffle"
    /// Corresponds to: AB$ ChangeZone | Origin$ Library | Destination$ Battlefield | ChangeType$ Land.Basic
    SearchLibrary {
        /// Player whose library to search
        player: PlayerId,
        /// Card type filter (e.g., "Land.Basic", "Creature", "Land")
        card_type_filter: String,
        /// Destination zone for the found card
        destination: crate::zones::Zone,
        /// Whether the card enters tapped (for battlefield)
        enters_tapped: bool,
        /// Whether to shuffle after searching
        shuffle: bool,
    },

    /// Attach Equipment to target creature
    /// Example: Spider-Suit's Equip ability
    /// Corresponds to: K:Equip:3
    /// The source_equipment field is filled in when the ability is activated
    AttachEquipment {
        /// The Equipment to attach (filled in during activation)
        source_equipment: CardId,
        /// Target creature to attach to
        target_creature: CardId,
    },

    /// Create token(s) under a player's control
    /// Example: Spider-Ham creates a Food token
    /// Corresponds to: DB$ Token | TokenAmount$ 1 | TokenScript$ c_a_food_sac | TokenOwner$ You
    /// When for_each_player is true, corresponds to: TokenOwner$ Player (each player creates tokens)
    CreateToken {
        /// Player who will control the tokens (ignored if for_each_player is true)
        controller: PlayerId,
        /// Token script name (e.g., "c_a_food_sac" for Food token)
        token_script: String,
        /// Number of tokens to create
        amount: u8,
        /// If true, each player creates the tokens (TokenOwner$ Player)
        for_each_player: bool,
    },

    /// Create a token that's a copy of an existing permanent
    /// Example: Cackling Counterpart, Ember Island Production
    /// Corresponds to: DB$ CopyPermanent | ValidTgts$ Creature.YouCtrl | SetPower$ 4 | AddTypes$ Hero
    ///
    /// Creates a token with the same characteristics as the target permanent,
    /// optionally with modifications (different P/T, additional types, etc.)
    CopyPermanent {
        /// The permanent to copy
        target: CardId,
        /// Player who will control the token
        controller: PlayerId,
        /// If true, remove Legendary supertype from the copy
        non_legendary: bool,
        /// Override the copy's power (None = use original)
        set_power: Option<i32>,
        /// Override the copy's toughness (None = use original)
        set_toughness: Option<i32>,
        /// Types to add to the copy (e.g., ["Hero"], ["Coward"])
        add_types: Vec<String>,
        /// Number of copies to create (default 1)
        num_copies: u8,
        /// Target restriction from ValidTgts$ (e.g., Creature.YouCtrl, Creature.OppCtrl)
        restriction: TargetRestriction,
    },

    /// Clone effect: the SOURCE permanent enters the battlefield as a copy of
    /// another permanent on the battlefield (CR 707).
    ///
    /// Example: Copy Artifact — `DB$ Clone | Choices$ Artifact.Other | AddTypes$ Enchantment`.
    /// Unlike `CopyPermanent` (which creates a token copy of a *target*), this
    /// rewrites the copiable values (CR 707.2) of the source object itself,
    /// then layers the `add_types` card types on top (Copy Artifact stays an
    /// Enchantment in addition to the copied artifact's types).
    ///
    /// The controller of `source` chooses which permanent to copy from those
    /// matching `choices_filter`. If `optional` is set the controller is first
    /// asked whether to copy at all (Copy Artifact's "You may ..."). The choice
    /// is routed through the PlayerController at resolution time (network-safe);
    /// the placeholder `chosen` is filled in there.
    Clone {
        /// The permanent that becomes the copy (the Copy Artifact itself).
        /// Placeholder `CardId::new(0)` at parse; resolved to the source card.
        source: CardId,
        /// The permanent whose copiable values are copied. Placeholder
        /// `CardId::new(0)` at parse; chosen by the controller at resolution.
        chosen: CardId,
        /// Filter restricting which permanents may be copied (Choices$).
        choices_filter: TargetRestriction,
        /// Card types to add on top of the copied values (AddTypes$),
        /// e.g. [CardType::Enchantment] for Copy Artifact.
        add_types: smallvec::SmallVec<[crate::core::CardType; 1]>,
        /// If true, the controller may decline to copy at all (ETBReplacement
        /// `Optional` flag — "You may have CARDNAME enter as a copy ...").
        optional: bool,
    },

    /// Balance effect - equalizes a type of permanent/cards across all players
    /// Example: "Each player sacrifices creatures until all players control the same number"
    /// Corresponds to: SP$ Balance | Valid$ Creature/Land | Zone$ Battlefield/Hand
    ///
    /// The spell controller's card type and zone define what to balance.
    /// Each player must sacrifice/discard down to match the player with the fewest.
    ///
    /// SubAbility chaining: After this Balance effect resolves, the sub_ability (if any)
    /// is looked up in the card's SVars and executed. This enables Balance's full
    /// Land → Hand → Creature chain.
    Balance {
        /// What type of card to balance ("Creature", "Land", or empty for any permanent)
        card_type: String,
        /// Zone to balance ("Battlefield" or "Hand")
        zone: String,
        /// Optional SubAbility$ reference (SVar name to execute after this effect)
        sub_ability: Option<String>,
    },

    /// Set base power and toughness until end of turn
    /// Example: Flexible Waterbender - "This creature has base power and toughness 5/2 until end of turn"
    /// Corresponds to: AB$ Animate | Defined$ Self | Power$ 5 | Toughness$ 2
    /// Also: AB$ Animate | Defined$ Self | Power$ 4 | Keywords$ Trample
    ///
    /// This effect sets the creature's base P/T (not a modifier), which then has +1/+1 counters added on top.
    /// The effect lasts until end of turn.
    /// Power and Toughness are optional - None means "don't change".
    /// Keywords can be granted along with P/T changes.
    ///
    /// `types_added` / `subtypes_added` model the `Types$` parameter on
    /// `AB$ Animate` — e.g. Mishra's Factory's `Types$ Artifact,Creature,Assembly-Worker`.
    /// These are the *card* types and subtypes the source becomes for the
    /// duration of the effect (Mishra's Factory becomes Land + Artifact +
    /// Creature with the Assembly-Worker subtype). They're removed at end of
    /// turn cleanup along with the temp P/T.
    ///
    /// `remove_creature_subtypes` mirrors `RemoveCreatureTypes$ True`: any
    /// pre-existing creature subtypes on the source are stripped before the
    /// new ones are added. Used when a manland animates into a *different*
    /// creature type than its printed subtypes.
    SetBasePowerToughness {
        target: CardId,
        power: Option<i32>,
        toughness: Option<i32>,
        /// Keywords to grant (e.g., Trample from Keywords$ parameter)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
        /// Card types to add until end of turn (e.g. `Types$ Artifact,Creature`).
        /// Empty = don't change types.
        #[serde(default)]
        types_added: smallvec::SmallVec<[crate::core::CardType; 2]>,
        /// Subtypes to add until end of turn (e.g. `Assembly-Worker` from
        /// `Types$ Artifact,Creature,Assembly-Worker`).
        #[serde(default)]
        subtypes_added: smallvec::SmallVec<[crate::core::Subtype; 2]>,
        /// If true, strip any pre-existing creature subtypes before adding
        /// the new ones (`RemoveCreatureTypes$ True`).
        #[serde(default)]
        remove_creature_subtypes: bool,
    },

    /// Airbend: Exile a permanent and grant its owner permission to cast it for {2}.
    ///
    /// Avatar set mechanic (CR 701.65b). Effect:
    /// "Exile [target]. While it's exiled, its owner may cast it for {2} rather than its mana cost."
    ///
    /// Corresponds to: `DB$ Airbend | ValidTgts$ Creature`
    ///
    /// Implementation:
    /// 1. Exile the target permanent
    /// 2. Create a PersistentEffect (MayPlayFromExile) that grants cast permission
    /// 3. The effect is cleaned up when the card leaves exile or is cast
    ///
    /// Cards using this:
    /// - Aang, the Last Airbender: ETB airbends nonland permanent
    /// - Monk Gyatso: Triggered on targeting other creatures
    /// - Glider Staff: ETB airbend creature
    /// - Airbender Ascension: ETB airbend creature
    Airbend {
        /// The permanent to airbend (will be exiled)
        target: CardId,
    },

    /// Earthbend: Target land becomes a 0/0 creature with haste, put N +1/+1 counters.
    ///
    /// Avatar set mechanic (CR 701.65a). Effect:
    /// "Target land you control becomes a 0/0 creature with haste that's still a land.
    /// Put N +1/+1 counters on it. When it dies or is exiled, return it to the
    /// battlefield tapped."
    ///
    /// Corresponds to: `DB$ Earthbend | Num$ 8`
    ///
    /// Implementation:
    /// 1. Add Creature type to the land (permanently)
    /// 2. Set base power/toughness to 0/0
    /// 3. Add Haste keyword
    /// 4. Put N +1/+1 counters on it
    /// 5. Create a DelayedTrigger for return-to-battlefield on death/exile
    ///
    /// Cards using this:
    /// - Avatar Kyoshi, Earthbender: "earthbend 8, then untap that land"
    /// - Bumi, Unleashed: "earthbend 4"
    /// - Badgermole: "earthbend 2"
    Earthbend {
        /// The land to earthbend (becomes a creature)
        target: CardId,
        /// Number of +1/+1 counters to put on the land
        num_counters: u8,
    },

    /// Firebend: Add red mana to combat mana pool (lasts until end of combat).
    ///
    /// Avatar set mechanic. Effect: "Add N {R}. This mana lasts until end of combat."
    ///
    /// Corresponds to: `DB$ Mana | CombatMana$ True | Produced$ R | Amount$ N`
    /// or keyword `K:Firebending:N`
    ///
    /// Implementation:
    /// 1. Add N red mana to the player's combat_mana_pool
    /// 2. The combat mana is cleared at end of combat (in end_combat_step)
    ///
    /// Cards using this:
    /// - Firebending Student: "Firebending X, where X is this creature's power"
    /// - Azula, Ruthless Firebender: "Firebending 1"
    /// - Fire Nation Cadets: "Firebending 1"
    Firebend {
        /// The player who gets the mana
        controller: PlayerId,
        /// Amount of red mana to add
        amount: u8,
    },

    /// Grant "can't be blocked" until end of turn.
    ///
    /// Effect: Target creature can't be blocked this turn.
    ///
    /// Corresponds to: `AB$ Effect | StaticAbilities$ Unblockable | RememberObjects$ Targeted`
    /// (with SVar: `Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered`)
    ///
    /// Implementation:
    /// 1. Create a PersistentEffect (CantBeBlocked) for the target creature
    /// 2. The effect is cleaned up at end of turn
    ///
    /// Cards using this:
    /// - Deserter's Disciple: "Another target creature you control with power 2 or less can't be blocked this turn."
    GrantCantBeBlocked {
        /// The creature that can't be blocked
        target: CardId,
    },

    /// Regenerate: Add a regeneration shield to target permanent (CR 701.15a).
    /// "The next time [permanent] would be destroyed this turn, instead remove
    /// all damage marked on it and its controller taps it. If it's an attacking
    /// or blocking creature, remove it from combat."
    ///
    /// Most cards target Self (e.g., Drudge Skeletons: "{B}: Regenerate CARDNAME.")
    /// Some cards target other creatures (e.g., Zombie Master granting regeneration).
    ///
    /// Cards using this: Drudge Skeletons, Sedge Troll, Skeletal Wurm, etc. (246 cards)
    Regenerate {
        /// The permanent to add a regeneration shield to
        target: CardId,
    },

    /// Prevent damage: Create a damage prevention shield on a target (CR 615.1).
    /// "Prevent the next N damage that would be dealt to [target] this turn."
    ///
    /// The shield is stored on the target's `damage_prevention` field and is consumed
    /// when damage would be dealt, reducing or eliminating the damage. Multiple shields
    /// stack additively. Cleared during the cleanup step.
    ///
    /// Cards using this: Militant Monk, Master Healer, Eiganjo Castle, etc. (81 cards)
    PreventDamage {
        /// The target to protect - can be a creature (CardId) or player
        target: TargetRef,
        /// Amount of damage to prevent
        amount: i32,
    },

    /// Create a *source-filtered* damage-prevention replacement effect on a
    /// player (CR 615.1, 615.6) — the general construct behind the Circles of
    /// Protection. Unlike [`Effect::PreventDamage`] (a blanket amount counter),
    /// this shield only prevents damage from a matching *source* (e.g. a chosen
    /// red source for Circle of Protection: Red) and lasts until end of turn.
    ///
    /// `protected` is the player the shield guards (resolved to the ability's
    /// controller at activation). `color` is the source-color filter; the
    /// chosen source `CardId` is filled in from the activated ability's target
    /// at resolution (placeholder until then). See [`crate::core::prevention`].
    PreventDamageFromSource {
        /// Player protected by the shield (CR 615 affected player).
        protected: PlayerId,
        /// Required color of the chosen source (Red for Circle of Protection: Red).
        color: Color,
        /// The chosen source permanent/spell. Placeholder until the ability's
        /// target is resolved at activation/resolution.
        source: CardId,
    },

    /// Modal spell choice - player selects modes from multiple predefined effects.
    ///
    /// Example: Heartless Act - "Choose one — Destroy target creature with no counters on it;
    ///                           or Remove up to three counters from target creature."
    /// Corresponds to: A:SP$ Charm | Choices$ Destroy,Remove
    ///
    /// During resolution, the controller is prompted to choose modes, then the selected
    /// modes' effects are resolved in order. Each mode has its own targeting requirements.
    ///
    /// Cards using this:
    /// - Heartless Act, Abzan Charm, Cryptic Command, Commands, etc.
    ModalChoice {
        /// The available modes the player can choose from.
        /// Each is a tuple of (effect, description, SVar name).
        /// The SVar name is used to look up targeting info.
        modes: SmallVec<[ModalMode; 4]>,

        /// Number of modes to select (e.g., 1 for "Choose one", 2 for "Choose two")
        num_to_choose: u8,

        /// Minimum number of modes to select (default = num_to_choose)
        min_to_choose: u8,

        /// Whether the same mode can be chosen multiple times
        can_repeat_modes: bool,
    },

    /// Dig: Exile top N cards from opponents' libraries.
    ///
    /// Effect: Look at the top N cards of each opponent's library, exile some/all.
    ///
    /// Corresponds to: `AB$ Dig | DigNum$ N | ChangeNum$ All | Defined$ Opponent | DestinationZone$ Exile`
    ///
    /// Implementation:
    /// 1. For each opponent, look at top N cards of their library
    /// 2. Move ChangeNum cards to the destination zone (Exile)
    /// 3. Optionally grant "may play" permission (via MayPlay$ True)
    ///
    /// Cards using this:
    /// - Fire Lord Ozai: "{6}: Exile the top card of each opponent's library. Until end of turn,
    ///   you may play one of those cards without paying its mana cost." (target_self=false)
    /// - Seismic Sense: "Look at top X cards of your library. You may reveal a creature or land
    ///   and put it into your hand. Put the rest on bottom in random order." (target_self=true)
    ///
    /// TODO(mtg-213): Implement "may play without paying mana cost" via persistent effects
    Dig {
        /// Number of cards to look at from each library (DigNum$)
        dig_count: u8,
        /// Number of cards to change zones (ChangeNum$ - "All" means all)
        change_count: u8,
        /// Whether ALL cards should be moved (ChangeNum$ All)
        change_all: bool,
        /// Destination zone for selected cards (Hand for most Dig, Exile for Fire Lord Ozai)
        destination: crate::zones::Zone,
        /// Destination zone for non-selected cards (DestinationZone2$, default Library bottom)
        rest_destination: crate::zones::Zone,
        /// Whether to grant "may play" permission for exiled cards
        may_play: bool,
        /// Whether "may play" costs no mana
        may_play_without_mana_cost: bool,
        /// Whether to dig from own library (true, default) or opponents' libraries (false)
        /// Parsed from Defined$ parameter: "You"/"" = true, "Opponent" = false
        target_self: bool,
        /// Whether selecting a card is optional (Optional$ True)
        optional: bool,
        /// Whether to put non-selected cards on bottom of library in random order
        /// (RestRandomOrder$ True)
        rest_random: bool,
        /// Whether to reveal dug cards to all players (Reveal$ True)
        reveal: bool,
        /// Filter for which cards are valid to select (ChangeValid$)
        /// Comma-separated type names like "Creature,Land" or "Artifact"
        /// Empty string means any card is valid for selection
        change_valid: SmallVec<[DigFilter; 2]>,
    },

    /// Create a delayed trigger that fires when a condition is met.
    ///
    /// Corresponds to: `SP$ DelayedTrigger | Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | Execute$ TrigEffect`
    ///
    /// Example: Fatal Fissure - "Choose target creature. When that creature dies this turn, you earthbend 4."
    ///
    /// Implementation:
    /// 1. Remember the targeted card
    /// 2. Create a DelayedTrigger with the specified condition (e.g., ZoneChange from Battlefield to Graveyard)
    /// 3. When the condition is met, execute the specified effect
    /// 4. If ThisTurn$ True, the trigger expires at end of turn
    ///
    /// Cards using this:
    /// - Fatal Fissure: Delayed trigger on creature death -> earthbend 4
    CreateDelayedTrigger {
        /// The card to track (target of the spell)
        tracked_card: CardId,
        /// The condition that fires the trigger
        condition: crate::core::DelayedTriggerCondition,
        /// The effect to execute when triggered
        effect: Box<Effect>,
        /// When the trigger expires (usually EndOfTurn for ThisTurn$ True)
        expiry: Option<crate::core::DelayedTriggerExpiry>,
    },

    /// Copy a spell on the stack
    ///
    /// Corresponds to: `DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | MayChooseTarget$ True`
    ///
    /// This effect is used in two contexts:
    /// 1. As Execute$ target of a DB$ DelayedTrigger with Mode$ SpellCast
    ///    (e.g., "When you cast a Lesson spell, copy it")
    /// 2. As a SubAbility chained to another effect
    ///    (e.g., Chain Lightning: "Then that player may pay RR. If they do, copy this spell")
    ///
    /// Cards using this:
    /// - Jeong Jeong: "copy it and you may choose new targets for the copy"
    /// - Chain Lightning: "that player may copy this spell and choose new targets"
    CopySpellAbility {
        /// Whether the player may choose new targets for the copy
        may_choose_targets: bool,
        /// What spell to copy: "Parent" (the current spell), "TriggeredSpellAbility" (triggering spell)
        /// Defaults to "Parent" for SubAbility use, "TriggeredSpellAbility" for delayed triggers
        defined_source: CopySpellSource,
        /// Who controls the copy (resolved player ID or reference like "TargetedOrController")
        controller: Option<String>,
    },

    /// Conditionally execute a sub-effect based on remembered cards
    ///
    /// Corresponds to: `DB$ ImmediateTrigger | ConditionDefined$ Remembered | ConditionPresent$ Card.nonLand | ConditionCompare$ GE1 | Execute$ TrigPutCounter`
    ///
    /// This effect checks if the remembered cards (stored by previous effects like DiscardCards
    /// with RememberDiscarded$ True) meet a condition, and if so, executes the sub-effect.
    ///
    /// Cards using this:
    /// - Teo, Spirited Glider: "When you discard a nonland card this way, put a +1/+1 counter on target creature"
    ImmediateTrigger {
        /// Condition to check against remembered cards
        condition: ImmediateTriggerCondition,
        /// Effect to execute if condition is met (SVar name resolved during parsing)
        sub_effects: Vec<Effect>,
    },

    /// Clear remembered cards storage
    ///
    /// Corresponds to: `DB$ Cleanup | ClearRemembered$ True`
    ///
    /// This effect clears the game.remembered_cards storage after ImmediateTrigger has checked it.
    ClearRemembered,

    /// Choose a color (WUBRG) and store it on the source card.
    ///
    /// Corresponds to: `AB$ ChooseColor | Cost$ ... | Defined$ You`
    ///
    /// The chosen color is stored in `Card::chosen_color` and can be referenced by
    /// subsequent abilities via "ChosenColor" (e.g., protection from chosen color,
    /// "Colors$ ChosenColor" in Animate effects, "Card.ChosenColor" in filters).
    ///
    /// Cards using this:
    /// - Caldera Kavu, Spiritmonger: "G: CARDNAME becomes the color of your choice until EOT"
    /// - Crosis, the Purger: Triggered ability choosing a color for discard
    /// - Skrelv, Defector Mite: Choose color for hexproof/blocking restrictions
    /// - Govern the Guildless: Choose colors for Animate effect
    ChooseColor {
        /// The player making the choice (placeholder resolved to card_owner)
        player: PlayerId,
        /// The card to store the chosen color on (usually the source card)
        source: CardId,
    },

    /// Wraps an effect with an UnlessCost condition
    ///
    /// The wrapped effect only executes if the payer pays (or doesn't pay, depending on switched flag).
    ///
    /// # Examples
    ///
    /// **Counter unless pays**: (switched=false) Effect resolves if payer does NOT pay
    /// ```text
    /// DB$ Counter | UnlessCost$ 2 | UnlessPayer$ TargetedController
    /// ```
    ///
    /// **You may discard**: (switched=true) Effect resolves if payer DOES pay
    /// ```text
    /// SP$ Draw | NumCards$ 2 | UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True
    /// ```
    UnlessCostWrapper {
        /// The effect to conditionally execute
        inner_effect: Box<Effect>,
        /// The UnlessCost condition
        unless_cost: UnlessCost,
    },

    /// Add an extra combat phase after the current one
    /// Example: "After this main phase, there is an additional combat phase" (Raphael Tag Team Tough)
    /// Corresponds to: DB$ AddPhase | PhaseType$ Combat
    AddPhase {
        /// Number of extra combat phases to add
        count: u8,
    },

    /// Placeholder for a recognized but unimplemented effect
    /// Produced instead of silently dropping the effect, so that spell resolution
    /// can warn/error instead of silently no-op'ing.
    Unimplemented {
        /// The API type name that was not implemented
        api_type: String,
    },
}

/// Condition for ImmediateTrigger effect
///
/// Used to check remembered cards against criteria before executing a sub-effect.
/// Corresponds to: `ConditionDefined$ Remembered | ConditionPresent$ Card.nonLand | ConditionCompare$ GE1`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImmediateTriggerCondition {
    /// At least one remembered card is a nonland card
    /// Corresponds to: ConditionPresent$ Card.nonLand | ConditionCompare$ GE1
    RememberedNonLand,
    /// At least one remembered card exists (any card)
    /// Corresponds to: ConditionCompare$ GE1
    AnyRemembered,
}

/// A counter-count condition evaluated against a single card (usually the
/// source card via `Defined$ Self`).
///
/// Encodes filters of the shape `counters_<CMP><N>_<TYPE>`, e.g.
/// `counters_GE1_SCREAM` (>= 1 scream counter) or `counters_EQ0_SCREAM`
/// (no scream counters). Used both by triggered-ability intervening-if
/// conditions (`IsPresent$ Card.Self+counters_GE1_SCREAM`) and by
/// `Effect::ConditionalSelfCounter` (`ConditionPresent$ Card.counters_EQ0_SCREAM`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SelfCounterCondition {
    /// The counter type being counted.
    pub counter_type: crate::core::CounterType,
    /// Comparison applied to the count of that counter type on the card.
    pub compare: CompareCondition,
}

impl SelfCounterCondition {
    /// Parse a `counters_<CMP><N>_<TYPE>` clause (the part after `counters_`).
    ///
    /// Examples of the full clause: `counters_GE1_SCREAM`, `counters_EQ0_SCREAM`.
    /// `clause` here is the substring after the leading `counters_`, e.g.
    /// `GE1_SCREAM`.
    pub fn parse_clause(clause: &str) -> Option<Self> {
        // Split off the trailing _<TYPE>; the compare token is everything before
        // the last underscore (compare tokens never contain underscores).
        let (cmp_str, type_str) = clause.rsplit_once('_')?;
        let compare = CompareCondition::parse(cmp_str)?;
        let counter_type = crate::core::CounterType::parse(type_str)?;
        Some(SelfCounterCondition { counter_type, compare })
    }

    /// Evaluate this condition against a card's current counter count.
    pub fn evaluate(&self, count: u8) -> bool {
        self.compare.evaluate(i32::from(count))
    }
}

/// Source of the spell to copy for CopySpellAbility
///
/// Corresponds to the `Defined$` parameter in DB$ CopySpellAbility
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CopySpellSource {
    /// Copy the parent spell (the current spell on the stack that has this as SubAbility)
    /// Used by Chain Lightning: "copy this spell"
    /// Corresponds to: Defined$ Parent
    #[default]
    Parent,
    /// Copy the spell that triggered this effect
    /// Used by Jeong Jeong: "copy it" (the triggering spell)
    /// Corresponds to: Defined$ TriggeredSpellAbility
    TriggeredSpellAbility,
}

/// Categorization of effects for targeting purposes.
///
/// Used by targeting.rs to determine what targets need to be collected for spells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTargetCategory {
    /// Effect targets players or has no targeting requirements.
    /// Examples: DrawCards, GainLife, Mill, Scry, CreateToken
    NoTargetNeeded,

    /// Effect requires a creature or permanent target.
    /// Examples: DestroyPermanent, TapPermanent, PumpCreature, ExilePermanent
    RequiresTarget,

    /// Effect uses filters to affect multiple permanents (no explicit targeting).
    /// Examples: PumpAllCreatures
    UsesFilter,

    /// Effect contains inner effects with their own targeting (modal spells).
    /// Examples: ModalChoice
    HasInnerTargeting,
}

impl Effect {
    /// Returns the targeting category for this effect.
    ///
    /// This is used to avoid duplicating effect categorization across targeting.rs.
    /// When a new Effect variant is added, this method must be updated.
    pub fn target_category(&self) -> EffectTargetCategory {
        match self {
            // Effects targeting players or with no target
            Effect::DrawCards { .. }
            | Effect::DrawCardsXPaid { .. }
            | Effect::Loot { .. }
            | Effect::DiscardCards { .. }
            | Effect::DiscardCardsXPaid { .. }
            | Effect::GainLife { .. }
            | Effect::GainLifeDynamic { .. }
            | Effect::LoseLife { .. }
            | Effect::ForceSacrifice { .. }
            | Effect::SetLife { .. }
            | Effect::Mill { .. }
            | Effect::DrainMana { .. }
            | Effect::Scry { .. }
            | Effect::Surveil { .. }
            | Effect::AddMana { .. }
            | Effect::Balance { .. }
            | Effect::CreateToken { .. }
            | Effect::Dig { .. }
            | Effect::SearchLibrary { .. }
            | Effect::Firebend { .. }
            | Effect::CopySpellAbility { .. }
            | Effect::ImmediateTrigger { .. }
            | Effect::ClearRemembered
            | Effect::AddTurn { .. }
            | Effect::AddPhase { .. }
            | Effect::ChooseColor { .. }
            | Effect::Proliferate
            // Phase-trigger "deal damage to the active/triggered player" — the
            // target player is resolved at trigger time (no cast-time target).
            | Effect::DealDamageToTriggeredPlayer { .. }
            | Effect::SelfExileFromStack { .. }
            // Self-zone-move and conditional-self wrappers operate on the source
            // card (Defined$ Self) — no cast-time target collection needed.
            | Effect::MoveSelfBetweenZones { .. }
            | Effect::ConditionalSelfCounter { .. }
            // Clone chooses which permanent to copy at resolution time (ETB
            // replacement), routed through the controller — there is no
            // cast-time target on the Copy Artifact spell itself.
            | Effect::Clone { .. }
            | Effect::Unimplemented { .. } => EffectTargetCategory::NoTargetNeeded,

            // Effects using filters (affect multiple permanents)
            Effect::PumpAllCreatures { .. }
            | Effect::AnimateAll { .. }
            | Effect::DestroyAll { .. }
            | Effect::SacrificeAll { .. }
            | Effect::DamageAll { .. }
            | Effect::TapAll { .. }
            | Effect::UntapAll { .. }
            | Effect::PutCounterAll { .. }
            | Effect::ChangeZoneAll { .. } => EffectTargetCategory::UsesFilter,

            // Modal spells have inner targeting
            Effect::ModalChoice { .. } => EffectTargetCategory::HasInnerTargeting,

            // UnlessCost wrapper delegates to inner effect's category
            Effect::UnlessCostWrapper { inner_effect, .. } => inner_effect.target_category(),

            // Effects requiring creature/permanent/spell targets
            Effect::DealDamage { .. }
            | Effect::DealDamageXPaid { .. }
            | Effect::EachDamage { .. }
            | Effect::DestroyPermanent { .. }
            | Effect::GainControl { .. }
            | Effect::Fight { .. }
            | Effect::TapPermanent { .. }
            | Effect::UntapPermanent { .. }
            | Effect::TapOrUntapPermanent { .. }
            | Effect::PumpCreature { .. }
            | Effect::DebuffCreature { .. }
            | Effect::CounterSpell { .. }
            | Effect::PutCounter { .. }
            | Effect::MultiplyCounter { .. }
            | Effect::RemoveCounter { .. }
            | Effect::ExilePermanent { .. }
            | Effect::AttachEquipment { .. }
            | Effect::CopyPermanent { .. }
            | Effect::SetBasePowerToughness { .. }
            | Effect::Airbend { .. }
            | Effect::Earthbend { .. }
            | Effect::GrantCantBeBlocked { .. }
            | Effect::Regenerate { .. }
            | Effect::PreventDamage { .. }
            | Effect::PreventDamageFromSource { .. }
            | Effect::CreateDelayedTrigger { .. }
            | Effect::PumpCreatureVariable { .. } => EffectTargetCategory::RequiresTarget,
        }
    }

    /// Returns true if this effect needs no explicit targeting (targets players, uses filters, etc.)
    ///
    /// This is a convenience helper combining NoTargetNeeded and UsesFilter categories.
    pub fn needs_no_creature_target(&self) -> bool {
        matches!(
            self.target_category(),
            EffectTargetCategory::NoTargetNeeded | EffectTargetCategory::UsesFilter
        )
    }
}

/// A single mode in a modal spell.
///
/// Contains the effect to execute and metadata for display/targeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalMode {
    /// The effect to execute when this mode is chosen
    pub effect: Box<Effect>,
    /// Human-readable description (from SpellDescription$)
    pub description: String,
    /// SVar name for this mode (e.g., "DBDestroy") - used for targeting lookup
    pub svar_name: String,
}

/// Events that can trigger abilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerEvent {
    /// When a card enters the battlefield
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self
    EntersBattlefield,

    /// When a card leaves the battlefield
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Any | ValidCard$ Card.Self
    LeavesBattlefield,

    /// At the beginning of upkeep
    /// Corresponds to: T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You
    BeginningOfUpkeep,

    /// At the beginning of the draw step
    /// Corresponds to: T:Mode$ Phase | Phase$ Draw | ValidPlayer$ You
    /// Example: Grafted Skullcap / Sylvan Library / Yawgmoth's Bargain —
    /// "At the beginning of your draw step, draw an additional card."
    /// Fires from the battlefield after the active player's mandatory draw.
    BeginningOfDraw,

    /// At the beginning of end step
    /// Corresponds to: T:Mode$ Phase | Phase$ EndOfTurn | ValidPlayer$ You
    BeginningOfEndStep,

    /// At the beginning of combat
    /// Corresponds to: T:Mode$ Phase | Phase$ BeginCombat | ValidPlayer$ You
    BeginningOfCombat,

    /// When a spell is cast
    /// Corresponds to: T:Mode$ SpellCast | ValidCard$ ...
    SpellCast,

    /// When a creature attacks
    /// Corresponds to: T:Mode$ Attacks | ValidCard$ Card.Self
    Attacks,

    /// When a creature blocks
    /// Corresponds to: T:Mode$ Blocks | ValidCard$ Card.Self
    Blocks,

    /// When a creature deals combat damage
    /// Corresponds to: T:Mode$ DamageDone | ValidSource$ Card.Self | CombatDamage$ True
    DealsCombatDamage,

    /// When a permanent is sacrificed
    /// Corresponds to: T:Mode$ Sacrificed | ValidCard$ Permanent.Other | ValidPlayer$ You
    Sacrificed,

    /// When a card is drawn
    /// Corresponds to: T:Mode$ Drawn | ValidCard$ Card.YouCtrl | Number$ 2
    /// The draw_number field in Trigger specifies which draw triggers (e.g., 2 = second card)
    CardDrawn,

    /// When a permanent becomes tapped
    /// Corresponds to: T:Mode$ Taps | ValidCard$ Card.Self
    /// Example: "Whenever CARDNAME becomes tapped, draw a card, then discard a card."
    Taps,

    /// When one or more creatures attack (batch trigger, fires once per declare attackers step)
    /// Corresponds to: T:Mode$ AttackersDeclared | AttackingPlayer$ You | ValidAttackers$ Creature.withFlying
    /// Example: "Whenever one or more creatures you control with flying attack, draw a card."
    AttackersDeclared,

    /// When a creature equipped by this Equipment dies
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.EquippedBy
    /// Example: Skullclamp - "Whenever equipped creature dies, draw two cards."
    EquippedCreatureDies,

    /// When a creature dealt damage by this card this turn dies.
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.DamagedBy | TriggerZones$ Battlefield
    /// Example: Sengir Vampire — "Whenever a creature dealt damage by Sengir
    /// Vampire this turn dies, put a +1/+1 counter on Sengir Vampire."
    /// Fires from the trigger source (Sengir) when ANY creature in the
    /// dying card's `damaged_by_this_turn` list contains the trigger source's
    /// CardId.
    DamagedCreatureDies,
}

/// A triggered ability that executes when an event occurs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    /// The event that triggers this ability
    pub event: TriggerEvent,

    /// The effects to execute when triggered
    pub effects: Vec<Effect>,

    /// Description of the trigger (for logging)
    pub description: String,

    /// If true, this trigger only fires when the source card itself triggers the event
    /// (e.g., "When this creature enters" only fires for this specific creature)
    /// If false, triggers for any card matching the event (e.g., "When any creature enters")
    pub trigger_self_only: bool,

    /// If true, the player may choose whether to use this triggered ability
    /// (e.g., "you may sacrifice a creature" - player can decline)
    /// If false, the trigger is mandatory
    pub optional: bool,

    /// Cost that must be paid to execute the trigger effects (for optional triggers)
    /// e.g., sacrificing a permanent, paying life, paying mana
    /// If None, the trigger has no additional cost beyond being optional
    pub cost: Option<super::Cost>,

    /// For CardDrawn triggers: which draw number triggers this (e.g., 2 = "second card drawn")
    /// None means every card drawn triggers it
    pub draw_number: Option<u8>,

    /// For CardDrawn triggers: true = triggers on controller's draws, false = opponent's draws
    pub triggers_on_controller_draw: bool,

    // =========================================================================
    // STRUCTURED FILTER FLAGS - replacing string markers in description
    // These provide compile-time checked filtering instead of runtime string parsing
    // =========================================================================
    /// When true, trigger only fires if event source is DIFFERENT from trigger source
    /// Replaces "[other]" marker in description
    /// Example: "Whenever you sacrifice ANOTHER permanent" (Pirate Peddlers)
    #[serde(default)]
    pub requires_other: bool,

    /// When true, trigger only fires if event source is a Land controlled by trigger controller
    /// Replaces "[landfall]" marker in description
    /// Example: Landfall triggers like "Whenever a land enters under your control"
    #[serde(default)]
    pub requires_landfall: bool,

    /// When true, trigger only fires on controller's turn
    /// Replaces "[controller_only]" marker in description
    /// Example: Upkeep triggers that only fire on your own upkeep
    #[serde(default)]
    pub controller_turn_only: bool,

    /// When true, trigger only fires if event source is NOT a creature
    /// Replaces "[noncreature]" marker in description
    /// Example: "Whenever you cast a noncreature spell"
    #[serde(default)]
    pub requires_noncreature: bool,

    /// For AttackersDeclared triggers: keyword required on attacking creatures
    /// Corresponds to ValidAttackers$ Creature.withFlying (or other keywords)
    /// None means any attacking creature triggers it
    #[serde(default)]
    pub valid_attackers_keyword: Option<crate::core::Keyword>,

    /// Zones in which the trigger source must reside for the trigger to fire.
    ///
    /// Corresponds to `TriggerZones$`. Defaults to `[Battlefield]` (the usual
    /// case). All Hallow's Eve uses `TriggerZones$ Exile` so its upkeep trigger
    /// fires while the card sits in exile (CR 603.6e — abilities that function
    /// in a zone other than the battlefield). Empty means "any zone".
    #[serde(default)]
    pub trigger_zones: smallvec::SmallVec<[crate::zones::Zone; 2]>,

    /// Intervening-if condition: the source card must satisfy this counter
    /// condition (in `present_zone`) for the trigger to fire (CR 603.4).
    ///
    /// Corresponds to `IsPresent$ Card.Self+counters_<CMP><N>_<TYPE>` combined
    /// with `PresentZone$`. All Hallow's Eve: `IsPresent$
    /// Card.Self+counters_GE1_SCREAM | PresentZone$ Exile`. None means no
    /// intervening-if check.
    #[serde(default)]
    pub present_self_condition: Option<SelfCounterCondition>,
}

impl Trigger {
    /// Create a new trigger with trigger_self_only defaulting to true
    /// Most ETB/LTB triggers only fire for the card itself
    pub fn new(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,           // Default: only fire for this card
            optional: false,                   // Default: mandatory trigger
            cost: None,                        // Default: no additional cost
            draw_number: None,                 // Default: trigger on any draw
            triggers_on_controller_draw: true, // Default: trigger on controller's draws
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            requires_noncreature: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
        }
    }

    /// Create a new trigger that fires for any card matching the event
    pub fn new_any(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: false,
            optional: false,
            cost: None,
            draw_number: None,
            triggers_on_controller_draw: true,
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            requires_noncreature: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
        }
    }

    /// Create an optional trigger with a cost
    /// Used for "you may [cost]. If you do, [effect]" abilities
    pub fn new_optional_with_cost(
        event: TriggerEvent,
        effects: Vec<Effect>,
        description: String,
        cost: super::Cost,
    ) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,
            optional: true,
            cost: Some(cost),
            draw_number: None,
            triggers_on_controller_draw: true,
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            requires_noncreature: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
        }
    }

    /// Create an optional trigger without a cost
    /// Used for "you may [effect]" abilities
    pub fn new_optional(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,
            optional: true,
            cost: None,
            draw_number: None,
            triggers_on_controller_draw: true,
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            requires_noncreature: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
        }
    }
}

/// Static ability that creates continuous effects
///
/// ## CR 613: Interaction of Continuous Effects
///
/// Static abilities create continuous effects that modify characteristics
/// of game objects. They are always "on" and don't use the stack.
///
/// Example from Spider-Suit:
/// ```text
/// S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2
/// ```
///
/// This creates a continuous effect in Layer 7c (MODIFYPT) that gives
/// the equipped creature +2/+2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticAbility {
    /// Continuous effect that modifies power/toughness
    ///
    /// Corresponds to: `S:Mode$ Continuous | AddPower$ X | AddToughness$ Y`
    /// Applied in CR 613 Layer 7c (MODIFYPT)
    ModifyPT {
        /// Selector for which cards are affected
        /// Example: "Creature.EquippedBy" = creature equipped by this Equipment
        /// Example: "Creature.YouCtrl" = creatures you control
        affected: AffectedSelector,

        /// Power bonus (can be negative)
        power: i32,

        /// Toughness bonus (can be negative)
        toughness: i32,

        /// Description for logging
        description: String,

        /// Optional condition for when this ability is active.
        /// None = always active. Example: Sedge Troll's +1/+1 is conditional
        /// on `IsPresent$ Swamp.YouCtrl` (see [`StaticCondition::ControlsPresent`]).
        condition: Option<StaticCondition>,
    },

    /// Continuous effect that grants a keyword ability
    ///
    /// Corresponds to: `S:Mode$ Continuous | AddKeyword$ Keyword`
    /// Applied in CR 613 Layer 6 (Abilities)
    ///
    /// Example: Spider-Punk grants Riot to other Spiders:
    /// `S:Mode$ Continuous | Affected$ Spider.Other+YouCtrl | AddKeyword$ Riot`
    GrantKeyword {
        /// Selector for which cards are affected
        affected: AffectedSelector,

        /// The keyword to grant
        keyword: crate::core::Keyword,

        /// Description for logging
        description: String,

        /// Optional condition for when this ability is active
        /// None = always active, Some(PlayerTurn) = only during controller's turn
        condition: Option<StaticCondition>,
    },

    /// Cost reduction static ability
    ///
    /// Corresponds to: `S:Mode$ ReduceCost | ValidCard$ X | Type$ Spell | Amount$ N`
    ///
    /// Example from Gran-Gran:
    /// `S:Mode$ ReduceCost | ValidCard$ Card.nonCreature | Type$ Spell | Activator$ You |
    ///  Amount$ 1 | IsPresent$ Lesson.YouOwn | PresentZone$ Graveyard | PresentCompare$ GE3`
    ///
    /// This reduces the cost of non-creature spells by {1} when there are 3+ Lessons
    /// in the controller's graveyard.
    ReduceCost {
        /// Which cards get the cost reduction
        /// Examples: "Card.nonCreature" = non-creature cards, "Card.Self" = only this card
        valid_card: CostReductionTarget,

        /// Amount of generic mana to reduce
        amount: u8,

        /// Condition for when the reduction applies (presence checks)
        condition: Option<CostReductionCondition>,

        /// Description for logging
        description: String,
    },

    /// Cost increase static ability
    ///
    /// Corresponds to: `S:Mode$ RaiseCost | ValidCard$ X | Type$ Spell | Amount$ N`
    /// or: `S:Mode$ RaiseCost | ValidCard$ Card.Self | Cost$ Sac<X/Land>`
    ///
    /// Example from Thalia, Guardian of Thraben:
    /// `S:Mode$ RaiseCost | ValidCard$ Card.nonCreature | Type$ Spell | Amount$ 1`
    ///
    /// Example from Tectonic Split:
    /// `S:Mode$ RaiseCost | ValidCard$ Card.Self | Type$ Spell | Cost$ Sac<X/Land/land(s)>`
    /// with `SVar:X:Count$Valid Land.YouCtrl/HalfUp`
    RaiseCost {
        /// Which cards get the cost increase
        /// Examples: "Card.nonCreature" = non-creature cards, "Card.Self" = only this card
        valid_card: CostReductionTarget,

        /// The additional cost to add
        raised_cost: RaisedCost,

        /// Description for logging
        description: String,
    },

    /// Grant an activated ability to affected permanents
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ Land.YouCtrl | AddAbility$ AnyMana`
    /// with `SVar:AnyMana:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 3`
    ///
    /// Example from Tectonic Split:
    /// Grants lands "{T}: Add three mana of any one color."
    ///
    /// Example from Chromatic Lantern:
    /// Grants lands "{T}: Add one mana of any color."
    GrantAbility {
        /// Selector for which cards are affected
        /// Example: "Land.YouCtrl" = lands you control
        affected: AffectedSelector,

        /// The ability to grant (stored as parsed ActivatedAbility)
        ability: ActivatedAbility,

        /// Description for logging
        description: String,
    },

    /// Continuous control-changing effect (CR 613.2 / layer 2).
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ Card.EnchantedBy | GainControl$ You`
    /// as printed on control-stealing Auras (Control Magic, Mind Control, Persuasion,
    /// Enslave, Confiscate, ...). The source Aura's controller gains control of the
    /// affected permanent for as long as the source has the static ability and the
    /// affected permanent is the source's attach target.
    ///
    /// Unlike `Effect::GainControl` (the one-shot `AB$ GainControl` of Threaten /
    /// Aladdin), this is a *continuous* effect that is re-derived every state-based
    /// check, so control reverts automatically the moment the Aura leaves the
    /// battlefield (destroyed, bounced, or the host dies) — no explicit "lose
    /// control at end of turn" bookkeeping is required.
    GainControl {
        /// Selector for which permanent is affected (typically `Card.EnchantedBy`).
        affected: AffectedSelector,

        /// Description for logging.
        description: String,
    },
}

/// Target selector for cost reduction abilities
///
/// Specifies which cards get their costs reduced by a ReduceCost static ability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostReductionTarget {
    /// Non-creature spells
    /// Corresponds to: `ValidCard$ Card.nonCreature`
    NonCreature,

    /// All spells (no restriction)
    /// Corresponds to: `ValidCard$ Card` or no ValidCard parameter
    AllSpells,

    /// Creature spells only
    /// Corresponds to: `ValidCard$ Creature`
    Creature,

    /// Spells of a specific subtype
    /// Corresponds to: `ValidCard$ Dragon`, `ValidCard$ Spirit`, etc.
    Subtype(crate::core::Subtype),

    /// Spells of a specific color (CR 105.1 / CR 202.2)
    /// Corresponds to: `ValidCard$ Card.White`, `ValidCard$ Card.Blue`, etc.
    /// Used by colour-hate enchantments — Gloom (white), Karma (swamps),
    /// CoP-style hosers — where the effect targets any spell that is the
    /// named colour regardless of controller.
    Color(crate::core::Color),
}

/// Condition for when a cost reduction applies
///
/// Used for abilities like Gran-Gran's "as long as there are three or more
/// Lesson cards in your graveyard"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostReductionCondition {
    /// What cards must be present (e.g., "Lesson.YouOwn")
    pub is_present: String,

    /// Which zone to check (e.g., Graveyard)
    pub present_zone: crate::zones::Zone,

    /// Minimum count required (from PresentCompare$ GE3 -> 3)
    pub min_count: u8,
}

/// Represents what additional cost is raised by a RaiseCost ability
///
/// Can be either a mana cost increase or a non-mana cost like sacrifice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaisedCost {
    /// Increase generic mana cost by this amount
    /// Corresponds to: `Amount$ N` where N is a number
    Mana(u8),

    /// Sacrifice N permanents of the given type
    /// Corresponds to: `Cost$ Sac<N/Type>` or `Cost$ Sac<X/Type>`
    Sacrifice {
        /// The amount to sacrifice (fixed or variable)
        amount: RaisedCostAmount,
        /// The type of permanent to sacrifice (e.g., "Land", "Creature")
        valid_type: String,
    },
}

/// Amount for a raised cost - can be fixed or variable (X)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaisedCostAmount {
    /// A fixed amount (e.g., Sac<1/Land>)
    Fixed(u8),
    /// A variable amount referencing an SVar (e.g., Sac<X/Land> with SVar:X:...)
    Variable(String),
}

/// Represents the type of cost for an UnlessCost condition
///
/// These correspond to the `UnlessCost$` parameter in card scripts.
/// Common patterns:
/// - `UnlessCost$ 2` - pay {2} generic mana
/// - `UnlessCost$ Discard<1/Card>` - discard 1 card
/// - `UnlessCost$ Sac<1/Creature>` - sacrifice 1 creature
/// - `UnlessCost$ PayLife<3>` - pay 3 life
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnlessCostType {
    /// Pay mana cost (e.g., "2", "1U", "X")
    Mana(crate::core::ManaCost),
    /// Discard N cards of the given type
    /// Format: `Discard<N/Type>` (e.g., Discard<1/Card>)
    Discard { count: u8, card_type: String },
    /// Sacrifice N permanents of the given type
    /// Format: `Sac<N/Type>` (e.g., Sac<1/Creature>)
    Sacrifice { count: u8, valid_type: String },
    /// Pay N life
    /// Format: `PayLife<N>`
    PayLife(u8),
    /// Reveal N cards of the given type from hand
    /// Format: `Reveal<N/Type>` (e.g., Reveal<1/Giant>)
    Reveal { count: u8, card_type: String },
}

/// Represents an UnlessCost condition that wraps an effect
///
/// In MTG Forge card scripts, this corresponds to:
/// - `UnlessCost$ <cost>` - the cost to pay
/// - `UnlessPayer$ <player>` - who pays (defaults to TargetedController)
/// - `UnlessSwitched$ True` - if present, effect executes when paid (otherwise when NOT paid)
///
/// # Examples
///
/// **Counter unless pays**: Effect executes when cost is NOT paid
/// ```text
/// DB$ Counter | UnlessCost$ 2 | UnlessPayer$ TargetedController
/// ```
///
/// **You may discard to draw**: Effect executes when cost IS paid (switched)
/// ```text
/// SP$ Draw | NumCards$ 2 | UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlessCost {
    /// The cost to pay
    pub cost: UnlessCostType,
    /// Who pays the cost (resolved player reference)
    /// Common values: "You", "TargetedController", "Player"
    pub payer: String,
    /// If true, effect executes when paid; if false, when not paid
    pub switched: bool,
}

impl UnlessCost {
    /// Create a new UnlessCost
    pub fn new(cost: UnlessCostType, payer: &str, switched: bool) -> Self {
        Self {
            cost,
            payer: payer.to_string(),
            switched,
        }
    }
}

/// Condition for when a static ability is active
///
/// Used for abilities like "During your turn, this creature has hexproof"
/// or "Sedge Troll gets +1/+1 as long as you control a Swamp".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticCondition {
    /// Active only during the controller's turn
    /// Corresponds to: `Condition$ PlayerTurn`
    PlayerTurn,
    /// Active only during opponents' turns
    /// Corresponds to: `Condition$ NotPlayerTurn`
    NotPlayerTurn,
    /// Active only while the source's controller has at least `min_count`
    /// permanents (or cards in `zone`) matching `filter`.
    ///
    /// Corresponds to: `IsPresent$ <filter>` (+ optional `PresentZone$`,
    /// `PresentCompare$`). Example from Sedge Troll:
    /// `S:Mode$ Continuous | ... | IsPresent$ Swamp.YouCtrl` — only active
    /// while the controller controls a Swamp.
    ControlsPresent {
        /// Card filter, e.g. `"Swamp.YouCtrl"` (subtype `.` ownership/control).
        filter: String,
        /// Zone in which to look for matching cards (default Battlefield).
        zone: crate::zones::Zone,
        /// Minimum number of matching cards required for the condition to hold.
        min_count: u8,
    },
}

/// Comparison operator for `PresentCompare$` activation/static conditions.
///
/// Forge encodes these as `EQ7`, `GE2`, `LE3`, etc. — a two-letter operator
/// followed by a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    /// `EQ` — equal to.
    Equal,
    /// `GE` — greater than or equal to.
    GreaterEqual,
    /// `LE` — less than or equal to.
    LessEqual,
    /// `GT` — strictly greater than.
    Greater,
    /// `LT` — strictly less than.
    Less,
}

impl CompareOp {
    /// Parse the two-letter Forge operator prefix (`EQ`/`GE`/`LE`/`GT`/`LT`).
    pub fn parse(prefix: &str) -> Option<Self> {
        match prefix {
            "EQ" => Some(CompareOp::Equal),
            "GE" => Some(CompareOp::GreaterEqual),
            "LE" => Some(CompareOp::LessEqual),
            "GT" => Some(CompareOp::Greater),
            "LT" => Some(CompareOp::Less),
            _ => None,
        }
    }

    /// Evaluate `actual <op> threshold`.
    pub fn matches(self, actual: usize, threshold: usize) -> bool {
        match self {
            CompareOp::Equal => actual == threshold,
            CompareOp::GreaterEqual => actual >= threshold,
            CompareOp::LessEqual => actual <= threshold,
            CompareOp::Greater => actual > threshold,
            CompareOp::Less => actual < threshold,
        }
    }
}

/// Restriction on when an activated ability may be activated, derived from
/// `IsPresent$ <filter> | PresentZone$ <zone> | PresentCompare$ <op><n>`.
///
/// "Activate only if you have exactly seven cards in hand" (Library of
/// Alexandria, Magus of the Library), "Activate only if you control two or
/// more white permanents" (Mistveil Plains), "...five or more lands"
/// (Cryptic Caves), etc. The count is over cards in `zone` matching `filter`
/// from the activating player's perspective, compared to `count` via `op`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationCondition {
    /// Forge `IsPresent$` filter, e.g. `"Card.YouOwn"`, `"Land.YouCtrl"`,
    /// `"Permanent.White+YouCtrl"`.
    pub filter: String,
    /// Zone to count in (default Battlefield; Hand for Library of Alexandria).
    pub zone: crate::zones::Zone,
    /// Comparison operator.
    pub op: CompareOp,
    /// Threshold count.
    pub count: u8,
}

/// Selector for which cards are affected by a static ability
///
/// Parsed from the `Affected$` parameter in card scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffectedSelector {
    /// The creature equipped by this Equipment
    /// Corresponds to: `Affected$ Creature.EquippedBy`
    CreatureEquippedBy,

    /// Creatures you control
    /// Corresponds to: `Affected$ Creature.YouCtrl`
    CreaturesYouControl,

    /// Other creatures you control (excluding self)
    /// Corresponds to: `Affected$ Creature.YouCtrl+Other`
    /// Used by cards like Elesh Norn that grant bonuses to "other creatures you control"
    CreaturesYouControlOther,

    /// All creatures
    /// Corresponds to: `Affected$ Creature`
    AllCreatures,

    /// This card itself
    /// Corresponds to: `Affected$ Card.Self`
    Self_,

    /// The land this Aura is attached to
    /// Corresponds to: `Affected$ Land.AttachedBy`
    /// Used by Auras that grant abilities to enchanted lands (e.g., Friendly Neighborhood)
    LandAttachedBy,

    /// Single creature type you control (tribal lords)
    /// Corresponds to: `Affected$ Goblin.YouCtrl`, `Affected$ Zombie.YouCtrl`, etc.
    /// Used by tribal lord cards that grant bonuses to a single creature type
    CreatureTypeYouControl {
        /// The creature subtype (e.g., "Goblin", "Zombie")
        subtype: crate::core::Subtype,
    },

    /// Single creature type you control, excluding self
    /// Corresponds to: `Affected$ Goblin.Other+YouCtrl`
    /// Used by tribal lord cards that exclude themselves from the bonus
    CreatureTypeOtherYouControl {
        /// The creature subtype (e.g., "Goblin", "Zombie")
        subtype: crate::core::Subtype,
    },

    /// Multiple creature types you control, excluding self
    /// Corresponds to: `Affected$ Spider.Other+YouCtrl,Boar.Other+YouCtrl,...`
    /// Used by cards like Spider-Ham that grant bonuses to multiple creature types
    /// The `Other` qualifier excludes the source card itself
    CreatureTypesOtherYouControl {
        /// List of creature subtypes (e.g., ["Spider", "Boar", "Goat", ...])
        types: Vec<crate::core::Subtype>,
    },

    /// The creature enchanted by this Aura
    /// Corresponds to: `Affected$ Creature.EnchantedBy`
    CreatureEnchantedBy,

    /// Artifact creatures you control, excluding self
    /// Corresponds to: `Affected$ Creature.Artifact+Other+YouCtrl`
    /// Used by cards like Master of Etherium that buff artifact creatures
    CreatureCardTypeOtherYouControl {
        /// The card type (e.g., "Artifact")
        card_type: crate::core::CardType,
    },

    /// Artifact creatures you control, including self
    /// Corresponds to: `Affected$ Creature.Artifact+YouCtrl`
    CreatureCardTypeYouControl {
        /// The card type (e.g., "Artifact")
        card_type: crate::core::CardType,
    },

    /// Land creatures you control
    /// Corresponds to: `Affected$ Creature.Land+YouCtrl`
    /// Used by cards that grant abilities to animated lands
    LandCreaturesYouControl,

    /// Non-human creatures you control, excluding self
    /// Corresponds to: `Affected$ Creature.nonHuman+Other+YouCtrl`
    /// Used by cards like Mikaeus, the Unhallowed
    CreatureNonTypeOtherYouControl {
        /// The creature subtype to exclude (e.g., "Human")
        excluded_subtype: crate::core::Subtype,
    },

    /// This card itself when equipped
    /// Corresponds to: `Affected$ Card.Self+equipped`
    /// Used by cards like Leonin Lightbringer, Kitesail Apprentice
    SelfWhenEquipped,

    /// This card itself when enchanted
    /// Corresponds to: `Affected$ Card.Self+enchanted`
    /// Used by cards like Thran Golem, Flaring Flame-Kin
    SelfWhenEnchanted,

    /// Creatures you control that are equipped
    /// Corresponds to: `Affected$ Creature.YouCtrl+equipped`
    /// Used by cards like Kemba, Kha Enduring
    EquippedCreaturesYouControl,

    /// Creatures you control that are enchanted
    /// Corresponds to: `Affected$ Creature.YouCtrl+enchanted`
    /// Used by cards like Sphere of Safety
    EnchantedCreaturesYouControl,

    /// All creatures of a specific type (global, not just yours)
    /// Corresponds to: `Affected$ Sliver`, `Affected$ Creature.Sliver`, `Affected$ Permanent.Sliver`
    /// Used by Sliver lords that affect ALL Slivers on the battlefield (both players)
    AllCreaturesOfType {
        /// The creature subtype (e.g., "Sliver")
        subtype: crate::core::Subtype,
    },

    /// The controller of this permanent (You)
    /// Corresponds to: `Affected$ You`
    /// Used by cards that grant abilities or effects to their controller
    /// Example: Absolute Virtue grants Protection to you
    You,

    /// All players in the game
    /// Corresponds to: `Affected$ Player`
    /// Used by symmetrical effects that affect all players equally
    Player,

    /// Lands you control
    /// Corresponds to: `Affected$ Land.YouCtrl`
    /// Used by cards like Chromatic Lantern that grant abilities to your lands
    LandsYouControl,

    /// Opponent's creatures
    /// Corresponds to: `Affected$ Creature.OppCtrl`
    /// Used by cards that debuff or affect enemy creatures
    CreaturesOpponentControls,

    /// Top card of your library
    /// Corresponds to: `Affected$ Card.TopLibrary+YouCtrl`
    /// Used by cards that let you look at or play the top card of your library
    /// Example: Courser of Kruphix, Garruk's Horde
    TopCardOfLibrary,

    /// Creature with something attached to it
    /// Corresponds to: `Affected$ Creature.AttachedBy`
    /// Used by Auras and Equipment that grant bonuses to the attached creature
    CreatureAttachedBy,

    /// Artifacts you control
    /// Corresponds to: `Affected$ Artifact.YouCtrl`
    /// Used by cards that grant bonuses to your artifacts
    ArtifactsYouControl,

    /// Other artifacts you control (excluding self)
    /// Corresponds to: `Affected$ Artifact.YouCtrl+Other` or `Artifact.Other+YouCtrl`
    /// Used by cards like Master of Etherium that affect other artifacts
    ArtifactsYouControlOther,

    /// All lands on the battlefield
    /// Corresponds to: `Affected$ Land`
    /// Used by global land effects (e.g., mass land animation)
    AllLands,

    /// Permanents you control
    /// Corresponds to: `Affected$ Permanent.YouCtrl`
    /// Used by cards that affect all your permanents regardless of type
    PermanentsYouControl,

    /// Token creatures you control
    /// Corresponds to: `Affected$ Creature.token+YouCtrl`
    /// Used by cards that buff token creatures specifically
    TokenCreaturesYouControl,

    /// Token creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Zombie.token+YouCtrl`, `Affected$ Spirit.token+YouCtrl`
    /// Used by cards that specifically buff token creatures of a certain type
    TokenCreatureTypeYouControl {
        /// The creature subtype (e.g., "Zombie", "Spirit")
        subtype: crate::core::Subtype,
    },

    /// Attacking creatures you control
    /// Corresponds to: `Affected$ Creature.attacking+YouCtrl`
    /// Used by cards that buff your attacking creatures
    AttackingCreaturesYouControl,

    /// All attacking creatures (regardless of controller)
    /// Corresponds to: `Affected$ Creature.attacking`
    /// Used by cards that affect all attackers
    AllAttackingCreatures,

    /// Opponent player(s)
    /// Corresponds to: `Affected$ Opponent`
    /// Used by effects that target or affect opponents
    Opponent,

    /// This card itself when attacking
    /// Corresponds to: `Affected$ Card.Self+attacking`
    /// Used by cards like Soltari Lancer that gain abilities while attacking
    SelfWhenAttacking,

    /// The artifact enchanted by this Aura
    /// Corresponds to: `Affected$ Artifact.EnchantedBy`
    /// Used by Auras that attach to artifacts (e.g., Splinter)
    ArtifactEnchantedBy,

    /// The planeswalker enchanted by this Aura
    /// Corresponds to: `Affected$ Planeswalker.EnchantedBy`
    /// Used by Auras that attach to planeswalkers
    PlaneswalkerEnchantedBy,

    /// The equipment enchanted by this Aura
    /// Corresponds to: `Affected$ Equipment.EnchantedBy`
    /// Used by Auras that attach to equipment
    EquipmentEnchantedBy,

    /// The land enchanted by this Aura
    /// Corresponds to: `Affected$ Land.EnchantedBy`
    /// Used by Auras that attach to lands (e.g., Squirrel Nest)
    LandEnchantedBy,

    /// Any permanent this Aura/Equipment is attached to
    /// Corresponds to: `Affected$ Card.AttachedBy`
    /// Used by generic Auras that can enchant any permanent type
    /// More generic than Creature.AttachedBy or Land.AttachedBy
    CardAttachedBy,

    /// Lands you own (not just control)
    /// Corresponds to: `Affected$ Land.YouOwn`
    /// Used by cards like Crucible of Worlds that let you play lands from graveyard
    LandsYouOwn,

    /// This card itself when untapped.
    ///
    /// Corresponds to: `Affected$ Card.Self+untapped`
    /// Used by cards that get bonuses while untapped (e.g., Wall of Roots +0/+3)
    SelfWhenUntapped,

    /// This card itself when monstrous (Monstrosity has been activated).
    ///
    /// Corresponds to: `Affected$ Card.Self+IsMonstrous`
    /// Used by cards with Monstrosity that gain abilities when monstrous
    SelfWhenMonstrous,

    /// Self when renowned (has +1/+1 counters from Renown ability)
    ///
    /// Corresponds to: `Affected$ Card.Self+IsRenowned`
    /// Used by cards with Renown that gain abilities when renowned
    SelfWhenRenowned,

    /// Tapped creatures you control, other than self.
    ///
    /// Corresponds to: `Affected$ Creature.tapped+YouCtrl+Other`
    /// Used by cards that benefit from or affect tapped creatures
    TappedCreaturesYouControlOther,

    /// Untapped creatures you control, other than self.
    ///
    /// Corresponds to: `Affected$ Creature.untapped+YouCtrl+Other`
    /// Used by cards that benefit from or affect untapped creatures
    UntappedCreaturesYouControlOther,

    /// Non-land permanents you control.
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl+nonLand`, `Affected$ Permanent.nonLand+YouCtrl`
    /// Used by cards that affect all non-land permanents you control
    NonLandPermanentsYouControl,

    /// Non-land cards you own (in any zone).
    ///
    /// Corresponds to: `Affected$ Card.YouOwn+nonLand`
    /// Used by cards that affect non-land cards you own
    NonLandCardsYouOwn,

    /// OR combination of multiple selectors (matches if ANY selector matches).
    ///
    /// Corresponds to comma-separated Affected$ values like:
    /// - `Affected$ Goblin.YouCtrl+Other,Orc.YouCtrl+Other` (tribal lords)
    /// - `Affected$ Instant,Sorcery` (spell type OR)
    /// - `Affected$ Creature.PairedWith,Creature.Self+Paired` (soulbond)
    ///
    /// Used when a card affects multiple distinct categories of permanents.
    Any(Vec<AffectedSelector>),

    /// All permanents on the battlefield.
    ///
    /// Corresponds to: `Affected$ Permanent`
    /// Used by effects that affect all permanents regardless of type or controller
    AllPermanents,

    /// All cards (any zone, any controller).
    ///
    /// Corresponds to: `Affected$ Card`
    /// Used by very broad effects that can affect cards in any zone
    AllCards,

    /// Cards you control (on the battlefield).
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl`
    /// Used by effects that affect all your permanents
    CardsYouControl,

    /// Cards owned by opponents.
    ///
    /// Corresponds to: `Affected$ Card.OppOwn`
    /// Used by effects that affect cards owned by opponents
    CardsOpponentOwns,

    /// This card itself when it has a minimum number of a specific counter type.
    ///
    /// Corresponds to: `Affected$ Card.Self+counters_GE*_TYPE`
    /// Examples:
    /// - `Card.Self+counters_GE8_CHARGE` (at least 8 charge counters)
    /// - `Card.Self+counters_GE1_P1P1` (at least 1 +1/+1 counter)
    ///
    /// Used by cards that gain abilities when they have enough counters.
    SelfWithCounters {
        /// The counter type (e.g., "CHARGE", "P1P1", "DIVINITY")
        counter_type: String,
        /// The minimum number of counters required
        minimum: u32,
    },

    /// Non-basic lands (either you control or all).
    ///
    /// Corresponds to: `Affected$ Land.nonBasic`, `Affected$ Land.nonBasic+YouCtrl`
    /// Used by effects that affect non-basic lands
    NonBasicLands,

    /// Creatures of a specific color, other than self.
    ///
    /// Corresponds to: `Affected$ Creature.Black+Other`, `Affected$ Creature.White+Other`
    /// Used by cards that buff creatures of a specific color excluding themselves
    CreatureColorOther {
        /// The color name (e.g., "Black", "White", "Blue")
        color: String,
    },

    /// All creatures of a specific color (including self).
    ///
    /// Corresponds to: `Affected$ Creature.White`, `Affected$ Creature.Black`, etc.
    /// Used by cards like Crusade that buff all creatures of a color
    AllCreaturesOfColor {
        /// The color name (e.g., "Black", "White", "Blue")
        color: String,
    },

    /// Humans equipped by this equipment.
    ///
    /// Corresponds to: `Affected$ Human.EquippedBy`
    /// Used by equipment that specifically grants bonuses to equipped Humans
    HumanEquippedBy,

    /// Cards that entered the battlefield this turn (usually self).
    ///
    /// Corresponds to: `Affected$ Card.Self+ThisTurnEntered`
    /// Used by cards that have effects when they ETB
    SelfThisTurnEntered,

    /// Card exiled with this source (imprint, exile-based effects).
    ///
    /// Corresponds to: `Affected$ Card.ExiledWithSource`
    /// Used by imprint effects like Chrome Mox, Isochron Scepter
    CardExiledWithSource,

    /// Top card of library (generic, any player).
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary`
    /// Used by Future Sight-like effects
    TopOfLibrary,

    /// Land card on top of your library.
    ///
    /// Corresponds to: `Affected$ Land.TopLibrary+YouCtrl`
    /// Used by effects that let you play lands from the top of your library
    LandTopOfLibrary,

    /// Non-land creature on top of your library.
    ///
    /// Corresponds to: `Affected$ Creature.TopLibrary+YouCtrl+nonLand`
    /// Used by effects that care about creature cards on top of library
    CreatureTopOfLibraryNonLand,

    /// Commander you control.
    ///
    /// Corresponds to: `Affected$ Card.IsCommander+YouCtrl`
    /// Used by Commander-specific cards
    CommanderYouControl,

    /// Creature equipped by a legendary equipment.
    ///
    /// Corresponds to: `Affected$ Card.EquippedBy+Legendary`
    /// Used by legendary equipment with special abilities
    EquippedByLegendary,

    /// Top card of library you own.
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary+YouOwn`
    /// Used by effects that affect cards you own on top of library
    TopOfLibraryYouOwn,

    /// Any permanent this is attached to (generic).
    ///
    /// Corresponds to: `Affected$ Permanent.AttachedBy`
    /// Used by generic auras/equipment that affect any permanent type
    PermanentAttachedBy,

    /// Non-creature artifacts.
    ///
    /// Corresponds to: `Affected$ Artifact.nonCreature`
    /// Used by effects that only affect non-creature artifacts
    ArtifactsNonCreature,

    /// All artifacts.
    ///
    /// Corresponds to: `Affected$ Artifact`
    /// Used by effects that affect all artifacts regardless of controller
    AllArtifacts,

    /// Basic lands you control.
    ///
    /// Corresponds to: `Affected$ Land.Basic+YouCtrl`
    /// Used by effects that affect basic lands you control
    BasicLandsYouControl,

    /// Specific basic land type (e.g., Mountain, Forest, Island).
    ///
    /// Corresponds to: `Affected$ Mountain`, `Affected$ Forest`, etc.
    /// Used by effects that affect specific land types
    SpecificLandType {
        /// The land type name (e.g., "Mountain", "Island")
        land_type: String,
    },

    /// Non-land cards with CMC at most X.
    ///
    /// Corresponds to: `Affected$ Card.nonLand+cmcLEX`
    /// Used by effects that care about converted mana cost
    NonLandCmcLE {
        /// The maximum CMC (often X, which would be resolved at runtime)
        max_cmc: i32,
    },

    /// Creature of a specific type with flying that opponent controls.
    ///
    /// Corresponds to: `Affected$ Creature.withFlying+OppCtrl`
    /// Used by effects that target flying creatures opponents control
    CreatureWithFlyingOppCtrl,

    /// Other creatures of a specific type (zombies, etc.) you control.
    ///
    /// Corresponds to: `Affected$ Creature.Zombie+Other`
    /// Used by zombie lords and similar effects
    CreatureTypeOther {
        /// The creature subtype
        subtype: crate::core::Subtype,
    },

    /// Slivers you control (specific handling).
    ///
    /// Corresponds to: `Affected$ Permanent.Sliver+YouCtrl`
    /// Used by Slivers that only affect your own Slivers
    SliversYouControl,

    /// Equipment attached to a permanent.
    ///
    /// Corresponds to: `Affected$ Permanent.EquippedBy`
    /// Used by effects that care about equipped permanents
    PermanentEquippedBy,

    /// Vehicles this is attached to.
    ///
    /// Corresponds to: `Affected$ Vehicle.AttachedBy`
    /// Used by crew-related effects
    VehicleAttachedBy,

    /// Non-land cards you own without Foretell.
    ///
    /// Corresponds to: `Affected$ Card.nonLand+YouOwn+withoutForetell`
    /// Used by Foretell-related effects
    NonLandCardsYouOwnWithoutForetell,

    /// Non-land cards on top of library.
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary+YouOwn+nonLand`
    /// Used by effects that care about non-land cards on top
    TopOfLibraryNonLand,

    /// Remembered cards (from imprint or other memory effects).
    ///
    /// Corresponds to: `Affected$ Card.IsRemembered`
    /// Used by effects that reference previously imprinted/remembered cards
    RememberedCards,

    /// Creature cards that were cast (not put into play).
    ///
    /// Corresponds to: `Affected$ Card.Creature+YouCtrl+wasCast`
    /// Used by effects that care about cast vs. put into play
    CreatureYouControlWasCast,

    /// Cards of a specific type that you own.
    ///
    /// Corresponds to: `Affected$ Instant.YouOwn`, `Affected$ Sorcery.YouOwn`, etc.
    /// Used by flashback-granting effects like Snapcaster Mage's ability
    /// or cards that let you cast spells from graveyard
    CardTypeYouOwn {
        /// The card type (e.g., Instant, Sorcery, Aura, Equipment)
        card_type: crate::core::CardType,
    },

    /// Cards of a specific subtype that you own.
    ///
    /// Corresponds to: `Affected$ Aura.YouOwn`, `Affected$ Equipment.YouOwn`
    /// where Aura/Equipment are subtypes, not card types.
    /// Used by effects that grant flashback or graveyard casting
    SubtypeYouOwn {
        /// The subtype (e.g., "Aura", "Equipment", "Merfolk", "Druid")
        subtype: crate::core::Subtype,
    },

    /// Card type on top of your library.
    ///
    /// Corresponds to: `Affected$ Instant.TopLibrary+YouCtrl`, `Affected$ Sorcery.TopLibrary+YouCtrl`
    /// Used by effects that let you cast specific card types from top of library
    CardTypeTopLibrary {
        /// The card type (e.g., Instant, Sorcery)
        card_type: crate::core::CardType,
    },

    /// Subtype on top of your library (non-land).
    ///
    /// Corresponds to: `Affected$ Angel.TopLibrary+YouCtrl+nonLand`, `Affected$ Human.TopLibrary+YouCtrl+nonLand`
    /// Used by effects that let you cast specific creature types from top of library
    SubtypeTopLibraryNonLand {
        /// The creature subtype (e.g., "Angel", "Human")
        subtype: crate::core::Subtype,
    },

    /// Permanent of a specific subtype you control.
    ///
    /// Corresponds to: `Affected$ Permanent.Servo+YouCtrl`, `Affected$ Permanent.Thopter+YouCtrl`
    /// Used by effects that buff specific permanent types
    PermanentSubtypeYouControl {
        /// The subtype (e.g., "Servo", "Thopter")
        subtype: crate::core::Subtype,
    },

    /// Creature equipped by this equipment, if it has a specific subtype.
    ///
    /// Corresponds to: `Affected$ Card.EquippedBy+Human`, `Affected$ Card.EquippedBy+Angel`
    /// Used by equipment that grants bonuses to specific creature types
    EquippedBySubtype {
        /// The required subtype (e.g., "Human", "Angel")
        subtype: crate::core::Subtype,
    },

    /// Non-creature artifacts you control.
    ///
    /// Corresponds to: `Affected$ Artifact.nonCreature+YouCtrl`
    /// Used by effects that affect non-creature artifacts you control
    ArtifactsNonCreatureYouControl,

    /// Other artifact creatures you control.
    ///
    /// Corresponds to: `Affected$ Artifact.Creature+YouCtrl+Other`
    /// Used by cards like Master of Etherium
    ArtifactCreaturesYouControlOther,

    /// Treasure tokens/permanents you control.
    ///
    /// Corresponds to: `Affected$ Card.Treasure+YouCtrl`
    /// Used by cards that buff or care about Treasures
    TreasuresYouControl,

    /// Cards you control that were cast (not put onto battlefield).
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl+wasCast`
    /// Used by effects that care about cast vs ETB
    CardsYouControlWasCast,

    /// Self card on top of library.
    ///
    /// Corresponds to: `Affected$ Card.Self+TopLibrary`
    /// Used by top-of-library casting effects on self
    SelfTopLibrary,

    /// Instant spells of a specific color you control.
    ///
    /// Corresponds to: `Affected$ Instant.Red+YouCtrl`, `Affected$ Instant.Green+YouCtrl`
    /// Used by effects that grant abilities to colored instants
    InstantColorYouControl {
        /// The color (e.g., "Red", "Green")
        color: String,
    },

    /// Sorcery spells of a specific color you control.
    ///
    /// Corresponds to: `Affected$ Sorcery.Red+YouCtrl`, `Affected$ Sorcery.Green+YouCtrl`
    /// Used by effects that grant abilities to colored sorceries
    SorceryColorYouControl {
        /// The color (e.g., "Red", "Green")
        color: String,
    },

    /// Card type with subtype on top of library.
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary+YouCtrl+Bird`, `Affected$ Card.TopLibrary+YouCtrl+Land`
    /// Used by effects that let you play specific types from top of library
    TopLibraryWithSubtype {
        /// The subtype filter (e.g., "Bird", "Land")
        subtype: crate::core::Subtype,
    },

    /// Permanents opponent controls.
    ///
    /// Corresponds to: `Affected$ Permanent.OppCtrl`
    /// Used by effects that debuff or affect enemy permanents
    PermanentsOpponentControls,

    /// Attacking creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Vampire.attacking+YouCtrl`, `Affected$ Pirate.attacking+YouCtrl`
    /// Used by tribal cards that grant bonuses to attacking creatures of a type
    AttackingCreatureTypeYouControl {
        /// The creature subtype (e.g., "Vampire", "Pirate")
        subtype: crate::core::Subtype,
    },

    /// Legendary creatures or permanents.
    ///
    /// Corresponds to: `Affected$ Creature.Legendary+YouCtrl`, `Affected$ Permanent.Legendary+YouCtrl`
    /// Used by effects that affect legendary permanents
    LegendaryYouControl,

    /// Other legendary permanents you control.
    ///
    /// Corresponds to: `Affected$ Permanent.Other+YouCtrl+Legendary`
    /// Used by effects that buff other legendaries
    LegendaryOtherYouControl,

    /// Equipped creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Warrior.YouCtrl+equipped`, `Affected$ Knight.YouCtrl+equipped`
    /// Used by equipment-matters tribal effects
    EquippedCreatureTypeYouControl {
        /// The creature subtype (e.g., "Warrior", "Knight")
        subtype: crate::core::Subtype,
    },

    /// Legendary creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Human.YouCtrl+Legendary`, `Affected$ Snake.Legendary+YouCtrl`
    /// Used by legendary-matters tribal effects
    LegendarySubtypeYouControl {
        /// The creature subtype (e.g., "Human", "Snake")
        subtype: crate::core::Subtype,
    },

    /// Other non-aura enchantments.
    ///
    /// Corresponds to: `Affected$ Enchantment.nonAura+Other`
    /// Used by cards that care about non-aura enchantments (excluding self)
    NonAuraEnchantmentsOther,

    /// This card itself when tapped.
    ///
    /// Corresponds to: `Affected$ Card.Self+tapped`
    /// Used by cards that gain abilities or stats when tapped
    SelfWhenTapped,

    /// This card itself if it was cast (not put onto battlefield).
    ///
    /// Corresponds to: `Affected$ Card.Self+wasCast`
    /// Used by effects that care about whether the card was cast
    SelfWhenCast,

    /// Enchantments you control.
    ///
    /// Corresponds to: `Affected$ Card.Enchantment+YouCtrl`, `Affected$ Enchantment.YouCtrl`
    /// Used by effects that affect your enchantments
    EnchantmentsYouControl,

    /// Historic permanents you control (legendary, artifact, or saga).
    ///
    /// Corresponds to: `Affected$ Card.Historic+YouCtrl`
    /// Used by effects that care about historic cards
    HistoricYouControl,

    /// Historic permanents you own (any zone).
    ///
    /// Corresponds to: `Affected$ Card.Historic+YouOwn`
    /// Used by effects that grant flashback or graveyard access to historic cards
    HistoricYouOwn,

    /// Card subtype with Other+YouCtrl pattern (e.g., `Card.Human+Other+YouCtrl`).
    ///
    /// Corresponds to: `Affected$ Card.Human+Other+YouCtrl`, etc.
    /// Different from creature-specific tribal lords - this is Card-prefixed
    CardSubtypeOtherYouControl {
        /// The subtype (e.g., "Human", "Merfolk")
        subtype: crate::core::Subtype,
    },

    /// Card subtype with YouCtrl pattern (e.g., `Card.Horror+YouCtrl`).
    ///
    /// Corresponds to: `Affected$ Card.Horror+YouCtrl`, etc.
    CardSubtypeYouControl {
        /// The subtype (e.g., "Horror", "Satyr")
        subtype: crate::core::Subtype,
    },

    /// Permanent subtype with Other+YouCtrl pattern (e.g., `Permanent.Dwarf+Other+YouCtrl`).
    ///
    /// Corresponds to: `Affected$ Permanent.Dwarf+Other+YouCtrl`, etc.
    /// For permanents (not just creatures) of a type
    PermanentSubtypeOtherYouControl {
        /// The subtype (e.g., "Dwarf", "Elf")
        subtype: crate::core::Subtype,
    },

    /// This card itself when NOT attacking.
    ///
    /// Corresponds to: `Affected$ Card.Self+!attacking`
    /// Used by cards that have abilities when not attacking
    SelfWhenNotAttacking,

    /// This card itself when NOT attacking and NOT blocking.
    ///
    /// Corresponds to: `Affected$ Card.Self+!attacking+!blocking`
    /// Used by cards that have abilities when not in combat
    SelfWhenNotInCombat,

    /// Artifact permanents that are not tokens.
    ///
    /// Corresponds to: `Affected$ Artifact.!token+YouCtrl`
    /// Used by effects that only affect non-token artifacts
    NonTokenArtifactsYouControl,

    /// Artifacts that are not legendary.
    ///
    /// Corresponds to: `Affected$ Card.Artifact+nonLegendary+YouCtrl`
    /// Used by effects that only affect non-legendary artifacts
    NonLegendaryArtifactsYouControl,

    /// Cards that were cast from exile.
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl+wasCastFromExile`
    /// Used by exile-casting effects (foretell, suspend, etc.)
    CardsYouControlCastFromExile,

    /// Commander you own (any zone).
    ///
    /// Corresponds to: `Affected$ Card.IsCommander+YouOwn`
    /// Used by Commander-specific effects
    CommanderYouOwn,

    /// Elf creatures other than self.
    ///
    /// Corresponds to: `Affected$ Card.Elf+Other`
    /// Used by elf lords that affect all elves
    SubtypeOther {
        /// The subtype (e.g., "Elf", "Merfolk")
        subtype: crate::core::Subtype,
    },
}

/// Cache for expensive string operations on ActivatedAbility
/// Pre-computed at ability creation time to avoid repeated allocations during gameplay
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityCache {
    /// Lowercase version of description (computed once)
    pub description_lowercase: String,

    /// Pre-computed contains() checks for targeting restrictions
    pub targets_tapped: bool,
    pub targets_untapped: bool,
    pub targets_creature: bool,
    pub targets_land: bool,
    pub requires_target: bool,
}

impl AbilityCache {
    /// Create a new cache from ability description
    pub fn new(description: &str) -> Self {
        let desc_lower = description.to_lowercase();

        AbilityCache {
            // Store lowercase version
            description_lowercase: desc_lower.clone(),

            // Targeting restriction flags
            targets_tapped: desc_lower.contains("tapped"),
            targets_untapped: desc_lower.contains("untapped"),
            targets_creature: desc_lower.contains("creature"),
            targets_land: desc_lower.contains("land"),
            requires_target: desc_lower.contains("target") || desc_lower.starts_with("equip"),
        }
    }
}

/// An activated ability that can be activated by paying a cost
/// Example: "{T}: Deal 1 damage to any target" (Prodigal Sorcerer)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivatedAbility {
    /// The cost to activate this ability
    pub cost: crate::core::Cost,

    /// The effects that execute when this ability resolves
    pub effects: Vec<Effect>,

    /// Description of the ability (for logging and display)
    pub description: String,

    /// Whether this is a mana ability (doesn't use the stack)
    pub is_mana_ability: bool,

    /// Whether this ability can only be activated at sorcery speed
    /// "Activate only as a sorcery" (CR 602.5d, CR 307.5)
    /// Requires: priority, main phase, your turn, stack empty
    pub sorcery_speed: bool,

    /// Whether this ability can only be activated during your turn
    /// "Activate only during your turn" (PlayerTurn$ True)
    /// Less restrictive than sorcery_speed - only checks turn ownership
    pub your_turn_only: bool,

    /// Whether this is an exhaust ability (can only be activated once per game)
    /// "Exhaust$ True" - activate each exhaust ability only once
    pub exhaust: bool,

    /// Optional "Activate only if ..." restriction from
    /// `IsPresent$ | PresentZone$ | PresentCompare$` (Library of Alexandria's
    /// "exactly seven cards in hand", Cryptic Caves' "five or more lands", ...).
    /// `None` = no extra restriction. Checked in `can_activate` alongside the
    /// other timing/cost gates.
    #[serde(default)]
    pub activation_condition: Option<crate::core::ActivationCondition>,

    /// Cache for expensive string operations (computed at creation time)
    pub cache: AbilityCache,
}

impl ActivatedAbility {
    /// Create a new activated ability
    pub fn new(cost: crate::core::Cost, effects: Vec<Effect>, description: String, is_mana_ability: bool) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability,
            sorcery_speed: false,  // Default to instant speed
            your_turn_only: false, // Default to any turn
            exhaust: false,        // Default to non-exhaust
            activation_condition: None,
            cache,
        }
    }

    /// Create a new sorcery-speed activated ability
    pub fn new_sorcery_speed(cost: crate::core::Cost, effects: Vec<Effect>, description: String) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability: false, // Sorcery-speed abilities are not mana abilities
            sorcery_speed: true,
            your_turn_only: false, // sorcery_speed implies your turn already
            exhaust: false,
            activation_condition: None,
            cache,
        }
    }

    /// Create a new your-turn-only activated ability
    /// Less restrictive than sorcery speed - can be activated any time during your turn
    pub fn new_your_turn_only(
        cost: crate::core::Cost,
        effects: Vec<Effect>,
        description: String,
        is_mana_ability: bool,
    ) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability,
            sorcery_speed: false, // Not sorcery speed
            your_turn_only: true, // Your turn only
            exhaust: false,
            activation_condition: None,
            cache,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_creation() {
        let player_id = PlayerId::new(1);
        let card_id = CardId::new(100);

        let damage_effect = Effect::DealDamage {
            target: TargetRef::Player(player_id),
            amount: 3,
        };

        let Effect::DealDamage { target, amount } = damage_effect else {
            panic!("Wrong effect type: expected DealDamage, got {damage_effect:?}");
        };
        assert_eq!(amount, 3);
        assert_eq!(target, TargetRef::Player(player_id));

        let draw_effect = Effect::DrawCards {
            player: player_id,
            count: 2,
        };

        let Effect::DrawCards { player, count } = draw_effect else {
            panic!("Wrong effect type: expected DrawCards, got {draw_effect:?}");
        };
        assert_eq!(player, player_id);
        assert_eq!(count, 2);

        let destroy_effect = Effect::DestroyPermanent {
            target: card_id,
            restriction: TargetRestriction::any(),
            no_regenerate: false,
        };

        let Effect::DestroyPermanent { target, .. } = destroy_effect else {
            panic!("Wrong effect type: expected DestroyPermanent, got {destroy_effect:?}");
        };
        assert_eq!(target, card_id);
    }

    #[test]
    fn test_count_expression_parse_fixed() {
        let svars = std::collections::HashMap::new();

        // Fixed positive value
        let expr = CountExpression::parse("+3", &svars);
        assert_eq!(expr, CountExpression::Fixed(3));

        // Fixed negative value
        let expr = CountExpression::parse("-2", &svars);
        assert_eq!(expr, CountExpression::Fixed(-2));

        // Fixed value without sign
        let expr = CountExpression::parse("5", &svars);
        assert_eq!(expr, CountExpression::Fixed(5));
    }

    #[test]
    fn test_count_expression_parse_valid_permanents() {
        let mut svars = std::collections::HashMap::new();
        svars.insert("X".to_string(), "Count$Valid Artifact.OppCtrl".to_string());

        let expr = CountExpression::parse("+X", &svars);
        assert!(
            matches!(&expr, CountExpression::ValidPermanents { filter } if filter == "Artifact.OppCtrl"),
            "Expected ValidPermanents with Artifact.OppCtrl filter, got {:?}",
            expr
        );
    }

    #[test]
    fn test_count_expression_parse_you_drew_this_turn() {
        let mut svars = std::collections::HashMap::new();
        svars.insert("X".to_string(), "Count$YouDrewThisTurn".to_string());

        let expr = CountExpression::parse("+X", &svars);
        assert_eq!(
            expr,
            CountExpression::CardsDrawnThisTurn,
            "Expected CardsDrawnThisTurn, got {:?}",
            expr
        );
    }

    #[test]
    fn test_count_expression_parse_unknown() {
        let svars = std::collections::HashMap::new();

        // Unknown variable -> defaults to 0
        let expr = CountExpression::parse("X", &svars);
        assert_eq!(expr, CountExpression::Fixed(0));
    }

    #[test]
    fn test_count_expression_parse_compare() {
        // Test Count$Compare parsing (Raucous Audience pattern)
        // X = "Count$Compare Y GE1.2.1" means: if Y >= 1 then 2 else 1
        // Y = "Count$Valid Creature.YouCtrl+powerGE4" means: count creatures with power >= 4
        let mut svars = std::collections::HashMap::new();
        svars.insert("X".to_string(), "Count$Compare Y GE1.2.1".to_string());
        svars.insert("Y".to_string(), "Count$Valid Creature.YouCtrl+powerGE4".to_string());

        let expr = CountExpression::parse("X", &svars);
        match &expr {
            CountExpression::Compare {
                source,
                condition,
                true_value,
                false_value,
            } => {
                // Check the nested source was resolved
                match source.as_ref() {
                    CountExpression::ValidPermanents { filter } => {
                        assert_eq!(filter, "Creature.YouCtrl+powerGE4");
                    }
                    CountExpression::Fixed(_)
                    | CountExpression::CardsDrawnThisTurn
                    | CountExpression::XPaid
                    | CountExpression::SpellsCastThisTurn
                    | CountExpression::Compare { .. } => {
                        panic!("Expected ValidPermanents, got {:?}", source)
                    }
                }
                // Check the condition
                assert!(matches!(condition, CompareCondition::GreaterOrEqual(1)));
                // Check the values
                assert_eq!(*true_value, 2);
                assert_eq!(*false_value, 1);
            }
            CountExpression::Fixed(_)
            | CountExpression::ValidPermanents { .. }
            | CountExpression::CardsDrawnThisTurn
            | CountExpression::XPaid
            | CountExpression::SpellsCastThisTurn => panic!("Expected Compare, got {:?}", expr),
        }
    }

    #[test]
    fn test_compare_condition_evaluate() {
        // Test condition evaluation
        assert!(CompareCondition::GreaterOrEqual(1).evaluate(1));
        assert!(CompareCondition::GreaterOrEqual(1).evaluate(2));
        assert!(!CompareCondition::GreaterOrEqual(1).evaluate(0));

        assert!(CompareCondition::LessOrEqual(2).evaluate(1));
        assert!(CompareCondition::LessOrEqual(2).evaluate(2));
        assert!(!CompareCondition::LessOrEqual(2).evaluate(3));

        assert!(CompareCondition::Equal(3).evaluate(3));
        assert!(!CompareCondition::Equal(3).evaluate(2));
    }
}
