//! Target and filter types for the effect system.
//!
//! This module houses the core targeting primitives — `TargetRef`,
//! `ControllerRestriction`, `TargetType`, `DigFilter`, and `TargetRestriction`
//! — that determine what a spell or ability can legally target, and how a mass
//! effect filter is matched against the game's cards. All types here are
//! pure data: no game-loop logic, no side effects.
//!
//! They are re-exported from the parent `effects` module and then from
//! `crate::core`, so callers import them from the familiar
//! `use crate::core::{TargetRef, TargetRestriction, …}` path.

use crate::core::{CardId, PlayerId};
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
