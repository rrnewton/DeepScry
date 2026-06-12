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
    /// resolution (read from the spell's damage bookkeeping). Used by Spirit
    /// Link's triggered "you gain that much life".
    DamageDealt,

    /// The damage dealt earlier in this same resolution, CAPPED by the damage
    /// target's life / loyalty / toughness BEFORE the damage was dealt. Used by
    /// Drain Life: "you gain life equal to the damage dealt, but not more than
    /// the player's life total / the planeswalker's loyalty / the creature's
    /// toughness before the damage was dealt."
    ///
    /// This is the semantic outcome of the card's `StoreSVar`/`LimitMax.Limit`
    /// chain (`DrainedLifeCard = SVar$Y/LimitMax.Limit`, `Y =
    /// Count$TotalDamageDoneByThisTurn`, `Limit` = the chosen target's
    /// characteristic): rather than implement a generic SVar-store + per-target
    /// condition selector, the engine reads the cap directly from the target.
    /// The cap is captured from the pre-damage snapshot (CR 608.2g/2h —
    /// last-known information; a player's life has already dropped by the time
    /// the chained GainLife runs). Reads only public characteristics, so it is
    /// information-independent (network determinism). Clamped to >= 0.
    ///
    /// `cap` is the pre-damage characteristic, locked from the target snapshot
    /// during target resolution (`None` until locked; treated as "no cap" if it
    /// somehow reaches resolution unlocked, degrading to plain `DamageDealt`).
    DamageDealtCappedByTarget { cap: Option<i32> },

    /// A [`CountExpression`] evaluated against the recipient player at
    /// resolution time. Used for hand-size-driven life gain such as Ivory
    /// Tower (`Count$ValidHand Card.YouOwn/Minus.4` — "gain life equal to the
    /// number of cards in your hand minus 4"). The count reads only public
    /// state (hand SIZE, permanent counts), so it is information-independent
    /// (network determinism). The result is clamped to >= 0 by the resolver
    /// (CR 119.4 — a player cannot gain negative life).
    Count(CountExpression),

    /// The toughness of the creature sacrificed to pay this ability's cost,
    /// captured via last-known information (CR 608.2g — the creature has already
    /// left the battlefield by the time the ability resolves). Used by Diamond
    /// Valley (`{T}, Sacrifice a creature: You gain life equal to the sacrificed
    /// creature's toughness`), whose `LifeAmount$ X` references
    /// `SVar:X:Sacrificed$CardToughness`. The `reference` card on the owning
    /// `GainLifeDynamic` is filled at resolution time from the sacrificed
    /// permanent recorded during cost payment; reading its retained (LKI)
    /// toughness is public information, so it is information-independent
    /// (network determinism). Clamped to >= 0.
    SacrificedToughness,

    /// The number of counters of a specific type on the card that triggered the
    /// effect (the "triggered card", read via last-known information per CR
    /// 608.2g/603.6c). Used by Hangarback Walker's death trigger: "create a
    /// 1/1 Thopter for each +1/+1 counter on CARDNAME." The count is captured
    /// into the `TriggerContext` before the card leaves the battlefield,
    /// ensuring deterministic network behavior (the counter count is public
    /// information — always visible on the battlefield).
    ///
    /// Parsed from SVar bodies of the form `TriggeredCard$CardCounters.<type>`,
    /// e.g. `SVar:Y:TriggeredCard$CardCounters.P1P1`.
    TriggeredCardCounters(crate::core::CounterType),
}

