//! Card effects and ability system

use crate::core::{CardId, CardType, Color, Keyword, KeywordArgs, PlayerId};
use crate::zones::Zone;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

pub mod activated_ability;
pub mod static_abilities;
pub mod triggers;

pub use activated_ability::{AbilityCache, ActivatedAbility};
pub use static_abilities::{
    ActivationCondition, ActivationPhaseWindow, AffectedSelector, AltCostCondition, CdaPtSource, CompareOp,
    CostReductionAmount, CostReductionCondition, CostReductionTarget, RaisedCost, RaisedCostAmount, StaticAbility,
    StaticCondition, UnlessCost, UnlessCostType,
};
pub use triggers::{CombatDamageTarget, ModalMode, Trigger, TriggerEvent};

/// Who is subject to a `CantBeCast` prohibition.
///
/// Parsed from the `Caster$` field in `S:Mode$ CantBeCast | Caster$ <value>` lines.
/// - `Any`         — no `Caster$` key present; applies to all players.
/// - `You`         — only the source card's *controller* is restricted.
/// - `YouNonActive`— only the controller while they are the **non-active** player.
/// - `Opponent`    — only opponents of the source's controller are restricted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CasterRestriction {
    /// Applies to all players (default when `Caster$` is absent).
    #[default]
    Any,
    /// Restricts only the source's controller.
    You,
    /// Restricts the source's controller only while they are non-active.
    YouNonActive,
    /// Restricts opponents of the source's controller only.
    Opponent,
}