impl DynamicAmount {
    /// Parse a `LifeAmount$ <expr>` value, resolving an `X`/`Y`/`Z` reference
    /// through the card's SVars, into a `DynamicAmount`.
    ///
    /// Recognised SVar bodies (tokenized, never substring-matched):
    /// - `Targeted$CardPower`      -> `TargetPower`
    /// - `Targeted$CardManaCost`   -> `TargetManaValue`
    /// - `TriggerCount$DamageAmount` -> `DamageDealt` (Spirit Link's triggered
    ///   pseudo-lifelink: "you gain that much life" reads the damage just dealt)
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
        // SVar bodies for these references are `<Selector>$<Characteristic>`.
        let (selector, characteristic) = svar_body.split_once('$')?;
        match (selector.trim(), characteristic.trim()) {
            ("Targeted", "CardPower") => Some(DynamicAmount::TargetPower),
            ("Targeted", "CardManaCost") => Some(DynamicAmount::TargetManaValue),
            // Diamond Valley: SVar:X:Sacrificed$CardToughness — gain life equal
            // to the toughness of the creature sacrificed to pay the cost. The
            // sacrificed creature is recorded during cost payment and read via
            // last-known information at resolution (CR 608.2g).
            ("Sacrificed", "CardToughness") => Some(DynamicAmount::SacrificedToughness),
            // Spirit Link: SVar:X:TriggerCount$DamageAmount — the amount of
            // damage the trigger event just reported.
            ("TriggerCount", "DamageAmount") => Some(DynamicAmount::DamageDealt),
            // Drain Life: SVar:DrainedLifeCard:SVar$Y/LimitMax.Limit, where
            // Y = Count$TotalDamageDoneByThisTurn and Limit = the target's
            // toughness / loyalty / life captured by the StoreSVar chain. The
            // "<damage>/LimitMax.Limit" shape means "damage dealt, capped by the
            // stored Limit" — i.e. gain = min(damage dealt, target life/loyalty/
            // toughness before damage). We recognise the `SVar$ <var>/LimitMax.…`
            // form (tokenized: a `/LimitMax.` segment) rather than the generic
            // SVar-store machinery.
            ("SVar", rest) if rest.contains("/LimitMax.") => {
                Some(DynamicAmount::DamageDealtCappedByTarget { cap: None })
            }
            // Hangarback Walker et al.: SVar:Y:TriggeredCard$CardCounters.P1P1
            // The token count equals the number of counters of type <T> on the
            // card that just triggered the effect (last-known information).
            ("TriggeredCard", counter_ref) => {
                let counter_str = counter_ref.strip_prefix("CardCounters.")?;
                let ct = crate::core::CounterType::parse(counter_str)?;
                Some(DynamicAmount::TriggeredCardCounters(ct))
            }
            // Count$… bodies (hand size, permanent counts, …) drive a dynamic
            // life amount evaluated against the recipient at resolution time.
            // Ivory Tower: SVar:X:Count$ValidHand Card.YouOwn/Minus.4. Delegate
            // to CountExpression::parse (DRY); a Fixed result means the body was
            // unrecognized, so fall back to the caller's fixed-amount path.
            ("Count", _) => {
                let expr = CountExpression::parse(value, svars);
                if matches!(expr, CountExpression::Fixed(_)) {
                    None
                } else {
                    Some(DynamicAmount::Count(expr))
                }
            }
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
    ///
    /// The optional `modifier` is applied to the raw count after counting.
    /// Forge encodes per-copy multiplication as `/Times.N` (e.g.
    /// `Count$Valid Shrine.YouCtrl/Times.2` = "2 × Shrines you control").
    ValidPermanents {
        /// The filter string (e.g., "Artifact.OppCtrl")
        filter: String,
        /// Arithmetic post-modifier applied to the raw count (`/Times.N`,
        /// `/Plus.N`, `/Minus.N`). Defaults to `None`.
        modifier: CountModifier,
    },

    /// Count cards drawn this turn (Count$YouDrewThisTurn)
    CardsDrawnThisTurn,

    /// Count cards in a player's HAND, with an optional arithmetic post-modifier.
    ///
    /// Corresponds to Forge's `Count$ValidHand <selector>/Minus.N` (and the
    /// symmetric `/Plus.N`). Example — Black Vise:
    /// `SVar:X:Count$ValidHand Card.ChosenCtrl/Minus.4` = "cards in the chosen
    /// player's hand minus 4". The hand owner is resolved at evaluation time
    /// from the player the count is being evaluated *for* (the trigger's
    /// triggered/chosen player), so the engine never needs hidden card
    /// identities — only the public hand SIZE — which keeps the result
    /// information-independent (network determinism) and a pure function of
    /// public state (CR 119: damage = max(0, handsize - 4)).
    CardsInHand {
        /// The raw selector that preceded the `/Modifier` (e.g.
        /// `Card.ChosenCtrl`). Retained for diagnostics / future selector
        /// widening; the current evaluator counts the player the expression is
        /// evaluated for (see `evaluate_count_expression`).
        selector: String,
        /// Arithmetic post-modifier applied to the raw hand size. `Minus(4)`
        /// for Black Vise's `/Minus.4`. The evaluated value is NOT clamped here
        /// (callers clamp to >= 0 where MTG requires it, e.g. damage).
        modifier: CountModifier,
    },

    /// The value of X paid when casting this spell (Count$xPaid)
    /// Resolved at effect execution time by reading Card::x_paid
    XPaid,

    /// The current power of the card THIS effect targets (`Targeted$CardPower`).
    ///
    /// Berserk: `NumAtt$ +X` with `SVar:X:Targeted$CardPower` = "+X/+0 where X
    /// is the target's power" (power-doubling). Resolved at effect-execution
    /// time from the target creature's *current* power — read BEFORE the pump is
    /// applied so the doubling uses the pre-pump value (CR 613: the +X/+0 layer
    /// applies once, locking X to the power at resolution). Power is public
    /// state, so this is information-independent (network determinism) and
    /// rewind-safe (a pure function of the target's power at the resolution
    /// instant, captured into the always-logged PumpCreature undo delta).
    TargetedCardPower,

    /// The power of the card that caused the trigger (last-known information).
    ///
    /// Anax, Hardened in the Forge: `SVar:Z:TriggeredCard$CardPower` used inside
    /// a `Count$Compare Z GE4.2.1` expression — "create 2 Satyr tokens if the
    /// dying creature had power >= 4, else create 1". Resolved in
    /// `resolve_effect_placeholder` from `TriggerContext::triggered_card_power`
    /// (captured via last-known information before the card moves zones,
    /// CR 608.2g / 603.6c). Information-independent: power is a public
    /// characteristic (CR 613), so server and all clients compute the same value.
    TriggeredCardPower,

    /// Count spells cast this turn (Count$YouCastThisTurn)
    SpellsCastThisTurn,

    /// Count cards in a player's GRAVEYARD matching a filter, with an optional
    /// arithmetic post-modifier.
    ///
    /// Corresponds to Forge's `Count$ValidGraveyard <filter>[/Modifier]`.
    /// Example — Combustion Technique:
    /// `SVar:X:Count$ValidGraveyard Lesson.YouOwn/Plus.2` = "Lesson cards in
    /// your graveyard plus 2". The filter resolves against the controller of the
    /// spell being cast (the `controller` passed to `evaluate_count_expression`).
    /// Graveyard contents are public information (CR 400.2), so this is
    /// information-independent and safe for network determinism.
    ValidGraveyard {
        /// The filter string (e.g., `"Lesson.YouOwn"`).
        filter: String,
        /// Arithmetic post-modifier applied to the raw count (`/Plus.N`).
        modifier: CountModifier,
    },

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

    /// The number of times Multikicker was paid for this spell (Count$TimesKicked).
    ///
    /// Corresponds to Forge's `SVar:XKicked:Count$TimesKicked` — e.g. Everflowing
    /// Chalice uses `K:etbCounter:CHARGE:XKicked` with this SVar so it enters with
    /// one CHARGE counter per kicker payment. Resolved at effect-execution time from
    /// `Card::times_kicked`, which is set by the priority loop when the caster pays
    /// Multikicker one or more times (CR 702.33a).
    TimesKicked,

    /// Conditional value based on whether the spell was kicked (Count$Kicked.true_val.false_val)
    ///
    /// Corresponds to Forge's `Count$Kicked.5.2` = "5 if kicked, 2 if not".
    /// The resolution requires the kicker-paid flag on the spell being resolved.
    /// Since we don't yet track kicker state at resolution time, this evaluates
    /// to `false_value` (unkicked) as a safe default.
    Kicked {
        /// Value if the spell was kicked
        kicked_value: i32,
        /// Value if the spell was NOT kicked (default)
        unkicked_value: i32,
    },

    /// Conditional value based on whether the spell was bargained (Count$Bargain.true_val.false_val)
    ///
    /// Corresponds to Forge's `Count$Bargain.3.2` = "3 if bargained, 2 if not".
    /// Bargain (The Wilds of Eldraine mechanic, CR 702.162) lets you sacrifice an
    /// artifact, enchantment, or token as an optional additional cost when casting
    /// the spell. Used by Torch the Tower's `SVar:X:Count$Bargain.3.2`.
    ///
    /// Since we don't yet track bargain-payment state at resolution time, this
    /// evaluates to `unbargained_value` as a conservative correct default — the
    /// spell always deals at least its base (non-bargained) amount.
    Bargain {
        /// Value if the spell was bargained (optional sacrifice paid)
        bargained_value: i32,
        /// Value if the spell was NOT bargained (default)
        unbargained_value: i32,
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

/// Arithmetic post-modifier on a `Count$...` expression.
///
/// Forge appends `/Minus.N`, `/Plus.N`, etc. to a count to shift the raw value
/// (e.g. Black Vise's `Count$ValidHand Card.ChosenCtrl/Minus.4`). We model the
/// two arithmetic forms we need today; `None` is the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountModifier {
    /// No modifier — the raw count is used as-is.
    #[default]
    None,
    /// Subtract N from the raw count (`/Minus.N`).
    Minus(i32),
    /// Add N to the raw count (`/Plus.N`).
    Plus(i32),
    /// Multiply the raw count by N (`/Times.N`).
    /// Used by per-Shrine life gain: `Count$Valid Shrine.YouCtrl/Times.2`
    /// means "count Shrines I control, then multiply by 2" (CR 700.4:
    /// "for each" iterates the counted objects; the /Times.N suffix is
    /// Forge's compact encoding of that per-copy multiplication).
    Times(i32),
}

impl CountModifier {
    /// Parse the suffix after the `/` in a `Count$...` value (e.g. `Minus.4`).
    /// Returns `None` (the variant) for unrecognized suffixes so the count is
    /// left unmodified rather than silently zeroed.
    pub fn parse(suffix: &str) -> Self {
        if let Some(rest) = suffix.strip_prefix("Minus.") {
            rest.parse()
                .ok()
                .map(CountModifier::Minus)
                .unwrap_or(CountModifier::None)
        } else if let Some(rest) = suffix.strip_prefix("Plus.") {
            rest.parse()
                .ok()
                .map(CountModifier::Plus)
                .unwrap_or(CountModifier::None)
        } else if let Some(rest) = suffix.strip_prefix("Times.") {
            rest.parse()
                .ok()
                .map(CountModifier::Times)
                .unwrap_or(CountModifier::None)
        } else {
            CountModifier::None
        }
    }

    /// Apply the modifier to a raw count value.
    pub fn apply(self, value: i32) -> i32 {
        match self {
            CountModifier::None => value,
            CountModifier::Minus(n) => value - n,
            CountModifier::Plus(n) => value + n,
            CountModifier::Times(n) => value * n,
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
            // `Targeted$CardPower` — the targeted creature's current power
            // (Berserk: NumAtt$ +X => +(target power)/+0, power-doubling).
            // This is NOT a Count$ expression; it resolves against the effect's
            // own target at execution time, so it has no `Count$` prefix.
            if svar_value == "Targeted$CardPower" {
                return CountExpression::TargetedCardPower;
            }
            // `TriggeredCard$CardPower` — the power of the card that fired the
            // trigger (last-known information). Used in Anax, Hardened in the
            // Forge: `SVar:Z:TriggeredCard$CardPower` inside a
            // `Count$Compare Z GE4.2.1` expression. Resolved in
            // `resolve_effect_placeholder` from `TriggerContext::triggered_card_power`.
            if svar_value == "TriggeredCard$CardPower" {
                return CountExpression::TriggeredCardPower;
            }
            // Parse Count$ expressions
            if let Some(rest) = svar_value.strip_prefix("Count$") {
                if rest == "xPaid" {
                    return CountExpression::XPaid;
                } else if rest == "TimesKicked" {
                    return CountExpression::TimesKicked;
                } else if let Some(hand_rest) = rest.strip_prefix("ValidHand ") {
                    // Count$ValidHand <selector>[/Modifier]
                    // (Black Vise: "Count$ValidHand Card.ChosenCtrl/Minus.4").
                    // Split the optional "/Modifier" arithmetic suffix off the
                    // selector. The selector keeps any inner dots (Card.ChosenCtrl);
                    // only the FIRST "/" introduces the modifier.
                    let (selector, modifier) = match hand_rest.split_once('/') {
                        Some((sel, suffix)) => (sel.to_string(), CountModifier::parse(suffix)),
                        None => (hand_rest.to_string(), CountModifier::None),
                    };
                    return CountExpression::CardsInHand { selector, modifier };
                } else if let Some(graveyard_rest) = rest.strip_prefix("ValidGraveyard ") {
                    // Count$ValidGraveyard <filter>[/Modifier]
                    // (Combustion Technique: "Count$ValidGraveyard Lesson.YouOwn/Plus.2").
                    // Split the optional "/Modifier" arithmetic suffix off the filter.
                    let (filter, modifier) = match graveyard_rest.split_once('/') {
                        Some((f, suffix)) => (f.to_string(), CountModifier::parse(suffix)),
                        None => (graveyard_rest.to_string(), CountModifier::None),
                    };
                    return CountExpression::ValidGraveyard { filter, modifier };
                } else if rest.starts_with("Valid ") {
                    // Count$Valid filter[/Modifier]
                    // Examples:
                    //   Count$Valid Artifact.OppCtrl        → filter="Artifact.OppCtrl", modifier=None
                    //   Count$Valid Shrine.YouCtrl/Times.2  → filter="Shrine.YouCtrl", modifier=Times(2)
                    let raw = rest.strip_prefix("Valid ").unwrap_or(rest);
                    let (filter, modifier) = match raw.split_once('/') {
                        Some((f, suffix)) => (f.to_string(), CountModifier::parse(suffix)),
                        None => (raw.to_string(), CountModifier::None),
                    };
                    return CountExpression::ValidPermanents { filter, modifier };
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
                } else if let Some(kicked_rest) = rest.strip_prefix("Kicked.") {
                    // Count$Kicked.TrueValue.FalseValue
                    // Example: "Kicked.5.2" = 5 if kicked, 2 if not
                    let parts: Vec<&str> = kicked_rest.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        if let (Ok(kicked_val), Ok(unkicked_val)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                            return CountExpression::Kicked {
                                kicked_value: kicked_val,
                                unkicked_value: unkicked_val,
                            };
                        }
                    }
                } else if let Some(bargain_rest) = rest.strip_prefix("Bargain.") {
                    // Count$Bargain.BargainedValue.UnbargainedValue
                    // Example: "Bargain.3.2" = 3 if bargained, 2 if not (Torch the Tower)
                    // Bargain (CR 702.162): optional sacrifice of artifact/enchantment/token at cast time.
                    let parts: Vec<&str> = bargain_rest.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        if let (Ok(bargained_val), Ok(unbargained_val)) =
                            (parts[0].parse::<i32>(), parts[1].parse::<i32>())
                        {
                            return CountExpression::Bargain {
                                bargained_value: bargained_val,
                                unbargained_value: unbargained_val,
                            };
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

/// Restrictions on what types of permanents (or spells on the stack) can be targeted
///
/// For spells like Disenchant ("destroy target artifact or enchantment"),
/// this would contain [Artifact, Enchantment].
/// For Terror ("destroy target creature"), this would contain [Creature].
/// An empty vec means any permanent/spell is valid.
///
/// Also used for CounterSpell spell restrictions:
/// - `requires_noncreature` encodes `ValidTgts$ Card.nonCreature` (Negate)
/// - `min_cmc` encodes `ValidTgts$ Card.cmcGE4` (Disdainful Stroke)
/// - `types` with Creature/Artifact/Enchantment encodes type-specific counters (Essence Scatter, Annul)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRestriction {
    /// Valid target types (if empty, any permanent/spell is valid)
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
    /// Required *originating set* of the card, from a `set<CODE>` qualifier
    /// (e.g. `Permanent.setARN`, `Card.setARN`). `None` = no set restriction.
    /// A card matches only if its earliest printing (`Card::origin_set`) equals
    /// this code. General machinery for any "originally printed in the <SET>
    /// expansion" card — City in a Bottle (`setARN`), Apocalypse Chime, etc.
    #[serde(default)]
    pub required_set: Option<crate::core::SetCode>,
    /// If true, the matched card must NOT be the effect's own source — the
    /// `Other` qualifier (e.g. `Permanent.Other`). Self-exclusion needs the
    /// source CardId, so plain [`TargetRestriction::matches`] ignores this flag;
    /// callers that know the source must use [`TargetRestriction::matches_excluding`].
    #[serde(default)]
    pub requires_other: bool,
    /// Required *subtype* of the card, from a bare subtype base-type in the
    /// filter (e.g. `ValidCards$ Plains`, `ValidCards$ Island`, `ValidTgts$
    /// Goblin`). `None` = no subtype restriction. A card matches only if its
    /// `subtypes` list contains this subtype. This is what makes Flashfires
    /// (`Destroy all Plains`) and Tsunami (`Destroy all Islands`) hit only the
    /// named land subtype instead of falling through to "match every permanent".
    #[serde(default)]
    pub required_subtype: Option<crate::core::Subtype>,
    /// Dynamic "power ≤ X" where X is the EFFECT SOURCE's current power, from a
    /// `powerLEX` qualifier (`ValidTgts$ Creature.powerLEX` — Old Man of the
    /// Sea: "target creature with power less than or equal to CARDNAME's
    /// power"). [`TargetRestriction::matches`] cannot evaluate this (it has no
    /// source), so the targeting site must call
    /// [`TargetRestriction::matches_with_source_power`] when this is set.
    #[serde(default)]
    pub power_le_source: bool,
    /// If true, target must NOT be a creature spell (Negate: `ValidTgts$ Card.nonCreature`).
    /// Checked at the CounterSpell targeting site against the spell on the stack.
    #[serde(default)]
    pub requires_noncreature: bool,
    /// Minimum mana value (CMC) requirement for a spell on the stack
    /// (Disdainful Stroke: `ValidTgts$ Card.cmcGE4`).
    /// `None` means no minimum CMC restriction.
    #[serde(default)]
    pub min_cmc: Option<u8>,
    /// If true, target creature must have the Defender keyword (CR 702.6).
    ///
    /// Corresponds to the `withDefender` qualifier in `ValidTgts$` /
    /// `ValidCards$` (e.g. `Creature.withDefender+YouCtrl` for Overgrown
    /// Battlement's mana ability, `Creature.withDefender` for Clear a Path).
    /// Checked via `card.has_keyword(Keyword::Defender)`.
    #[serde(default)]
    pub requires_defender: bool,
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
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            requires_defender: false,
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
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            requires_defender: false,
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
        if let Some(set) = &self.required_set {
            desc.push_str(&format!(" printed in {set}"));
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

        // Check noncreature restriction (e.g. Card.nonCreature)
        if self.requires_noncreature && card.is_creature() {
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

        // Check set-origin restriction (e.g. City in a Bottle's `setARN`).
        // Matches only cards whose EARLIEST printing is the named set. A card
        // with no known origin set (tokens, custom cards) never matches a
        // set-origin filter.
        if let Some(set) = &self.required_set {
            if !card.is_from_set(set) {
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

        // Check subtype restriction (e.g. Flashfires `Plains`, Tsunami `Island`).
        // A card matches only if its subtype list contains at least one of the required subtypes (which may be comma-separated).
        if let Some(subtype) = &self.required_subtype {
            let sub_str = subtype.as_str();
            let matches_any = sub_str.split(',').any(|s| {
                let s_subtype = crate::core::Subtype::new(s.trim());
                card.subtypes.contains(&s_subtype)
            });
            if !matches_any {
                return false;
            }
        }

        // Check minimum CMC restriction (Disdainful Stroke: Card.cmcGE4)
        if let Some(min) = self.min_cmc {
            if card.mana_cost.cmc() < min {
                return false;
            }
        }

        // Check `withDefender` — target must have the Defender keyword (CR 702.6).
        // Overgrown Battlement, Axebane Guardian, Clear a Path, etc.
        if self.requires_defender && !card.has_keyword(crate::core::Keyword::Defender) {
            return false;
        }

        // Check type restriction
        if self.types.is_empty() {
            return true; // No type restriction
        }
        self.types.iter().any(|t| t.matches(card))
    }

    /// True when this restriction matches ANY card regardless of its identity
    /// — i.e. every field is at its permissive default (no type / controller /
    /// power / color / set / token / counter / artifact / remembered / other
    /// constraint). This is the `ChangeType$ Card` / unqualified filter used by
    /// mass shuffle-back effects (Timetwister, Wheel of Fortune, Windfall,
    /// Mnemonic Nexus).
    ///
    /// Used by `Effect::ChangeZoneAll` on a SHADOW game: the opponent's hidden
    /// hand cards are late-bound reserved CardIds with no instance, so their
    /// identity cannot be inspected — but if the filter matches any card they
    /// must still be moved (otherwise the opponent's library ends up short and
    /// its subsequent shuffle consumes a different amount of RNG than the
    /// server's, breaking deterministic-simulation lockstep — mtg-728 sig-2c).
    pub fn is_unrestricted(&self) -> bool {
        self.types.is_empty()
            && self.controller == ControllerRestriction::Any
            && self.power_ge.is_none()
            && self.power_le.is_none()
            && !self.requires_no_counters
            && !self.requires_nontoken
            && !self.requires_remembered
            && !self.requires_nonartifact
            && self.required_color.is_none()
            && self.required_set.is_none()
            && !self.requires_other
    }

    /// Like [`TargetRestriction::matches`] but also honors the `Other`
    /// self-exclusion qualifier against a known effect source.
    ///
    /// `source` is the CardId of the permanent whose ability is doing the
    /// filtering (e.g. the City in a Bottle resolving the sweep). When
    /// `requires_other` is set, the candidate `card` is rejected if it IS the
    /// source. Use this at any mass-effect site that has the source available;
    /// `matches` (no source) treats `Other` as a no-op for back-compat with
    /// callers that genuinely have no source (none today filter on `Other`).
    pub fn matches_excluding(&self, card: &crate::core::Card, source: crate::core::CardId) -> bool {
        if self.requires_other && card.id == source {
            return false;
        }
        self.matches(card)
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
    /// - "Card.nonCreature" -> requires_noncreature=true (Negate: counter any noncreature spell)
    /// - "Card.cmcGE4" -> min_cmc=4 (Disdainful Stroke: counter spells with CMC >= 4)
    pub fn parse(valid_tgts: &str) -> Self {
        let mut types = SmallVec::new();
        let mut requires_no_counters = false;
        let mut requires_nontoken = false;
        let mut requires_remembered = false;
        let mut requires_nonartifact = false;
        let mut requires_noncreature = false;
        let mut requires_defender = false;
        let mut min_cmc = None;
        let mut controller = ControllerRestriction::Any;
        let mut power_ge = None;
        let mut power_le = None;
        let mut required_color = None;
        let mut required_set = None;
        let mut requires_other = false;
        let mut required_subtype: Option<crate::core::Subtype> = None;
        let mut power_le_source = false;

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
                        "nonCreature" => requires_noncreature = true,
                        "Other" => requires_other = true,
                        // `withDefender` — target must have the Defender keyword
                        // (CR 702.6). Used by Overgrown Battlement's mana
                        // ability, Clear a Path, Axebane Guardian, etc.
                        "withDefender" => requires_defender = true,
                        m if m.starts_with("cmcGE") => {
                            // Parse cmcGE4 -> min_cmc = 4 (Disdainful Stroke)
                            if let Ok(n) = m.trim_start_matches("cmcGE").parse::<u8>() {
                                min_cmc = Some(n);
                            }
                        }
                        // Set-origin qualifier `set<CODE>` (e.g. `setARN`):
                        // matches a card whose earliest printing is that set.
                        m if m.starts_with("set") && m.len() > 3 => {
                            required_set = Some(crate::core::SetCode::new(&m[3..]));
                        }
                        "White" => required_color = Some(crate::core::Color::White),
                        "Blue" => required_color = Some(crate::core::Color::Blue),
                        "Black" => required_color = Some(crate::core::Color::Black),
                        "Red" => required_color = Some(crate::core::Color::Red),
                        "Green" => required_color = Some(crate::core::Color::Green),
                        // Dynamic "power ≤ source's power" (Old Man of the Sea).
                        // MUST precede the numeric `powerLE` arm, since "powerLEX"
                        // also starts with "powerLE" (and "X" is not a number).
                        "powerLEX" => power_le_source = true,
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
                // Universal selectors match any permanent (no type/subtype filter).
                "" | "Any" | "Permanent" | "Card" | "Spell" => {}
                // Any other bare base-type is a SUBTYPE filter, not a card type:
                // `ValidCards$ Plains` / `Island` (basic land types — Flashfires,
                // Tsunami), `ValidTgts$ Goblin`, etc. Previously these fell through
                // to "match any", so e.g. `DestroyAll | ValidCards$ Plains` wiped
                // EVERY permanent. Match against the card's subtypes instead.
                other => {
                    if let Some(ref mut existing) = required_subtype {
                        let mut new_str = existing.as_str().to_string();
                        new_str.push(',');
                        new_str.push_str(other);
                        required_subtype = Some(crate::core::Subtype::new(&new_str));
                    } else {
                        required_subtype = Some(crate::core::Subtype::new(other));
                    }
                }
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
            required_set,
            requires_other,
            required_subtype,
            power_le_source,
            requires_noncreature,
            min_cmc,
            requires_defender,
        }
    }

    /// Like [`TargetRestriction::matches`], but also enforces a dynamic
    /// `powerLEX` threshold against the effect source's current power: the
    /// candidate's power must be ≤ `source_power` (Old Man of the Sea). When
    /// `power_le_source` is unset this is identical to `matches`.
    pub fn matches_with_source_power(&self, card: &crate::core::Card, source_power: i32) -> bool {
        if !self.matches(card) {
            return false;
        }
        if self.power_le_source && i32::from(card.current_power()) > source_power {
            return false;
        }
        true
    }
}

/// How long a one-shot `AB$ GainControl` (Threaten / Aladdin / Old Man of the
/// Sea) keeps the gained control, parsed from the card-script `LoseControl$`
/// list. Strong-typed so the resolution + the control-revert SBA pass cannot
/// confuse "permanent steal" with a duration-bounded one (CR 613 / 800.4a).
///
/// `LoseControl$` token meanings (Java Forge):
/// - `EOT`            -> [`ControlDuration::EndOfTurn`] (Threaten / Act of Treason)
/// - `LeavesPlay` / `LoseControl` (no `Untap`/`StaticCommandCheck`) ->
///   [`ControlDuration::WhileControlSource`] -- you keep control only as long as
///   you control the SOURCE permanent (Aladdin: "for as long as you control
///   Aladdin"). The control reverts on the next SBA once the source leaves the
///   battlefield or you lose control of it.
/// - absent          -> [`ControlDuration::Permanent`] (Control Magic-style one-shots).
///
/// (Old Man of the Sea's `Untap,...,StaticCommandCheck` tapped+power-comparison
/// duration is a further variant tracked under mtg-713 B1 follow-up.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControlDuration {
    /// Control is kept indefinitely (no `LoseControl$`). Default.
    #[default]
    Permanent,
    /// Control returns to the owner at end of turn (`LoseControl$ EOT`).
    /// NOTE: the EOT revert hook is still TODO (mtg-77); modeled here so the
    /// parser/resolver carry the right intent.
    EndOfTurn,
    /// Control is kept only while the activating player controls the SOURCE
    /// permanent (`LoseControl$ LeavesPlay,LoseControl`). Aladdin.
    WhileControlSource,
}

/// How a variable-amount damage effect is divided among its chosen targets.
///
/// Encodes the card-script `DivideEvenly$` parameter (CR 601.2d). Strong-typed
/// so the resolution path cannot confuse "single target, full damage" with
/// "split evenly among N targets".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DamageDivision {
    /// No division: deal the full amount to a single chosen target (default).
    #[default]
    None,
    /// `DivideEvenly$ RoundedDown` (Fireball): deal `floor(total / N)` to each of
    /// the `N` chosen targets, remainder lost.
    EvenlyRoundedDown,
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
    ///
    /// `divide` encodes the `DivideEvenly$` parameter (CR 601.2d). When it is
    /// `DamageDivision::EvenlyRoundedDown` (Fireball), the source consumes ALL
    /// chosen targets and deals `floor(x_paid / N)` to each of the `N` targets
    /// (remainder lost); when `None`, exactly one target is consumed and dealt
    /// the full `x_paid`.
    DealDamageXPaid { target: TargetRef, divide: DamageDivision },

    /// Deal damage equal to a `CountExpression` evaluated at resolution time.
    ///
    /// Used when `NumDmg$ X` refers to a `Count$...` SVar that is NOT a plain
    /// X-paid payment — for example Combustion Technique:
    /// `SVar:X:Count$ValidGraveyard Lesson.YouOwn/Plus.2`
    /// "deals damage equal to 2 plus the number of Lesson cards in your graveyard."
    ///
    /// `target` is resolved from `ValidTgts$` at cast time (like `DealDamage`).
    /// `count` is evaluated against the casting player at resolution (like
    /// `DealDamageToTriggeredPlayer`), combining both concerns cleanly.
    DealDamageDynamic { target: TargetRef, count: CountExpression },

    /// Resolved form of a `DivideEvenly$` X-damage spell (Fireball). Produced at
    /// resolution from `DealDamageXPaid { divide: EvenlyRoundedDown }` once the
    /// chosen targets and `x_paid` are known: deals `amount_each` to every
    /// listed target. Never parsed, logged, or sent on the wire — it is an
    /// internal resolved effect, like the concrete forms `DealDamageXPaid`
    /// resolves into.
    DealDamageDivided {
        /// All chosen targets (creatures and/or player sentinels), each dealt
        /// `amount_each` damage. Order is the controller's chosen order.
        targets: smallvec::SmallVec<[TargetRef; 4]>,
        /// `floor(x_paid / N)` damage dealt to each target (remainder lost).
        amount_each: i32,
    },

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

    /// Sacrifice the source card itself ("sacrifice CARDNAME").
    ///
    /// Used for `DB$ Sacrifice` with no `SacValid$` — the card sacrifices
    /// itself, optionally wrapped in `UnlessCostWrapper` (Stasis, Aura Flux,
    /// Arcades Sabboth: "at the beginning of your upkeep, sacrifice CARDNAME
    /// unless you pay {cost}"). The `source` field is a placeholder
    /// (`CardId::new(0)`) when stored on the trigger; the phase-trigger
    /// executor resolves it to the actual source `CardId` at fire time.
    SacrificeSelf {
        /// The card to sacrifice (placeholder until resolved at trigger time)
        source: crate::core::CardId,
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
        /// How long control is kept (parsed from `LoseControl$`). See
        /// [`ControlDuration`].
        duration: ControlDuration,
        /// Type/controller restriction on the target, parsed from `ValidTgts$`
        /// (e.g. Aladdin `Artifact`, Old Man of the Sea `Creature.powerLEX`).
        /// Empty (`TargetRestriction::any`) means "any permanent", matching the
        /// historical default of "an opponent's creature".
        #[serde(default = "TargetRestriction::any")]
        restriction: TargetRestriction,
        /// The permanent whose continued control by `new_controller` sustains a
        /// [`ControlDuration::WhileControlSource`] grant (Aladdin). Threaded in
        /// at resolution time (the resolving card). `None` for durations that
        /// don't depend on a source.
        #[serde(default)]
        source: Option<CardId>,
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
    /// `only_if_bargained` — when true (produced by `Condition$ Bargain` in the
    /// card script), the scry is skipped unless the source spell was bargained
    /// (i.e. `Card.bargain_paid == true`). Used by Torch the Tower's rider:
    /// "If this spell was bargained, ... you scry 1."
    ///
    /// AI heuristic: Keep spells, put excess lands on bottom
    Scry {
        player: PlayerId,
        count: u8,
        /// Only execute if the source spell was bargained (CR 702.162).
        /// Defaults to `false` (unconditional scry) for all cards except
        /// those with `Condition$ Bargain` in their scry sub-ability.
        #[serde(default)]
        only_if_bargained: bool,
    },

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
    /// `spell_restriction` restricts which spells are legal targets, parsed from
    /// `ValidTgts$`. The restriction's fields encode:
    /// - `required_color`: color-hosers (Red/Blue Elemental Blast, `Card.Blue`)
    /// - `types`: type-specific counters (Essence Scatter `Creature`, Annul `Artifact,Enchantment`)
    /// - `requires_noncreature`: Negate (`Card.nonCreature`)
    /// - `min_cmc`: Disdainful Stroke (`Card.cmcGE4`)
    ///
    /// A default (all-none) restriction means any spell is a legal target (plain Counterspell).
    CounterSpell {
        target: CardId,
        /// Restriction on which spells on the stack are legal targets.
        /// Default (all fields unset) = counter any spell.
        #[serde(default = "TargetRestriction::any")]
        spell_restriction: TargetRestriction,
        /// `RememberCounteredCMC$ True` — record the countered spell's mana
        /// value into `GameState::remembered_amount` so a chained delayed
        /// trigger (Mana Drain) can add that much mana later.
        #[serde(default)]
        remember_mana_value: bool,
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

    /// Mark a creature so that, if it would die this turn, it is exiled
    /// instead of going to the graveyard (CR 614 zone-change replacement,
    /// duration "this turn"). Used by Disintegrate's
    /// `ReplaceDyingDefined$ ThisTargetedCard.Creature` clause: "If the
    /// creature would die this turn, exile it instead." The flag lives on
    /// the creature and is cleared at end of turn; the death-destination
    /// chokepoint (`death_destination_for_card`) honors it alongside the
    /// finality-counter exile-instead rule. `target` is a placeholder /
    /// reuse-previous sentinel until resolution binds it to the chosen
    /// creature.
    ExileIfWouldDieThisTurn { target: CardId },

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

    /// Maze of Ith: prevent all combat damage this creature would deal OR receive
    /// this turn (CR 615, replacement effect "prevent all combat damage that would
    /// be dealt to and dealt by CARDNAME this turn").
    ///
    /// Corresponds to `DB$ Effect | ReplacementEffects$ RPrevent1,RPrevent2 |
    /// RememberObjects$ Targeted | ExileOnMoved$ Battlefield` which is the
    /// SubAbility of Maze of Ith's Untap activated ability.
    ///
    /// Sets `Card::prevent_all_combat_damage_this_turn` on the targeted creature;
    /// the flag is cleared at the cleanup step. `target` is `CardId::placeholder()`
    /// from the loader and is filled at resolution from the preceding UntapPermanent
    /// effect's target (via the `last_resolved_target` sub-ability chain).
    PreventAllCombatDamageThisTurn { target: CardId },

    /// Return up to N cards from a player's graveyard to their hand, where N is
    /// determined at resolution time from `GameState::remembered_cards.len()`.
    ///
    /// Corresponds to `DB$ ChangeZone | Origin$ Graveyard | Destination$ Hand |
    /// ChangeNum$ Y | ChangeType$ Card.YouOwn` where `SVar:Y:Remembered$Amount`.
    /// Used by Recall: "return a card from your graveyard to your hand for each
    /// card discarded this way" (the discard step stores cards in `remembered_cards`;
    /// the count of cards to return equals `remembered_cards.len()`).
    ///
    /// The AI picks the best cards to return using `choose_card_to_retrieve_from_graveyard`.
    /// The player is a placeholder until resolution, when it is bound to the
    /// controller of the resolving spell.
    ReturnCardsFromGraveyardToHand {
        /// Player whose graveyard to return cards from.
        player: PlayerId,
    },

    /// Put `count` cards from a player's hand on top of their library (mandatory
    /// self-selection).  The controller chooses which cards; in non-interactive
    /// fallback mode the lowest-value cards are picked deterministically.
    ///
    /// Corresponds to `DB$ ChangeZone | Origin$ Hand | Destination$ Library |
    /// ChangeNum$ <N> | Mandatory$ True | Reorder$ True` (Brainstorm's sub-ability
    /// that puts two cards from your hand on top of your library in any order).
    ///
    /// MTG CR 701.19: when an effect says to put one or more cards from a player's
    /// hand on top of that player's library, the player chooses which card(s) to put
    /// back and the order.  The effect is mandatory — the controller MUST put the
    /// specified number of cards back (if available).
    PutCardsFromHandOnTopOfLibrary { player: PlayerId, count: u8 },

    /// Reveal any number of matching cards from a player's hand, optionally
    /// storing the count in `GameState::remembered_amount` for use by
    /// chained sub-abilities (e.g. Metalworker: "reveal artifact cards; add
    /// {C}{C} for each").
    ///
    /// Corresponds to `AB$ Reveal | RevealValid$ <filter> | AnyNumber$ True |
    /// RememberRevealed$ True | SubAbility$ DBMana`.
    ///
    /// MTG CR 701.15 (Reveal): a player reveals a card by showing it to all
    /// other players.  The effect is mandatory if `AnyNumber$ False`; with
    /// `AnyNumber$ True` the controller chooses how many to reveal (≥ 0).
    RevealCardsFromHand {
        /// Player whose hand to reveal from.
        player: PlayerId,
        /// Card filter string (e.g. `"Card.Artifact+YouCtrl"`).
        filter: String,
        /// If true, the player chooses how many to reveal (≥ 0).
        any_number: bool,
        /// If true, store the revealed count in `GameState::remembered_amount`.
        remember_count: bool,
    },

    /// Return exactly one card matching a type filter from a player's graveyard
    /// to their hand.  The AI picks the highest-value matching card.
    ///
    /// Corresponds to `DB$ ChangeZone | Origin$ Graveyard | Destination$ Hand |
    /// ValidTgts$ <filter>` without a `ChangeNum$` (i.e. return exactly 1 card).
    /// Example: Stormchaser's Talent level-2 trigger: return target instant or
    /// sorcery from your graveyard to hand.
    ReturnGraveyardCardToHand {
        /// Player whose graveyard to search.
        player: PlayerId,
        /// Comma-separated card type filter (e.g. `"Instant,Sorcery"`).
        /// Empty string means any card.
        type_filter: String,
    },

    /// Return exactly one card matching a `ValidTgts$` filter from a player's
    /// graveyard to any destination zone (Library, Battlefield, etc.).
    ///
    /// Generalises `ReturnGraveyardCardToHand` for non-Hand destinations.
    ///
    /// Corresponds to `SP$/DB$ ChangeZone | Origin$ Graveyard |
    /// Destination$ <Library|Battlefield|…> | ValidTgts$ <filter>` without
    /// `ChangeNum$`. Examples:
    ///   - Reclaim: `Origin$ Graveyard | Destination$ Library | ValidTgts$ Card.YouCtrl`
    ///   - Goryo's Vengeance: `Origin$ Graveyard | Destination$ Battlefield |
    ///                         ValidTgts$ Creature.Legendary+YouCtrl | GainControl$ True`
    ///   - Debtors' Knell trigger: `Origin$ Graveyard | Destination$ Battlefield |
    ///                              ValidTgts$ Creature | GainControl$ True`
    ///
    /// `gain_control` mirrors `GainControl$ True` — puts the card under the
    /// caster's control (reanimation) rather than its owner's. MTG CR 701.3:
    /// a player puts a card onto the battlefield under their control unless
    /// stated otherwise.
    ///
    /// The `player` is a placeholder until resolution (bound to the spell/
    /// trigger's controller). The AI picks the highest-value matching card from
    /// the graveyard of `player` (for `YouCtrl` effects) or any player's
    /// graveyard (for `ValidTgts$ Creature` without ownership restriction).
    ReturnGraveyardCardToZone {
        /// Controller of the spell/ability that triggered this effect.
        /// `PlayerId::placeholder()` until resolved at spell resolution time.
        player: PlayerId,
        /// Comma-separated card type filter (e.g. `"Creature.Legendary"`, `"Card"`).
        /// Empty string means any card.
        type_filter: String,
        /// Destination zone (Library, Battlefield, Hand — anything except Graveyard).
        destination: crate::zones::Zone,
        /// If true, the card enters the battlefield under the caster's control
        /// regardless of who owns it (`GainControl$ True`). Used for reanimation.
        gain_control: bool,
        /// Library position for `Destination$ Library`:
        /// `0` = top (Reclaim puts the card on TOP of library, CR 401.4),
        /// `1` = bottom.  Ignored for non-Library destinations.
        library_position: u8,
    },

    /// Return a card that just died from the graveyard to the battlefield,
    /// but only as an enchantment (removing all creature types).
    ///
    /// Corresponds to Enduring Vitality's death trigger:
    ///   `DB$ ChangeZone | Defined$ TriggeredNewCardLKICopy | Origin$ Graveyard
    ///    | Destination$ Battlefield | StaticEffect$ Animate`
    /// with Animate: `Mode$ Continuous | Affected$ Card.IsRemembered |
    ///   AddType$ Enchantment | RemoveCardTypes$ True`
    ///
    /// Semantics (CR 400.7, CR 110.5c):
    /// 1. Find the source card in the graveyard (by CardId).
    /// 2. Move it from Graveyard → Battlefield under its owner's control.
    /// 3. Remove all card types that are not Enchantment (strip Creature type etc.)
    ///    and ensure it has the Enchantment card type — so the resulting permanent
    ///    is purely an enchantment and will not re-trigger "when this creature dies."
    ///
    /// Cards using this:
    ///   - Enduring Vitality: "When Enduring Vitality dies, if it was a creature,
    ///     return it to the battlefield under its owner's control. It's an enchantment."
    ReturnSelfAsEnchantment {
        /// The card that died and should be returned. Resolved from the dying card's
        /// CardId in `check_death_triggers`; placeholder (CardId::new(0)) until then.
        source: CardId,
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

    /// Create a planeswalker emblem and place it in the controller's command zone
    /// (CR 113.2 — emblems are objects with abilities, placed in the command zone,
    /// that persist for the rest of the game and can never be removed).
    ///
    /// Corresponds to: `AB$ Effect | StaticAbilities$ X | Duration$ Permanent` or
    /// `AB$ Effect | Triggers$ X | Duration$ Permanent` (with `Planeswalker$ True`).
    ///
    /// At resolution the engine mints a synthetic Card in the controller's command
    /// zone with the given static abilities and/or triggers. The existing continuous-
    /// effects system and phase-trigger scanning already check the command zone, so
    /// no further special-casing is needed — the emblem acts exactly like a permanent
    /// with those abilities, except it lives in the command zone instead of the
    /// battlefield (CR 113.4: emblems have no controller; we track the creating
    /// player as the owner for scoping purposes).
    CreateEmblem {
        /// The player who created the emblem (becomes the "controller" for scoping
        /// static abilities such as "creatures you control get …")
        controller: PlayerId,
        /// Human-readable emblem name (e.g. "Emblem — Elspeth, Sun's Champion")
        emblem_name: String,
        /// Static abilities on the emblem (e.g. +2/+2 + flying to your creatures)
        static_abilities: Vec<StaticAbility>,
        /// Triggered abilities on the emblem (e.g. "at the beginning of each
        /// opponent's upkeep, that player sacrifices a creature")
        triggers: Vec<Trigger>,
    },

    /// Create token(s) whose count is determined at trigger-resolution time from
    /// a dynamic amount (e.g. the number of counters on the triggering card).
    ///
    /// Corresponds to: DB$ Token | TokenAmount$ Y | TokenScript$ c_1_1_a_thopter_flying
    /// where SVar:Y:TriggeredCard$CardCounters.P1P1.
    ///
    /// This is the `CreateToken` shape with a `DynamicAmount` instead of a
    /// static `u8`. `resolve_effect_placeholder` converts it to `CreateToken`
    /// with the concrete amount filled in from the `TriggerContext`, after which
    /// `execute_effect` processes it via the normal `CreateToken` path.
    CreateTokenDynamic {
        /// Player who will control the tokens (ignored if for_each_player is true)
        controller: PlayerId,
        /// Token script name (e.g., "c_1_1_a_thopter_flying" for Thopter token)
        token_script: String,
        /// Dynamic amount resolved at trigger-fire time
        amount: DynamicAmount,
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

    /// Advance a Class enchantment to the next level.
    ///
    /// Class enchantments (e.g. Stormchaser's Talent, Barbarian Class) have
    /// level-up activated abilities that can be paid at sorcery speed.  When
    /// the ability resolves this effect fires, which:
    ///
    /// 1. Increments the `CounterType::Level` counter on the Class card.
    /// 2. Fires any one-time `ClassLevelGained` triggers at that level.
    /// 3. Attaches any ongoing triggers / statics defined by `AddTrigger$`
    ///    or `AddStaticAbility$` at that level to the card permanently.
    ///
    /// `class_card_id` is the permanent being leveled up; `target_level` is
    /// the level being reached (e.g. `2` for the first paid activation).
    ClassLevelUp {
        class_card_id: crate::core::CardId,
        target_level: u8,
    },

    /// Grant one-time permission to cast a targeted instant/sorcery from the
    /// graveyard this turn (CR 400.7 + CR 614 replacement).
    ///
    /// Corresponds to `A:AB$ Play | TgtZone$ Graveyard | ...` on planeswalkers
    /// such as Chandra, Acolyte of Flame (−2 loyalty).
    ///
    /// When executed:
    /// 1. Creates a `PersistentEffectKind::CastTargetedSpellFromGraveyard` that
    ///    tracks the chosen card and grants cast permission for the rest of the turn.
    /// 2. If `exile_on_resolution` is true, also sets
    ///    `Card::exile_if_would_go_to_graveyard_this_turn` on the targeted card,
    ///    so that `resolve_spell_finalize` sends it to exile instead of the
    ///    graveyard when it resolves (CR 614 zone-change replacement).
    ///
    /// The `target` is `CardId::placeholder()` until binding at cast time.
    PlayFromGraveyard {
        /// The graveyard card to cast. Placeholder until targeting binds it.
        target: CardId,
        /// If true, exile the card instead of putting it into the graveyard
        /// when it would resolve (the `ReplaceGraveyard$ Exile` clause).
        exile_on_resolution: bool,
        /// Comma-separated card type filter (e.g. `"Instant,Sorcery"`).
        /// The valid types come from the `ValidTgts$` parameter in the card script.
        /// Empty string means any instant/sorcery (fallback).
        type_filter: String,
        /// Maximum mana value (CMC) for the targeted card, from `cmcLE<N>` in
        /// the `ValidTgts$` qualifier. `None` = no maximum CMC restriction.
        max_mana_value: Option<u8>,
    },

    /// Look at the top N cards of `player`'s library, then put them back in any
    /// order (CR 701.22 — "look at"). In the AI-only path the order is
    /// unchanged (keeping the current top order is always a legal choice per
    /// the rules); the important effect is that the ability resolves without
    /// emitting an `Unimplemented` warning.
    ///
    /// Sensei's Divining Top: `A:AB$ RearrangeTopOfLibrary | Defined$ You | NumCards$ 3`
    RearrangeTopOfLibrary {
        /// The player who looks at (and re-orders) the top of their library.
        player: crate::core::PlayerId,
        /// Number of cards to look at (default 3).
        count: u8,
    },

    /// Cause `player` to skip their next untap step (CR 502.1).
    ///
    /// The skip flag is set on the player's `Player::skip_untap_next_turn` field
    /// and cleared by the `untap_step` handler the next time that player
    /// reaches their untap step.
    ///
    /// Yosei, the Morning Star: `DB$ SkipPhase | ValidTgts$ Player | Step$ Untap`
    SkipUntapStep {
        /// The player whose next untap step will be skipped.
        player: crate::core::PlayerId,
    },

    /// For-each loop: execute `sub_effects` once for each member of `iterate_over`.
    ///
    /// Corresponds to: `DB$ RepeatEach | RepeatSubAbility$ <svar> | DefinedCards$ Targeted`
    /// or: `A:SP$ RepeatEach | RepeatPlayers$ Player | RepeatSubAbility$ <svar>`
    ///
    /// CR 609.3: Effects that use "for each" repeat an action once per member of the
    /// named set. Actions execute sequentially; state-based actions check between each.
    ///
    /// **Pattern A — iterate over cards** (`iterate_over = Cards { .. }`):
    /// - Iterates over `targets` (the spell's chosen targets, resolved at spell-resolution time).
    /// - If `require_in_graveyard` is true (`ChangeZoneTable$ True`), only includes cards
    ///   that are currently in a graveyard or exiled zone (cards that were NOT actually
    ///   destroyed — e.g., indestructible permanents — are skipped).
    /// - For each qualifying card: sets `game.remembered_cards = [card_id]`, then executes
    ///   each effect in `sub_effects`.
    ///
    /// **Pattern B — iterate over players** (`iterate_over = AllPlayers`):
    /// - Iterates over all players in the current turn order.
    /// - For each player: sets `game.remembered_players = [player_id]`, then executes
    ///   each effect in `sub_effects`.
    ///
    /// Used by: Terastodon (token per destroyed permanent), Tragic Arrogance (player loop).
    RepeatEach {
        /// Effects to execute once per member (from `RepeatSubAbility$` chain,
        /// including any `SubAbility$` chains chained off the RepeatEach itself)
        sub_effects: Vec<Effect>,
        /// What to iterate over (resolved at parse/resolve time)
        iterate_over: RepeatEachIterate,
    },

    /// Placeholder for a recognized but unimplemented effect
    /// Produced instead of silently dropping the effect, so that spell resolution
    /// can warn/error instead of silently no-op'ing.
    Unimplemented {
        /// The API type name that was not implemented
        api_type: String,
    },

    /// A recognized effect that INTENTIONALLY does nothing in this engine,
    /// because its game-relevant outcome is modeled elsewhere. Distinct from
    /// `Unimplemented` (which signals a real gap and logs a warning) — `NoOp`
    /// is silent and expected.
    ///
    /// Used for Forge's `DB$ StoreSVar`: a pseudo-API that stashes a computed
    /// value into a card SVar for a later effect to read. Drain Life uses it to
    /// compute its life-gain cap (`Limit` = target toughness/loyalty/life), but
    /// this engine reads that cap directly from the target snapshot
    /// (DynamicAmount::DamageDealtCappedByTarget) rather than maintaining a
    /// runtime SVar store, so the StoreSVar nodes themselves do nothing.
    NoOp {
        /// The API type name, for debug logging only.
        api_type: String,
    },

    /// Grant the player permission to play an additional land this turn.
    ///
    /// Corresponds to: `A:SP$ Effect | StaticAbilities$ Exploration` where
    /// `SVar:Exploration:Mode$ Continuous | Affected$ You | AdjustLandPlays$ 1`
    ///
    /// The temporary grant (spell/trigger path) creates a `PersistentEffectKind::ExtraLandPlay`
    /// cleaned up at end of turn. The permanent form (Oracle of Mul Daya, etc.) uses
    /// `StaticAbility::ExtraLandPlay` on the on-battlefield card instead.
    ///
    /// `player` may be `PlayerId::new(0)` as a placeholder when created from a
    /// `StaticAbilities$` SVar — resolved to the spell's controller at execution time
    /// in `execute_effect`.
    ExtraLandPlay {
        /// The player who gains the additional land play.
        player: PlayerId,
        /// Number of extra land plays (usually 1).
        amount: u8,
    },
}

/// What a `RepeatEach` effect iterates over.
///
/// Resolved at parse time (for `AllPlayers`) or at spell-resolution time (for
/// `Cards`). The distinction matters because chosen targets are known only when
/// the spell resolves, whereas all-players is always available.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RepeatEachIterate {
    /// Iterate over the spell's chosen targets.
    ///
    /// `targets` is populated at `resolve_effect_target` time from `chosen_targets`.
    /// `require_in_graveyard` is true when `ChangeZoneTable$ True` is set —
    /// only cards currently in the graveyard (i.e., successfully destroyed) are
    /// iterated; permanents that survived (indestructible, replaced, etc.) are skipped.
    Cards {
        /// The resolved chosen targets for this spell (filled in at resolution time)
        targets: Vec<CardId>,
        /// If true, only iterate over cards that are now in a graveyard zone
        /// (`ChangeZoneTable$ True`).
        require_in_graveyard: bool,
    },
    /// Iterate over all players in turn order (`RepeatPlayers$ Player`).
    AllPlayers,
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

/// Intervening-if condition evaluated against the trigger source card itself
/// (CR 603.4). Encodes the `IsPresent$ Card.Self+<filter>` family of self-state
/// gates that suppress a trigger when the source no longer meets the printed
/// "if" clause at the moment it would trigger.
///
/// Two shapes are supported:
/// - `Counter`: `IsPresent$ Card.Self+counters_<CMP><N>_<TYPE>` (All Hallow's
///   Eve: at least one SCREAM counter).
/// - `Tapped` / `Untapped`: `IsPresent$ Card.untapped` / `Card.tapped`
///   (Howling Mine: "if CARDNAME is untapped, that player draws an additional
///   card" — CR 603.4 the source must be untapped right now).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PresentSelfCondition {
    /// The source must satisfy a counter-count comparison.
    Counter(SelfCounterCondition),
    /// The source must currently be untapped (`IsPresent$ Card.untapped`).
    Untapped,
    /// The source must currently be tapped (`IsPresent$ Card.tapped`).
    Tapped,
}

impl PresentSelfCondition {
    /// Parse the `IsPresent$` value of a phase/self trigger into a self-state
    /// intervening-if condition, if it encodes one we model.
    ///
    /// Recognized clauses (split on `.`/`+`):
    /// - `counters_<CMP><N>_<TYPE>` -> `Counter` (All Hallow's Eve).
    /// - `untapped` -> `Untapped` (Howling Mine).
    /// - `tapped`   -> `Tapped`.
    ///
    /// Returns `None` when the `IsPresent$` value does not contain a clause we
    /// understand (the trigger then carries no self-state intervening-if).
    pub fn parse(is_present: &str) -> Option<Self> {
        is_present.split(['.', '+']).find_map(|clause| match clause {
            "untapped" => Some(PresentSelfCondition::Untapped),
            "tapped" => Some(PresentSelfCondition::Tapped),
            _ => clause
                .strip_prefix("counters_")
                .and_then(SelfCounterCondition::parse_clause)
                .map(PresentSelfCondition::Counter),
        })
    }
}

/// Source of the spell to copy for CopySpellAbility
///
/// Corresponds to the `Defined$` parameter in DB$ CopySpellAbility
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CopySpellSource {
    /// Copy a SEPARATELY-TARGETED spell or ability on the stack (the
    /// "copy target instant/sorcery/activated/triggered ability" class —
    /// Twincast, Reverberate, Fork, Strionic Resonator, Return the Favor, ...).
    /// These scripts have NO `Defined$`; they carry `TargetType$`/`ValidTgts$`
    /// naming the OTHER spell/ability to copy. This mechanic (clone an arbitrary
    /// targeted stack object) is NOT yet implemented, so it resolves as a SAFE
    /// NO-OP.
    ///
    /// This is the DEFAULT for a bare `CopySpellAbility`. It must NEVER fall back
    /// to `Parent`: a `Parent` self-copy of one of these cards copies ITSELF, and
    /// because the copy carries the same self-copy mode it copies itself again —
    /// an INFINITE self-replication loop (the commander-format hang, where
    /// Return the Favor span forever). Only an explicit `Defined$ Parent`
    /// (Chain Lightning) is a real parent self-copy, and that one terminates via
    /// its `{R}{R}` UnlessCost gate.
    #[default]
    TargetedSpell,
    /// Copy the parent spell (the current spell on the stack that has this as SubAbility)
    /// Used by Chain Lightning: "copy this spell"
    /// Corresponds to: Defined$ Parent
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
            | Effect::CreateTokenDynamic { .. }
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
            // DealDamageDynamic IS a targeted effect (target collected at cast
            // time), so it falls through to the default `true` return below.
            | Effect::DealDamageToTriggeredPlayer { .. }
            | Effect::SelfExileFromStack { .. }
            // Self-zone-move and conditional-self wrappers operate on the source
            // card (Defined$ Self) — no cast-time target collection needed.
            | Effect::MoveSelfBetweenZones { .. }
            | Effect::ReturnCardsFromGraveyardToHand { .. }
            | Effect::PutCardsFromHandOnTopOfLibrary { .. }
            | Effect::RevealCardsFromHand { .. }
            | Effect::PreventAllCombatDamageThisTurn { .. }
            | Effect::ConditionalSelfCounter { .. }
            // Clone chooses which permanent to copy at resolution time (ETB
            // replacement), routed through the controller — there is no
            // cast-time target on the Copy Artifact spell itself.
            | Effect::Clone { .. }
            // ExileIfWouldDieThisTurn rides on the parent DealDamage's chosen
            // target (Disintegrate's ReplaceDyingDefined clause); it never
            // collects its own cast-time target.
            | Effect::ExileIfWouldDieThisTurn { .. }
            // ClassLevelUp targets the Class card itself (self-resolving), not a
            // cast-time target — the class_card_id is baked in at ability creation.
            | Effect::ClassLevelUp { .. }
            | Effect::ReturnGraveyardCardToHand { .. }
            | Effect::ReturnGraveyardCardToZone { .. }
            | Effect::SacrificeSelf { .. }
            | Effect::ReturnSelfAsEnchantment { .. }
            | Effect::CreateEmblem { .. }
            | Effect::RearrangeTopOfLibrary { .. }
            | Effect::SkipUntapStep { .. }
            | Effect::Unimplemented { .. }
            | Effect::NoOp { .. }
            // RepeatEach targets are consumed from chosen_targets at resolve_effect_target
            // time (Pattern A) or iterates over all players (Pattern B).  Either way
            // the targeting system does not need to separately collect targets for it.
            | Effect::RepeatEach { .. }
            // ExtraLandPlay grants a land-play permission to a specific player;
            // no cast-time target is selected (player is the spell's controller).
            | Effect::ExtraLandPlay { .. } => EffectTargetCategory::NoTargetNeeded,

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
            | Effect::DealDamageDivided { .. }
            | Effect::DealDamageDynamic { .. }
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
            | Effect::PumpCreatureVariable { .. }
            // PlayFromGraveyard targets the instant/sorcery card in the graveyard.
            | Effect::PlayFromGraveyard { .. } => EffectTargetCategory::RequiresTarget,
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

/// Which combat-damage recipient class a `DealsCombatDamage` trigger watches.
///
/// Combat damage is dealt as one simultaneous event (CR 510.2). A creature's
/// `DealsCombatDamage` trigger sees that one event, but the trigger's
/// `ValidTarget$` clause restricts *which* recipients count:
///
/// - `ValidTarget$ Player` / `Opponent` / `Player,Planeswalker` -> [`Player`](Self::Player):
///   fire only when the source dealt combat damage to a player (or
///   planeswalker), amount = damage dealt to players. (Hypnotic Specter,
///   Mark of Sakiko.)
/// - `ValidTarget$ Creature` -> [`Creature`](Self::Creature): fire only when
///   the source dealt combat damage to a creature, amount = damage to
///   creatures.
/// - no `ValidTarget$` restriction (or an aggregating `DamageDealtOnce`) ->
///   [`Any`](Self::Any): fire whenever the source dealt ANY combat damage,
///   amount = total combat damage dealt to all recipients (Spirit Link's
///   lifelink, CR 119.3-style).
///
/// Replaces the dead `[any-damage]` / `[damages-creature]` description markers
/// with a structured filter consumed at the single combat-damage firing site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CombatDamageTarget {
    /// Fire only on combat damage dealt to a player/planeswalker.
    Player,
    /// Fire only on combat damage dealt to a creature.
    Creature,
    /// Fire on any combat damage dealt (default; matches Lifelink semantics).
    #[default]
    Any,
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

    /// When a creature attacks and is not blocked (fires at the end of the
    /// declare-blockers step, after all blockers are assigned).
    /// Corresponds to: T:Mode$ AttackerUnblocked | ValidCard$ Card.Self
    /// Example: Eternal of Harsh Truths — "Whenever ~ attacks and isn't blocked, draw a card."
    /// Floral Spuzzem — "Whenever ~ attacks and isn't blocked, you may destroy target
    ///                  artifact defending player controls."
    AttackerUnblocked,

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

    /// When a permanent is tapped for mana
    /// Corresponds to: T:Mode$ TapsForMana | ValidCard$ ...
    TapsForMana,

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

    /// When ANY creature dies (goes to the graveyard from the battlefield),
    /// regardless of who controls it or the trigger source.
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature[.YouCtrl/.OppCtrl/.Other]
    /// Example: Fecundity — "Whenever a creature dies, that creature's controller may draw a card."
    /// Fires from the trigger source (Fecundity), which sits on the battlefield
    /// while OTHER creatures die. `check_death_triggers` scans the battlefield
    /// for permanents carrying this trigger when any creature dies; the dying
    /// creature's controller is threaded as `TriggeredCardController` via the
    /// `TriggerContext` so `Defined$ TriggeredCardController` resolves to them.
    /// The `ValidCard$` controller qualifier (`.YouCtrl` / `.OppCtrl`) is stored
    /// so the firing site can filter by the dying creature's controller relative
    /// to the trigger source's controller (mtg-409 follow-up, mtg-913 B12).
    CreatureDies {
        /// Controller restriction on the *dying* creature, relative to the
        /// trigger source's controller: `None` = any creature, `Some(true)` =
        /// only creatures the source's controller controls (`.YouCtrl`),
        /// `Some(false)` = only creatures an opponent controls (`.OppCtrl`).
        you_control: Option<bool>,
        /// When `true`, the trigger source's own death does NOT fire the trigger
        /// (`Creature.Other`). When `false`, the source dying also counts.
        exclude_self: bool,
    },

    /// When a Class enchantment reaches a specific level.
    ///
    /// Corresponds to: T:Mode$ ClassLevelGained | ClassLevel$ N | ValidCard$ Card.Self
    ///
    /// Fires on the Class enchantment itself after `Effect::ClassLevelUp`
    /// advances the card's `CounterType::Level` counter to `level`.  Used for
    /// one-time "when this Class becomes level N" effects (e.g. Stormchaser's
    /// Talent level-2: return an instant or sorcery from your graveyard to
    /// your hand).
    ClassLevelGained {
        /// The level that was just reached.
        level: u8,
    },

    /// When a card is discarded
    /// Corresponds to: T:Mode$ Discarded | ValidCard$ Card.YouOwn | TriggerZones$ Battlefield
    /// Example: Monument to Endurance — "Whenever you discard a card, choose one..."
    /// Fires from any permanent on the battlefield when its controller (or any
    /// player matching ValidCard$) discards a card.
    CardDiscarded,

    /// When THIS card is itself discarded (the discarded card is the trigger
    /// source), fired on its last-known information as it moves Hand→Graveyard
    /// (CR 603.6/603.10 — a leaves-the-zone trigger looking back at the object).
    ///
    /// Corresponds to: T:Mode$ Discarded | ValidCard$ Card.Self
    ///   | ValidCause$ SpellAbility.OppCtrl | Execute$ ...
    /// Example: Psychic Purge — "When a spell or ability an opponent controls
    /// causes you to discard Psychic Purge, that player loses 5 life."
    ///
    /// Distinct from [`CardDiscarded`], which is a battlefield permanent
    /// watching its controller's discards (Monument to Endurance). This event
    /// fires on the DISCARDED CARD ITSELF and is gated by `requires_opponent_
    /// cause`: it only fires when the discard was caused by a spell/ability
    /// controlled by an OPPONENT of the card's owner (the `cause` threaded into
    /// `GameState::discard_card`), so a self-discard (cleanup, your own looting)
    /// does NOT fire it.
    Discarded,
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

    /// When true, the trigger only fires on the turn of the player CHOSEN by the
    /// source's ETB ChoosePlayer replacement (`ValidPlayer$ Player.Chosen`).
    /// Black Vise's upkeep trigger fires only on the chosen player's upkeep.
    /// The chosen player is stored in `Card::chosen_player`; the firing sites
    /// gate on `active_player == card.chosen_player`. Mutually exclusive with
    /// `controller_turn_only` in practice (a trigger is either "your" or
    /// "chosen player's" turn, not both).
    #[serde(default)]
    pub chosen_player_turn_only: bool,

    /// When true, the trigger only fires on the upkeep of the ENCHANTED
    /// permanent's controller — a DIFFERENT player than the source Aura's
    /// controller for a curse Aura (`ValidPlayer$ Player.EnchantedController`).
    /// Paralyze's "At the beginning of the upkeep of enchanted creature's
    /// controller, that player may pay {4}; if they do, untap the creature."
    /// The firing sites gate on `active_player == cards[aura.attached_to].
    /// controller` instead of `active_player == aura.controller`. If the Aura
    /// is not attached (no `attached_to`), the trigger cannot fire.
    #[serde(default)]
    pub enchanted_controller_turn_only: bool,

    /// For [`TriggerEvent::Discarded`] self-triggers: when true the trigger
    /// fires ONLY if the discard was caused by a spell or ability controlled by
    /// an OPPONENT of the card's owner (`ValidCause$ SpellAbility.OppCtrl`).
    /// Psychic Purge — "When a spell or ability an OPPONENT controls causes you
    /// to discard this, that player loses 5 life." The firing site
    /// (`discard_card`) consults the `cause` threaded in explicitly; if that
    /// cause is absent (a discard with no spell/ability cause, e.g. the cleanup-
    /// step over-the-limit discard) or is the card's own owner (self-discard /
    /// own looting), the trigger does NOT fire.
    #[serde(default)]
    pub requires_opponent_cause: bool,

    /// When true, trigger only fires if event source is NOT a creature
    /// Replaces "[noncreature]" marker in description
    /// Example: "Whenever you cast a noncreature spell"
    #[serde(default)]
    pub requires_noncreature: bool,

    /// When true, trigger only fires if the cast spell is an instant or sorcery.
    /// Corresponds to `ValidCard$ Instant,Sorcery` on SpellCast triggers.
    /// Example: Stormchaser's Talent level 3 "Whenever you cast an instant or sorcery spell"
    #[serde(default)]
    pub requires_instant_or_sorcery: bool,

    /// When true, the trigger fires only when the event source is the
    /// permanent this trigger's card is *attached to* (`ValidSource$
    /// Card.AttachedBy`). Used by Auras/Equipment that watch the host's
    /// actions, e.g. Spirit Link's "Whenever enchanted creature deals damage,
    /// you gain that much life." The check is `attached_to == event_source`.
    #[serde(default)]
    pub requires_attached_source: bool,

    /// For `DealsCombatDamage` triggers: which combat-damage recipient class
    /// (player vs. creature vs. any) this trigger fires on. Derived from the
    /// `ValidTarget$` clause at parse time and consumed at the single
    /// combat-damage firing site (`resolve_combat_damage`), so a player-only
    /// trigger does NOT fire when the creature only damages a blocker, while
    /// Spirit Link's any-damage lifelink fires for damage to players AND
    /// creatures. Ignored for non-`DealsCombatDamage` events.
    #[serde(default)]
    pub combat_damage_target: CombatDamageTarget,

    /// For `DealsCombatDamage` triggers: when true, the trigger fires ONLY on
    /// combat damage, never on non-combat damage (`CombatDamage$ True`, e.g.
    /// Hypnotic Specter "deals COMBAT damage to a player").
    ///
    /// `DealsCombatDamage` is the shared event for both combat and non-combat
    /// "deals damage" triggers; the two have distinct firing sites
    /// (`resolve_combat_damage` for combat, `resolve_spell_execute_effects` for
    /// non-combat). The non-combat firing site (mtg-r9po1) consults this flag to
    /// skip combat-only triggers, while the combat site fires all of them.
    /// "Whenever ~ deals damage" (no COMBAT qualifier, e.g. Spirit Link's
    /// `DamageDealtOnce`) leaves this `false` so it fires on either kind.
    #[serde(default)]
    pub requires_combat_damage: bool,

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

    /// Intervening-if condition: the source card must satisfy this self-state
    /// condition for the trigger to fire (CR 603.4).
    ///
    /// Corresponds to `IsPresent$ Card.Self+<filter>` (optionally combined with
    /// `PresentZone$`). Supported filters: a `counters_<CMP><N>_<TYPE>`
    /// counter-count (All Hallow's Eve: `IsPresent$ Card.Self+counters_GE1_SCREAM
    /// | PresentZone$ Exile`) and a tap-status check (Howling Mine: `IsPresent$
    /// Card.untapped` — "if CARDNAME is untapped"). None means no intervening-if
    /// check.
    #[serde(default)]
    pub present_self_condition: Option<PresentSelfCondition>,

    /// Intervening-if condition: the source card must have dealt damage to an
    /// opponent this turn for the trigger to fire (CR 603.4). Corresponds to
    /// `IsPresent$ Card.Self+dealtDamageToOppThisTurn` — Whirling Dervish's "at
    /// the beginning of each end step, if CARDNAME dealt damage to an opponent
    /// this turn, put a +1/+1 counter on it". Checked against the source card's
    /// `dealt_damage_to_opponent_this_turn` per-turn flag.
    #[serde(default)]
    pub present_self_dealt_damage_to_opponent: bool,

    /// For TapsForMana triggers: filter for the tapped permanent
    #[serde(default)]
    pub taps_for_mana_valid_card: Option<String>,

    /// For TapsForMana triggers: activator restriction (You, Opponent, Player.NonActive, etc.)
    #[serde(default)]
    pub taps_for_mana_activator: Option<String>,

    /// When true, trigger fires ONLY on opponents' turns, never on the
    /// controller's own turn. Corresponds to `ValidPlayer$ Player.Opponent`
    /// on upkeep/phase triggers. Example: Sorin, Solemn Visitor's emblem
    /// "At the beginning of each opponent's upkeep, that player sacrifices a
    /// creature." Without this flag the trigger would fire on ALL players'
    /// upkeeps including the controller's own (wrong). Mutually exclusive with
    /// `controller_turn_only` in practice — a trigger fires on your turns, the
    /// opponent's turns, or all turns.
    #[serde(default)]
    pub opponent_turn_only: bool,
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
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
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
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
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
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
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
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
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

    /// Continuous "destroy/sacrifice any matching permanent" sweep — the
    /// `T:Mode$ Always` state-trigger pattern (CR 603.8, applied like a
    /// state-based action). While the source permanent is on the battlefield,
    /// every battlefield permanent matching `restriction` is moved to its
    /// owner's graveyard (sacrificed). Re-checked at every state-based-action
    /// pass, so it covers BOTH the one-time on-enter sweep AND "destroy any
    /// such permanent that enters afterward" with a single rule.
    ///
    /// General machinery: City in a Bottle uses
    /// `ValidCards$ Permanent.!token+setARN+Other`, but any "whenever one or
    /// more permanents matching X are on the battlefield, sacrifice them"
    /// state-trigger maps here. The `Other` qualifier in `restriction`
    /// excludes the source itself (checked via `matches_excluding`).
    SacrificeMatchingPresent {
        /// Filter for which permanents are continuously swept.
        restriction: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Cast-prohibition static: spells matching `valid_card` can't be cast
    /// while the source is on the battlefield. Corresponds to
    /// `S:Mode$ CantBeCast | ValidCard$ <filter>` (City in a Bottle:
    /// `ValidCard$ Card.setARN`). General color/set/type-hoser machinery.
    CantBeCast {
        /// Which cards may not be cast (a card filter such as `Card.setARN`).
        valid_card: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Land-play prohibition static: lands (and, in Forge, spells) matching
    /// `valid_card` can't be played/cast while the source is on the
    /// battlefield. Corresponds to `S:Mode$ CantPlayLand | ValidCard$ <filter>`
    /// (City in a Bottle: "can't play lands ... originally printed in ARN").
    CantPlayLand {
        /// Which cards may not be played as lands (e.g. `Card.setARN`).
        valid_card: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Per-creature block restriction (CR 509.1b / 509.4): the source creature
    /// (the *blocker*) can't be declared as a blocker for any attacker matching
    /// `attacker_filter`. Corresponds to the
    /// `S:Mode$ CantBlockBy | ValidAttacker$ <filter> | ValidBlocker$ Creature.Self`
    /// shape, where `ValidBlocker$ Creature.Self` pins the restriction to the
    /// source itself (Ironclaw Orcs: `ValidAttacker$ Creature.powerGE2`,
    /// "can't block creatures with power 2 or greater").
    ///
    /// This is the *blocker-side* form of `CantBlockBy`; the *evasion* form
    /// (`ValidAttacker$ Creature.Self` with no `ValidBlocker$`, meaning "this
    /// attacker can't be blocked") is a different shape and is NOT modelled here.
    CantBlockMatching {
        /// Filter for which ATTACKERS this creature may not block
        /// (e.g. `Creature.powerGE2`). Evaluated against the attacker card.
        attacker_filter: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Allows casting spells as though they had flash
    ///
    /// Corresponds to: `S:Mode$ CastWithFlash | ValidCard$ <filter>`
    CastWithFlash {
        /// Which cards are affected (e.g. Card.nonCreature)
        valid_card: TargetRestriction,
        /// Description for logging
        description: String,
    },

    /// Damage-increase replacement effect (CR 614.1a): when a qualifying red
    /// source controlled by this permanent's controller would deal damage to an
    /// opponent or opponent-controlled permanent, it deals that much plus
    /// `bonus` instead.
    ///
    /// Corresponds to Torbran, Thane of Red Fell's static:
    ///   `R:Event$ DamageDone | ValidSource$ Card.RedSource+YouCtrl
    ///    | ValidTarget$ Player.Opponent,Permanent.OppCtrl | ReplaceWith$ DmgPlus2`
    /// where `DmgPlus2` resolves to `ReplaceCount$DamageAmount/Plus.2`.
    ///
    /// This is deliberately narrow: it only models the "RedSource + YouCtrl →
    /// Opponent/OppCtrl target → +N" shape (the shape Torbran has). Generalising
    /// to arbitrary ValidSource/ValidTarget predicates can be done later when
    /// another card requires it.
    DamageIncrease {
        /// Extra damage to add per damage event (e.g. 2 for Torbran).
        bonus: u32,
        /// Description for logging.
        description: String,
    },

    /// Continuous damage-prevention replacement effect (CR 614.1e / 615.1):
    /// prevent all damage from sources of the chosen color to the enchanted
    /// creature.
    ///
    /// Corresponds to Prismatic Ward's static:
    ///   `R:Event$ DamageDone | Prevent$ True | ValidTarget$ Creature.EnchantedBy
    ///    | ValidSource$ Card.ChosenColor`
    ///
    /// The chosen color is stored on the Aura card at ETB time (via
    /// `K:ETBReplacement:Other:ChooseColor`). At damage resolution, if the
    /// source card's colors include the chosen color and the target creature is
    /// the enchanted creature, the damage is prevented.
    PreventDamageToEnchantedByChosenColor {
        /// Description for logging.
        description: String,
    },

    /// Attack prohibition conditional on the defending player's board state.
    ///
    /// Corresponds to Orgg's static:
    ///   `S:Mode$ CantAttack | ValidCard$ Card.Self
    ///    | UnlessDefender$ !controlsCreature.untapped+powerGE<N>`
    ///
    /// The source creature can't attack if the defending player controls at
    /// least one untapped creature whose power is >= `min_power`. This models
    /// the "can't attack unless defender has NO untapped creature with power ≥ N"
    /// restriction from CR 508.1 (attack legality).
    ///
    /// Evaluated at declare-attackers time (CR 508.1c — "the creature can't attack").
    CantAttackIfDefenderHasUntappedPowerGE {
        /// Minimum power a defending creature must have to lock out the attacker.
        min_power: i32,
        /// Description for logging.
        description: String,
    },

    /// Global attack/block prohibition for a set of creatures (CR 508.1c / 509.1b).
    ///
    /// Corresponds to `S:Mode$ CantAttack | ValidCard$ <filter>`,
    /// `S:Mode$ CantBlock | ValidCard$ <filter>`, or the combined
    /// `S:Mode$ CantAttack,CantBlock | ValidCard$ <filter>` (Light of Day).
    ///
    /// While the source permanent is on the battlefield, ALL battlefield
    /// creatures matching `filter` (regardless of controller) are prohibited
    /// from attacking (if `cant_attack`) and/or blocking (if `cant_block`).
    /// This is distinct from `CantAttackIfDefenderHasUntappedPowerGE` (Orgg),
    /// which restricts one specific creature conditionally.
    CantAttackOrBlockMatching {
        /// Attack prohibition: if true, matching creatures can't attack.
        cant_attack: bool,
        /// Block prohibition: if true, matching creatures can't block.
        cant_block: bool,
        /// Which creatures are restricted.
        filter: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Activated-ability lock: while the source is on the battlefield, no
    /// creature (matching `creature_filter`) may activate an activated ability.
    ///
    /// Corresponds to Cursed Totem:
    ///   `S:Mode$ CantBeActivated | ValidCard$ Creature | ValidSA$ Activated`
    ///
    /// Evaluated at action-generation time: when collecting activated abilities
    /// for a player, any activated ability on a card matching `creature_filter`
    /// is suppressed.
    CantBeActivated {
        /// Creatures whose activated abilities are suppressed.
        creature_filter: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Allows the controller to play additional lands per turn.
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ You | AdjustLandPlays$ N`
    ///
    /// Permanent form (on-battlefield static): Oracle of Mul Daya, Exploration enchantment,
    /// Azusa Lost but Seeking, etc. The extra plays accumulate from all such statics
    /// currently on the battlefield and controlled by the relevant player.
    ///
    /// Applied in `GameState::effective_max_lands()` which sums all `ExtraLandPlay`
    /// statics on battlefield permanents plus `PersistentEffectKind::ExtraLandPlay`
    /// for temporary grants (e.g. the Explore spell).
    ExtraLandPlay {
        /// Number of additional lands per turn (typically 1, 2 for Azusa).
        amount: u8,
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

/// Restriction on the turn-step window in which an activated ability may be
/// activated, derived from `ActivationPhases$ <start>-><end>`.
///
/// "Activate only during combat" (Jade Statue's `BeginCombat->EndCombat`
/// animate, CR 602.5: an ability's activation-timing restriction is part of
/// the ability). The activating step must satisfy `start <= step <= end` in
/// turn order. Because [`Step`](crate::game::phase::Step) is declared in turn
/// order and derives `Ord`, the window is a simple inclusive range check —
/// no per-turn flag, so it is trivially rewind-safe (it reads only the
/// current step, which is reconstructed deterministically on replay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationPhaseWindow {
    /// First step (inclusive) at which the ability may be activated.
    pub start: crate::game::phase::Step,
    /// Last step (inclusive) at which the ability may be activated.
    pub end: crate::game::phase::Step,
}

impl ActivationPhaseWindow {
    /// True if `step` falls within `[start, end]` (inclusive), in turn order.
    pub fn contains(&self, step: crate::game::phase::Step) -> bool {
        self.start <= step && step <= self.end
    }

    /// Parse a `ActivationPhases$ <start>-><end>` value (e.g.
    /// `"BeginCombat->EndCombat"`). A bare single step (`"Upkeep"`) is treated
    /// as a one-step window. Returns `None` if either token is unrecognised.
    ///
    /// Only the single contiguous-range form is modelled here. Forge also has a
    /// *disjoint* multi-range form (`"Upkeep->Main1,Main2->Cleanup"` = "any time
    /// except combat", used by a handful of cards like Aggravated Assault). Those
    /// values contain a comma and/or more than one `->`; we return `None` for
    /// them so the loader leaves the ability unrestricted rather than mis-gating
    /// it to a wrong window. (TODO(mtg-713 B6 follow-up): model disjoint windows
    /// as a small set/vec of ranges if a championship card needs the
    /// except-combat case enforced.)
    pub fn parse(value: &str) -> Option<Self> {
        use crate::game::phase::Step;
        // Reject the disjoint multi-range form (comma list or >1 arrow).
        if value.contains(',') || value.matches("->").count() > 1 {
            return None;
        }
        let (start_tok, end_tok) = match value.split_once("->") {
            Some((s, e)) => (s, e),
            None => (value, value),
        };
        let start = Step::from_script_name(start_tok.trim())?;
        let end = Step::from_script_name(end_tok.trim())?;
        // Guard against an inverted range (would never match any step).
        if start > end {
            return None;
        }
        Some(ActivationPhaseWindow { start, end })
    }
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

    /// All creatures (any controller) whose current power is >= the threshold.
    /// Corresponds to: `ValidCard$ Creature.powerGE<N>` (e.g. Meekstone's
    /// `Creature.powerGE3` doesn't-untap lock). Controller-agnostic: power is
    /// the creature's current (effective) power, evaluated continuously.
    CreaturesWithPowerGE(i32),

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

    /// Instants you control
    /// Corresponds to: `Affected$ Instant.YouCtrl`
    InstantYouControl,

    /// Sorceries you control
    /// Corresponds to: `Affected$ Sorcery.YouCtrl`
    SorceryYouControl,

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

    /// Optional turn-step window restriction from `ActivationPhases$
    /// <start>-><end>` (Jade Statue's combat-only `BeginCombat->EndCombat`
    /// animate). `None` = activatable in any step (the common case). Checked
    /// in `can_activate` alongside the other timing gates. Rewind-safe: reads
    /// only the deterministically-reconstructed current step (CR 602.5).
    #[serde(default)]
    pub activation_phases: Option<crate::core::ActivationPhaseWindow>,

    /// True for `AB$ ManaReflected` abilities (Fellwar Stone: "Add one mana of
    /// any color that a land an opponent controls could produce"). The static
    /// mana-production cache treats such a source as AnyColor (an upper bound),
    /// but at activation time the produced color is constrained to the set of
    /// colors the `Valid$` lands could actually produce — computed from public
    /// battlefield state, so it stays information-independent / deterministic.
    #[serde(default)]
    pub produces_reflected_mana: bool,

    /// The zone in which this ability can be activated (default: Battlefield).
    ///
    /// Parsed from `ActivationZone$` in the card script, e.g.:
    ///   `ActivationZone$ Graveyard` — activatable while the card is in the
    ///   owner's graveyard (CR 702.25b unearth, graveyard-recursion engines).
    ///   `ActivationZone$ Hand`      — activatable from hand (cycling uses a
    ///   dedicated SpellAbility::Cycle variant, but this field handles any
    ///   future hand-activated non-cycle abilities).
    ///
    /// The action enumerator uses this to walk the correct zone when building
    /// the list of available actions (see `push_activatable_abilities`).
    #[serde(default = "default_activation_zone")]
    pub activation_zone: crate::zones::Zone,

    /// Cache for expensive string operations (computed at creation time)
    pub cache: AbilityCache,
}

/// Serde default helper — `Zone::Battlefield` is the overwhelmingly common case.
fn default_activation_zone() -> crate::zones::Zone {
    crate::zones::Zone::Battlefield
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
            activation_phases: None,
            produces_reflected_mana: false,
            activation_zone: crate::zones::Zone::Battlefield,
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
            activation_phases: None,
            produces_reflected_mana: false,
            activation_zone: crate::zones::Zone::Battlefield,
            cache,
        }
    }

    /// Create a new your-turn-only activated ability
    /// Less restrictive than sorcery speed - can be activated any time during my turn
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
            activation_phases: None,
            produces_reflected_mana: false,
            activation_zone: crate::zones::Zone::Battlefield,
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
    fn test_activation_phase_window_parse_jade_statue() {
        use crate::game::phase::Step;
        // Jade Statue: ActivationPhases$ BeginCombat->EndCombat.
        let window = ActivationPhaseWindow::parse("BeginCombat->EndCombat").expect("should parse combat window");
        assert_eq!(window.start, Step::BeginCombat);
        assert_eq!(window.end, Step::EndCombat);

        // Inclusive on both ends; every combat step is inside, non-combat is out.
        assert!(window.contains(Step::BeginCombat));
        assert!(window.contains(Step::DeclareAttackers));
        assert!(window.contains(Step::DeclareBlockers));
        assert!(window.contains(Step::CombatDamage));
        assert!(window.contains(Step::EndCombat));
        assert!(!window.contains(Step::Upkeep));
        assert!(!window.contains(Step::Main1));
        assert!(!window.contains(Step::Main2));
        assert!(!window.contains(Step::End));
    }

    #[test]
    fn test_activation_phase_window_spaced_and_single() {
        use crate::game::phase::Step;
        // Spaced human form (Siren's Call writes "Declare Blockers").
        let w = ActivationPhaseWindow::parse("Upkeep->Declare Blockers").expect("spaced form should parse");
        assert_eq!(w.start, Step::Upkeep);
        assert_eq!(w.end, Step::DeclareBlockers);

        // A bare single step is a one-step window.
        let single = ActivationPhaseWindow::parse("Upkeep").expect("single step should parse");
        assert_eq!(single.start, Step::Upkeep);
        assert_eq!(single.end, Step::Upkeep);
        assert!(single.contains(Step::Upkeep));
        assert!(!single.contains(Step::Draw));

        // An inverted range is rejected (would never match).
        assert!(ActivationPhaseWindow::parse("EndCombat->BeginCombat").is_none());
        // An unrecognised token is rejected.
        assert!(ActivationPhaseWindow::parse("BeginCombat->Nonsense").is_none());

        // Bare "Main" maps to Main1 (Forge's `Upkeep->Main`).
        let upkeep_to_main = ActivationPhaseWindow::parse("Upkeep->Main").expect("Upkeep->Main should parse");
        assert_eq!(upkeep_to_main.start, Step::Upkeep);
        assert_eq!(upkeep_to_main.end, Step::Main1);

        // The disjoint multi-range form is intentionally NOT modelled (returns
        // None -> ability left unrestricted), so we never mis-gate it.
        assert!(ActivationPhaseWindow::parse("Upkeep->Main1,Main2->Cleanup").is_none());
        assert!(ActivationPhaseWindow::parse("Main1,Main2").is_none());
    }

    #[test]
    fn test_dynamic_amount_parse_count_handsize() {
        use std::collections::HashMap;
        // Ivory Tower: SVar:X:Count$ValidHand Card.YouOwn/Minus.4
        let mut svars = HashMap::new();
        svars.insert("X".to_string(), "Count$ValidHand Card.YouOwn/Minus.4".to_string());
        match DynamicAmount::parse("X", &svars) {
            Some(DynamicAmount::Count(CountExpression::CardsInHand { selector, modifier })) => {
                assert_eq!(selector, "Card.YouOwn");
                assert_eq!(modifier, CountModifier::Minus(4));
            }
            other => panic!("expected Count(CardsInHand) for Ivory Tower, got {other:?}"),
        }
    }

    #[test]
    fn test_dynamic_amount_parse_preserves_known_forms() {
        use std::collections::HashMap;
        let mut svars = HashMap::new();
        svars.insert("X".to_string(), "Targeted$CardPower".to_string());
        svars.insert("Y".to_string(), "TriggerCount$DamageAmount".to_string());
        svars.insert("Z".to_string(), "Count$Bogus".to_string()); // unrecognized Count body
        assert_eq!(DynamicAmount::parse("X", &svars), Some(DynamicAmount::TargetPower));
        assert_eq!(DynamicAmount::parse("Y", &svars), Some(DynamicAmount::DamageDealt));
        assert_eq!(DynamicAmount::parse("5", &svars), Some(DynamicAmount::Fixed(5)));
        // An unrecognized Count$ body falls back to None so the caller's fixed
        // path runs rather than silently gaining 0.
        assert_eq!(DynamicAmount::parse("Z", &svars), None);
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
            matches!(&expr, CountExpression::ValidPermanents { filter, .. } if filter == "Artifact.OppCtrl"),
            "Expected ValidPermanents with Artifact.OppCtrl filter, got {:?}",
            expr
        );
    }

    /// B1 fix regression: `Count$Valid Shrine.YouCtrl/Times.2` must parse the
    /// `/Times.2` suffix as `CountModifier::Times(2)` and strip it from the filter.
    ///
    /// Before the fix the whole string was stored as the filter
    /// (`"Shrine.YouCtrl/Times.2"`) with no modifier, causing:
    ///   1. `count_permanents_matching` to warn "Unknown filter type" and
    ///      count ALL permanents (wrong).
    ///   2. The ×2 multiplier to never be applied (gain was 1× instead of 2×).
    #[test]
    fn test_count_expression_parse_valid_permanents_times_modifier() {
        let mut svars = std::collections::HashMap::new();
        svars.insert("X".to_string(), "Count$Valid Shrine.YouCtrl/Times.2".to_string());

        let expr = CountExpression::parse("X", &svars);
        assert!(
            matches!(
                &expr,
                CountExpression::ValidPermanents {
                    filter,
                    modifier: CountModifier::Times(2)
                } if filter == "Shrine.YouCtrl"
            ),
            "Expected ValidPermanents {{ filter: \"Shrine.YouCtrl\", modifier: Times(2) }}, got {:?}",
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
    fn test_count_expression_parse_valid_hand_minus() {
        // Black Vise (mtg-cuf0e): Count$ValidHand Card.ChosenCtrl/Minus.4 must
        // parse to CardsInHand{ selector: "Card.ChosenCtrl", modifier: Minus(4) }.
        // Before the fix this hit no `Count$ValidHand` arm and fell through to
        // Fixed(0), making Black Vise deal 0 damage.
        let mut svars = std::collections::HashMap::new();
        svars.insert("X".to_string(), "Count$ValidHand Card.ChosenCtrl/Minus.4".to_string());

        let expr = CountExpression::parse("X", &svars);
        match &expr {
            CountExpression::CardsInHand { selector, modifier } => {
                assert_eq!(selector, "Card.ChosenCtrl");
                assert_eq!(*modifier, CountModifier::Minus(4));
            }
            CountExpression::Fixed(_)
            | CountExpression::ValidPermanents { .. }
            | CountExpression::CardsDrawnThisTurn
            | CountExpression::XPaid
            | CountExpression::TimesKicked
            | CountExpression::SpellsCastThisTurn
            | CountExpression::ValidGraveyard { .. }
            | CountExpression::Kicked { .. }
            | CountExpression::Bargain { .. }
            | CountExpression::TargetedCardPower
            | CountExpression::TriggeredCardPower
            | CountExpression::Compare { .. } => panic!("Expected CardsInHand, got {:?}", expr),
        }

        // No modifier and a Plus modifier round-trip too.
        svars.insert("Y".to_string(), "Count$ValidHand Card.YouCtrl".to_string());
        assert!(matches!(
            CountExpression::parse("Y", &svars),
            CountExpression::CardsInHand {
                modifier: CountModifier::None,
                ..
            }
        ));
        svars.insert("Z".to_string(), "Count$ValidHand Card.YouCtrl/Plus.2".to_string());
        assert!(matches!(
            CountExpression::parse("Z", &svars),
            CountExpression::CardsInHand {
                modifier: CountModifier::Plus(2),
                ..
            }
        ));

        // CountModifier arithmetic.
        assert_eq!(CountModifier::Minus(4).apply(6), 2);
        assert_eq!(CountModifier::Minus(4).apply(3), -1); // unclamped; caller clamps
        assert_eq!(CountModifier::None.apply(5), 5);
        assert_eq!(CountModifier::Plus(2).apply(5), 7);

        // Combustion Technique: Count$ValidGraveyard Lesson.YouOwn/Plus.2
        // should parse to ValidGraveyard { filter: "Lesson.YouOwn", modifier: Plus(2) }.
        svars.insert(
            "CT".to_string(),
            "Count$ValidGraveyard Lesson.YouOwn/Plus.2".to_string(),
        );
        match CountExpression::parse("CT", &svars) {
            CountExpression::ValidGraveyard { filter, modifier } => {
                assert_eq!(filter, "Lesson.YouOwn");
                assert_eq!(modifier, CountModifier::Plus(2));
            }
            CountExpression::Fixed(_)
            | CountExpression::ValidPermanents { .. }
            | CountExpression::CardsDrawnThisTurn
            | CountExpression::CardsInHand { .. }
            | CountExpression::XPaid
            | CountExpression::TimesKicked
            | CountExpression::SpellsCastThisTurn
            | CountExpression::Compare { .. }
            | CountExpression::TargetedCardPower
            | CountExpression::TriggeredCardPower
            | CountExpression::Kicked { .. }
            | CountExpression::Bargain { .. } => {
                panic!("Expected ValidGraveyard, got {:?}", &svars["CT"])
            }
        }
        // No-modifier variant.
        svars.insert("CTB".to_string(), "Count$ValidGraveyard Spell.YouOwn".to_string());
        assert!(matches!(
            CountExpression::parse("CTB", &svars),
            CountExpression::ValidGraveyard {
                modifier: CountModifier::None,
                ..
            }
        ));
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
                    CountExpression::ValidPermanents { filter, .. } => {
                        assert_eq!(filter, "Creature.YouCtrl+powerGE4");
                    }
                    CountExpression::Fixed(_)
                    | CountExpression::CardsDrawnThisTurn
                    | CountExpression::CardsInHand { .. }
                    | CountExpression::XPaid
                    | CountExpression::TimesKicked
                    | CountExpression::SpellsCastThisTurn
                    | CountExpression::ValidGraveyard { .. }
                    | CountExpression::Kicked { .. }
                    | CountExpression::Bargain { .. }
                    | CountExpression::TargetedCardPower
                    | CountExpression::TriggeredCardPower
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
            | CountExpression::CardsInHand { .. }
            | CountExpression::XPaid
            | CountExpression::TimesKicked
            | CountExpression::SpellsCastThisTurn
            | CountExpression::ValidGraveyard { .. }
            | CountExpression::Kicked { .. }
            | CountExpression::Bargain { .. }
            | CountExpression::TargetedCardPower
            | CountExpression::TriggeredCardPower => panic!("Expected Compare, got {:?}", expr),
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