/// Action to schedule on the target at the beginning of the next end step.
///
/// Used by Animate effects (`AtEOT$ Sacrifice` / `AtEOT$ Exile`) to implement
/// "that creature gains haste; sacrifice/exile it at the beginning of the next
/// end step" — e.g. Sneak Attack and Goryo's Vengeance (CR 603.7a delayed
/// triggered ability fires at the beginning of the next end step).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtEotAction {
    /// Sacrifice the target at the beginning of the next end step.
    /// Used by Sneak Attack: `AtEOT$ Sacrifice`.
    Sacrifice,
    /// Exile the target at the beginning of the next end step.
    /// Used by Goryo's Vengeance: `AtEOT$ Exile`.
    Exile,
}

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
    /// If true, target must NOT be in the "remembered" set — `!IsRemembered` qualifier.
    ///
    /// Used by Tragic Arrogance's `SacAllOthers`:
    ///   `ValidCards$ Permanent.nonLand+!IsRemembered`
    /// At runtime, `execute_sacrifice_all` calls
    /// `matches_excluding_remembered(&game.remembered_cards)` which returns false
    /// for any card whose id appears in the remembered list.
    #[serde(default)]
    pub requires_not_remembered: bool,
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
    /// Maximum mana value (CMC) restriction (`cmcLE<N>` qualifier).
    ///
    /// Corresponds to `ValidCards$ Creature.cmcLE3` (Consume the Meek),
    /// `ValidTgts$ Instant.YouCtrl+cmcLE3` (Past in Flames), etc.
    /// `None` means no maximum CMC restriction.
    #[serde(default)]
    pub max_cmc: Option<u8>,
    /// If true, target creature must have the Defender keyword (CR 702.6).
    ///
    /// Corresponds to the `withDefender` qualifier in `ValidTgts$` /
    /// `ValidCards$` (e.g. `Creature.withDefender+YouCtrl` for Overgrown
    /// Battlement's mana ability, `Creature.withDefender` for Clear a Path).
    /// Checked via `card.has_keyword(Keyword::Defender)`.
    #[serde(default)]
    pub requires_defender: bool,
    /// If true, the card must share its name with the current `GameState::remembered_name`
    /// (Cranial Extraction: `ChangeType$ Card.NamedCard`). Plain `matches()` always
    /// returns false for named-card filters — callers must use `matches_with_name`.
    ///
    /// Parsed from the `NamedCard` qualifier in `ValidCards$` / `ChangeType$`.
    #[serde(default)]
    pub requires_named_card: bool,
    /// Exact mana value (CMC) restriction (`cmcEQ<N>` qualifier, static form).
    ///
    /// Corresponds to `ValidCards$ Permanent.nonLand+cmcEQ2` (literal N) when
    /// the CMC is known at load time. For the dynamic SVar form (`cmcEQX`),
    /// `cmc_eq_svar` is set instead and the caller resolves it at runtime.
    /// `None` means no exact-CMC restriction.
    #[serde(default)]
    pub exact_cmc: Option<u8>,
    /// If true, the exact-CMC filter (`cmcEQX`) references an SVar whose value
    /// is not known until resolution time (Ratchet Bomb: CMC must equal the
    /// number of charge counters). The caller is responsible for resolving the
    /// SVar and writing the result into `exact_cmc` before matching.
    #[serde(default)]
    pub cmc_eq_svar: bool,
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
            requires_not_remembered: false,
            requires_nonartifact: false,
            required_color: None,
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            max_cmc: None,
            requires_defender: false,
            requires_named_card: false,
            exact_cmc: None,
            cmc_eq_svar: false,
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
            requires_not_remembered: false,
            requires_nonartifact: false,
            required_color: None,
            required_set: None,
            requires_other: false,
            required_subtype: None,
            power_le_source: false,
            requires_noncreature: false,
            min_cmc: None,
            max_cmc: None,
            requires_defender: false,
            requires_named_card: false,
            exact_cmc: None,
            cmc_eq_svar: false,
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

        // Named-card filters require the runtime name from `GameState::remembered_name`.
        // Plain `matches()` has no access to GameState, so it always returns false here;
        // callers that know the remembered name must use `matches_with_name` instead.
        if self.requires_named_card {
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

        // Check maximum CMC restriction (Consume the Meek: Creature.cmcLE3)
        if let Some(max) = self.max_cmc {
            if card.mana_cost.cmc() > max {
                return false;
            }
        }

        // Check exact CMC restriction (Ratchet Bomb: Permanent.nonLand+cmcEQ<N>).
        // `exact_cmc` is populated by the caller from a static `cmcEQ<N>` qualifier
        // or by resolving the SVar X (charge-counter count) at activation time.
        if let Some(eq) = self.exact_cmc {
            if card.mana_cost.cmc() != eq {
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
            && !self.requires_not_remembered
            && !self.requires_nonartifact
            && self.required_color.is_none()
            && self.required_set.is_none()
            && !self.requires_other
            && !self.requires_named_card
            && self.min_cmc.is_none()
            && self.max_cmc.is_none()
            && self.exact_cmc.is_none()
            && !self.cmc_eq_svar
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

    /// Like [`TargetRestriction::matches`], but also enforces the
    /// `requires_not_remembered` qualifier at runtime by checking the provided
    /// remembered-card slice.
    ///
    /// Used by `execute_sacrifice_all` for Tragic Arrogance's
    /// `ValidCards$ Permanent.nonLand+!IsRemembered`: a permanent whose id
    /// appears in `remembered` is the one the caster chose to keep, so it must
    /// NOT be sacrificed.
    pub fn matches_with_remembered(&self, card: &crate::core::Card, remembered: &[crate::core::CardId]) -> bool {
        // If this filter requires cards that are NOT in the remembered list,
        // reject any card whose id IS in remembered.
        if self.requires_not_remembered && remembered.contains(&card.id) {
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
    /// - "Permanent.nonLand+cmcEQ2" -> exact_cmc=2 (static literal form)
    /// - "Permanent.nonLand+cmcEQX" -> cmc_eq_svar=true (dynamic SVar form; caller resolves X)
    pub fn parse(valid_tgts: &str) -> Self {
        let mut types = SmallVec::new();
        let mut requires_no_counters = false;
        let mut requires_nontoken = false;
        let mut requires_remembered = false;
        let mut requires_not_remembered = false;
        let mut requires_nonartifact = false;
        let mut requires_noncreature = false;
        let mut requires_defender = false;
        let mut requires_named_card = false;
        let mut min_cmc = None;
        let mut max_cmc = None;
        let mut exact_cmc: Option<u8> = None;
        let mut cmc_eq_svar = false;
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
                        "!IsRemembered" => requires_not_remembered = true,
                        "YouCtrl" => controller = ControllerRestriction::YouCtrl,
                        "OppCtrl" => controller = ControllerRestriction::OppCtrl,
                        "ActivePlayerCtrl" => controller = ControllerRestriction::ActivePlayerCtrl,
                        // Forge DSL: "ControlledBy TriggeredDefendingPlayer" — target must be
                        // controlled by the defending player in the current combat.  In a 2-player
                        // game the defending player is always the opponent of the attacker, so we
                        // map this to OppCtrl for targeting purposes.
                        "ControlledBy TriggeredDefendingPlayer" => controller = ControllerRestriction::OppCtrl,
                        "nonArtifact" => requires_nonartifact = true,
                        "nonCreature" => requires_noncreature = true,
                        "Other" => requires_other = true,
                        // `withDefender` — target must have the Defender keyword
                        // (CR 702.6). Used by Overgrown Battlement's mana
                        // ability, Clear a Path, Axebane Guardian, etc.
                        "withDefender" => requires_defender = true,
                        // `NamedCard` — card must share its name with the current
                        // `GameState::remembered_name` (Cranial Extraction:
                        // `ChangeType$ Card.NamedCard`). Plain `matches()` always
                        // returns false for named-card filters; callers use
                        // `matches_with_name` instead.
                        "NamedCard" => requires_named_card = true,
                        // `nonLand` — card must not be a land type. Used in the
                        // ValidCards$ on NameCard to constrain what name can be
                        // chosen; we don't enforce it in the filter predicate (any
                        // nonland card name the controller picks is AI-chosen from
                        // public info anyway).
                        "nonLand" => {} // silently accepted; no-op in the filter
                        m if m.starts_with("cmcGE") => {
                            // Parse cmcGE4 -> min_cmc = 4 (Disdainful Stroke)
                            if let Ok(n) = m.trim_start_matches("cmcGE").parse::<u8>() {
                                min_cmc = Some(n);
                            }
                        }
                        m if m.starts_with("cmcLE") => {
                            // Parse cmcLE3 -> max_cmc = 3 (Consume the Meek, Past in Flames)
                            if let Ok(n) = m.trim_start_matches("cmcLE").parse::<u8>() {
                                max_cmc = Some(n);
                            }
                        }
                        // `cmcEQX` — dynamic exact-CMC filter: CMC must equal SVar X at
                        // resolution time (Ratchet Bomb: charge-counter count). Mark the
                        // flag; the caller resolves X and populates `exact_cmc` before use.
                        // MUST precede the numeric `cmcEQ<N>` arm because "cmcEQX" also
                        // starts with "cmcEQ" (and "X" is not a number).
                        "cmcEQX" => cmc_eq_svar = true,
                        m if m.starts_with("cmcEQ") => {
                            // Parse cmcEQ2 -> exact_cmc = 2 (static literal form)
                            if let Ok(n) = m.trim_start_matches("cmcEQ").parse::<u8>() {
                                exact_cmc = Some(n);
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
            requires_not_remembered,
            requires_nonartifact,
            required_color,
            required_set,
            requires_other,
            required_subtype,
            power_le_source,
            requires_noncreature,
            min_cmc,
            max_cmc,
            requires_defender,
            requires_named_card,
            exact_cmc,
            cmc_eq_svar,
        }
    }

    /// Like [`TargetRestriction::matches`], but also filters by `name`:
    /// when `requires_named_card` is set, the card's name must equal `name`
    /// (case-sensitive, Cranial Extraction / Memoricide style). For all other
    /// restrictions the check delegates to [`TargetRestriction::matches`].
    pub fn matches_with_name(&self, card: &crate::core::Card, name: &str) -> bool {
        if self.requires_named_card && card.name.as_str() != name {
            return false;
        }
        // All other restrictions (type, controller, CMC, etc.) still apply —
        // but for the `ChangeType$ Card.NamedCard` pattern the base type is "Card"
        // (no type filter), so `matches` on a pure named-card restriction returns
        // true for any card once the name check passes.
        //
        // We must NOT call `self.matches(card)` directly here because that
        // function short-circuits to `false` when `requires_named_card` is set
        // (it has no access to the runtime name). Instead, clone with the flag
        // cleared so the delegate checks all OTHER restrictions normally.
        if self.requires_named_card {
            let mut without_name_guard = self.clone();
            without_name_guard.requires_named_card = false;
            without_name_guard.matches(card)
        } else {
            self.matches(card)
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
        /// Source card for dynamic SVar resolution (Ratchet Bomb: `cmcEQX`
        /// where X = charge-counter count on the Bomb itself). Set by
        /// `resolve_self_target` to the resolving card's `CardId`; `None` if
        /// the restriction needs no SVar resolution.
        #[serde(default)]
        cmc_eq_source: Option<CardId>,
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

    /// Untap exactly one (the first tapped) permanent matching a filter.
    /// Used for Hokori, Dust Drinker's upkeep trigger: "that player untaps a
    /// land they control" — one land, player's choice resolved as first tapped.
    UntapOne { restriction: TargetRestriction },

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

    /// Tap N untapped permanents matching a filter controlled by a player.
    ///
    /// Corresponds to Tangle Wire's forced-tap trigger:
    ///   `DB$ ChooseCard | Defined$ TriggeredPlayer | Choices$ <filter> | Amount$ X | SubAbility$ DBTap`
    ///   `DB$ Tap | Defined$ ChosenCard`
    ///
    /// The converter collapses the ChooseCard→Tap→Cleanup sub-ability chain into
    /// a single effect. The AI heuristic taps the least-valuable permanents first
    /// (same as the discard heuristic: prefer to tap things that are already
    /// somewhat restricted, e.g., tap creatures before lands).
    TapPermanentsMatchingFilter {
        /// The player who must tap permanents (placeholder: resolved to active player at trigger time).
        player: PlayerId,
        /// Combined filter string from `Choices$` (e.g. `"Artifact,Creature,Land"`).
        /// Each comma-separated segment is a type that qualifies.
        choices_filter: String,
        /// Number of permanents to tap (`Amount$ X` = fade counter count at fire time;
        /// stored as 0 at parse time and resolved to a concrete count by the
        /// trigger executor before calling execute_effect).
        count: u8,
    },

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
        /// Simple keywords to grant (e.g., Double Strike from `KW$ Double Strike`).
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
        /// Complex (parameterized) keywords to grant, e.g., `Landwalk:Forest`
        /// from `KW$ Landwalk:Forest`. Stored separately because they need both
        /// the keyword bit and the land-type (or other) parameter to correctly
        /// enforce blocking restrictions and to be removed on cleanup.
        /// Serde-defaulted so existing serialized data without this field
        /// deserializes correctly (empty = no complex keywords).
        #[serde(default)]
        keyword_args_granted: smallvec::SmallVec<[KeywordArgs; 2]>,
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
        /// Simple keywords to grant (e.g., Flying, Trample)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
        /// Complex (parameterized) keywords to grant (e.g., Landwalk:Island)
        #[serde(default)]
        keyword_args_granted: smallvec::SmallVec<[KeywordArgs; 2]>,
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
        /// Simple keywords to grant (optional)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
        /// Complex (parameterized) keywords to grant (e.g., Landwalk:Forest)
        #[serde(default)]
        keyword_args_granted: smallvec::SmallVec<[KeywordArgs; 2]>,
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

    /// Grant a player the ability to cast spells matching `valid_card` as though
    /// they had flash, until end of turn.
    ///
    /// Corresponds to:
    ///   `AB$ Effect | StaticAbilities$ STPlay | Duration$ UntilYourNextTurn`
    ///   where `SVar:STPlay:Mode$ CastWithFlash | ValidCard$ <filter>`.
    ///
    /// Creates a `PersistentEffectKind::GrantCastWithFlash` for the spell's
    /// controller. The priority loop checks this alongside `StaticAbility::CastWithFlash`
    /// on battlefield permanents.
    GrantCastWithFlash {
        /// Player who may cast the affected spells with flash. Placeholder until resolved.
        player: PlayerId,
        /// Filter for which cards may be cast with flash (e.g. `Sorcery`).
        valid_card: TargetRestriction,
    },

    /// Return a permanent to its owner's hand (bounce).
    ///
    /// Corresponds to: `AB$ ChangeZone | Origin$ Battlefield | Destination$ Hand | ValidTgts$ <filter>`
    /// Examples: Teferi −3 (artifact/creature/enchantment), Petty Theft (nonland opponent-controlled).
    ///
    /// `target` is `CardId::placeholder()` until targeting resolves. `restriction` encodes
    /// the `ValidTgts$` filter so target-validation and AI target-selection can apply it.
    ReturnPermanentToHand {
        /// The permanent to return. Placeholder until targeting resolves.
        target: CardId,
        /// Filter from the card's `ValidTgts$` (e.g. Artifact,Creature,Enchantment).
        restriction: TargetRestriction,
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
        /// Optional player scope for per-zone searches (Cranial Extraction:
        /// "Search TARGET PLAYER's graveyard, hand, and library"). When `Some`,
        /// only that player's per-player zones (Hand/Library/Graveyard/Exile)
        /// are searched; when `None` all players' zones are searched (default,
        /// Timetwister / Wheel / Tormod's Crypt behaviour).
        ///
        /// `PlayerId::placeholder()` encodes "the opponent of the spell's
        /// controller" and is resolved to the actual opponent at execution time.
        #[serde(default)]
        target_player: Option<PlayerId>,
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
        /// Whether to push the moved card onto `remembered_cards` so chained
        /// `Defined$ Remembered` sub-abilities (e.g. Goryo's Vengeance DBPump
        /// haste grant) can find it. Corresponds to `RememberChanged$ True`.
        #[serde(default)]
        remember_changed: bool,
    },

    /// Put a creature card of matching type from the controller's hand onto the
    /// battlefield directly (without paying mana cost).
    ///
    /// Used by Sneak Attack: `A:AB$ ChangeZone | Origin$ Hand | Destination$
    /// Battlefield | ChangeType$ Creature.YouCtrl | RememberChanged$ True`.
    /// The moved card is pushed onto `remembered_cards` when `remember_changed`
    /// so the chained Animate sub-ability (`Defined$ Remembered`) can target it.
    ///
    /// CR 701.3: "put onto the battlefield" means the controller of the effect
    /// controls the permanent; ownership stays with the card's owner.
    PutCreatureFromHandOnBattlefield {
        /// Controller of the ability (who puts the creature onto the battlefield).
        /// `PlayerId::placeholder()` until resolved at activation time.
        player: PlayerId,
        /// Card type / filter string (e.g. `"Creature"` from `ChangeType$
        /// Creature.YouCtrl`). Restricts which cards in hand are eligible.
        type_filter: String,
        /// Whether to push the moved card onto `remembered_cards`.
        /// `RememberChanged$ True` → true; default false.
        remember_changed: bool,
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

    /// Create a token whose power and toughness equal the `stored_int` of a source card
    /// (Phyrexian Processor: `{4},{T}: Create an X/X black Phyrexian Minion token where
    /// X is the life paid as Phyrexian Processor entered`).
    ///
    /// At resolution the engine reads `source_card.stored_int` (the life paid on ETB),
    /// uses it as both power and toughness, then creates the token from `token_script`.
    /// If `stored_int` is `None` (Processor somehow entered without the ETB firing),
    /// the token defaults to 0/0 — matching the pre-fix behavior so no new crash paths.
    ///
    /// Corresponds to:
    ///   `A:AB$ Token | Cost$ 4 T | TokenScript$ b_x_x_phyrexian_minion |
    ///    TokenPower$ LifePaidOnETB | TokenToughness$ LifePaidOnETB`
    CreateTokenWithStoredPt {
        /// The card whose `stored_int` supplies P/T (the Processor itself).
        source_card: crate::core::CardId,
        /// Controller of the new token (resolved from the placeholder at activation time).
        controller: crate::core::PlayerId,
        /// Token script name (e.g. `"b_x_x_phyrexian_minion"`).
        token_script: String,
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
        /// Subtypes to add to the token copy (e.g., `[Subtype("Hero")]`, `[Subtype("Coward")]`).
        ///
        /// These are creature/permanent *subtypes* (CR 205.3), not card-type
        /// supertypes. Parsed from the `AddTypes$` parameter of `DB$ CopyPermanent`
        /// (despite the Forge parameter name, the values are subtypes). Using
        /// `Subtype` rather than `String` makes the domain explicit and eliminates
        /// free-form string comparisons at execution time.
        add_subtypes: smallvec::SmallVec<[crate::core::Subtype; 2]>,
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
        /// Card type to balance. `None` means any permanent; `Some(t)` restricts to
        /// cards of that type (e.g. `Some(CardType::Land)` for Balance's default,
        /// `Some(CardType::Creature)` for the creature-equalizing sub-chain).
        card_type: Option<CardType>,
        /// Zone in which to balance. Only `Zone::Battlefield` (sacrifice permanents)
        /// and `Zone::Hand` (discard) are used in practice.
        zone: Zone,
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
        /// Simple keywords to grant (e.g., Trample from Keywords$ parameter)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
        /// Complex (parameterized) keywords to grant (e.g., Landwalk:Forest)
        #[serde(default)]
        keyword_args_granted: smallvec::SmallVec<[KeywordArgs; 2]>,
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
        /// Optional end-of-turn action on the target (Sneak Attack: sacrifice;
        /// Goryo's Vengeance: exile). When set, a phase-based delayed trigger
        /// fires at the beginning of the next end step.
        ///
        /// Parsed from `AtEOT$ Sacrifice` / `AtEOT$ Exile` in Animate scripts.
        /// The delayed trigger targets the same card as the Animate effect.
        #[serde(default)]
        at_eot: Option<AtEotAction>,
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

    /// For each permanent type in `types`, choose one permanent of that type
    /// controlled by the player in `remembered_players[0]` and push the chosen
    /// permanents onto `game.remembered_cards`.
    ///
    /// Corresponds to Tragic Arrogance's `YouChoose` SVar:
    ///   `DB$ ChooseCard | Defined$ You | ChooseEach$ Artifact & Creature & Enchantment & Planeswalker
    ///    | ControlledByPlayer$ Remembered | RememberChosen$ True | Mandatory$ True`
    ///
    /// The CASTER (Defined$ You) makes each choice from among permanents
    /// controlled by the current-loop player (`ControlledByPlayer$ Remembered`,
    /// where Remembered = the player stored in `remembered_players[0]` by the
    /// enclosing `RepeatEach | RepeatPlayers$ Player` loop).
    ///
    /// AI heuristic:
    /// - When choosing for ITSELF (current loop player == caster): keep the
    ///   highest-mana-value permanent of each type (save the best).
    /// - When choosing for an OPPONENT: keep the lowest-mana-value permanent
    ///   of each type (sacrifice the opponent's best).
    /// - If a player controls no permanent of a given type, that type is
    ///   silently skipped (nothing is remembered for it).
    ChooseAndRememberOneOfEach {
        /// The permanent types to iterate over (one choice per type).
        /// Parsed from `ChooseEach$ Artifact & Creature & Enchantment & Planeswalker`.
        types: Vec<TargetType>,
    },

    /// Choose a card name and store it in `GameState::remembered_name`.
    ///
    /// Corresponds to: `SP$ NameCard | Defined$ You | ValidCards$ Card.nonLand | ...`
    /// (Cranial Extraction, Memoricide, Cranial Extraction — any "name a card" spell).
    ///
    /// The chosen name is stored in `GameState::remembered_name` and read by
    /// subsequent sub-abilities in the same resolution chain (e.g. `ChangeType$
    /// Card.NamedCard` in `ChangeZoneAll`). This is the SPELL-ABILITY form of
    /// NameCard (resolution-time choice); Pithing Needle's ETB form stores on
    /// `Card::chosen_name` instead (different scope: per-card persistent vs.
    /// per-resolution transient).
    ///
    /// AI heuristic: name the card most prevalent in the opponent's visible zones
    /// (hand count hidden, so we target based on graveyard + battlefield clues).
    /// In the AI implementation, we simply pick the most common card name in the
    /// opponent's graveyard; if the graveyard is empty, we pick the most common
    /// card name on the battlefield. This is information-independent (graveyard
    /// and battlefield are public).
    ChooseName {
        /// The player making the choice (placeholder resolved to card_owner at cast)
        player: PlayerId,
    },

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
            | Effect::CreateTokenWithStoredPt { .. }
            | Effect::Dig { .. }
            | Effect::SearchLibrary { .. }
            | Effect::Firebend { .. }
            | Effect::CopySpellAbility { .. }
            | Effect::ImmediateTrigger { .. }
            | Effect::ClearRemembered
            | Effect::AddTurn { .. }
            | Effect::AddPhase { .. }
            | Effect::ChooseName { .. }
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
            | Effect::PutCreatureFromHandOnBattlefield { .. }
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
            | Effect::ExtraLandPlay { .. }
            // ChooseAndRememberOneOfEach reads from remembered_players (set by RepeatEach);
            // no cast-time target selection.
            | Effect::ChooseAndRememberOneOfEach { .. }
            // GrantCastWithFlash grants flash-casting permission to the spell's controller;
            // no cast-time target is selected.
            | Effect::GrantCastWithFlash { .. } => EffectTargetCategory::NoTargetNeeded,

            // Effects using filters (affect multiple permanents)
            Effect::PumpAllCreatures { .. }
            | Effect::AnimateAll { .. }
            | Effect::DestroyAll { .. }
            | Effect::SacrificeAll { .. }
            | Effect::DamageAll { .. }
            | Effect::TapAll { .. }
            | Effect::UntapAll { .. }
            | Effect::UntapOne { .. }
            | Effect::PutCounterAll { .. }
            | Effect::ChangeZoneAll { .. }
            | Effect::TapPermanentsMatchingFilter { .. } => EffectTargetCategory::UsesFilter,

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
            | Effect::ReturnPermanentToHand { .. }
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
