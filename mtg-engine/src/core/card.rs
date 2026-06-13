//! Card types and definitions

use crate::core::{
    CardId, CardName, Color, CounterType, Effect, GameEntity, Keyword, KeywordArgs, KeywordSet, ManaCost,
    ManaProduction, PlayerId, Subtype, Trigger,
};
use crate::loader::CardDefinition;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Card types in MTG
/// Copy-eligible since it's a simple enum with no data fields
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardType {
    Creature,
    Instant,
    Sorcery,
    Enchantment,
    Artifact,
    Land,
    Planeswalker,
}

impl CardType {
    /// Parse a card-type word from a card script (e.g. the `AddTypes$` parameter
    /// of `DB$ Clone`, or a token of the `Types:` line).
    ///
    /// Returns `None` for supertypes (Legendary, Snow) and subtypes, which are
    /// handled by separate parsers — this keeps the strongly-typed `CardType`
    /// enum honest about what it represents.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Creature" => Some(Self::Creature),
            "Instant" => Some(Self::Instant),
            "Sorcery" => Some(Self::Sorcery),
            "Enchantment" => Some(Self::Enchantment),
            "Artifact" => Some(Self::Artifact),
            "Land" => Some(Self::Land),
            "Planeswalker" => Some(Self::Planeswalker),
            _ => None,
        }
    }

    /// Canonical card-script spelling of this card type.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Creature => "Creature",
            Self::Instant => "Instant",
            Self::Sorcery => "Sorcery",
            Self::Enchantment => "Enchantment",
            Self::Artifact => "Artifact",
            Self::Land => "Land",
            Self::Planeswalker => "Planeswalker",
        }
    }
}

/// Cache for precomputed properties on Card
/// Pre-computed at card load time to avoid repeated computations during gameplay
///
/// DESIGN: Mana production is derived from parsed ActivatedAbility data (via `Produced$`
/// parameter in card files), NOT from oracle text. This follows the Java Forge approach
/// where `AbilityManaPart.origProduced` stores the structured Produced$ value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardCache {
    // ==== Precomputed type checks (eliminate Vec::contains overhead) ====
    /// Precomputed: Is this card a land?
    /// Derived from types.contains(&CardType::Land) for O(1) checks
    pub is_land: bool,

    /// Precomputed: Is this card a creature?
    /// Derived from types.contains(&CardType::Creature) for O(1) checks
    pub is_creature: bool,

    /// Precomputed: Is this card an artifact?
    /// Derived from types.contains(&CardType::Artifact) for O(1) checks
    pub is_artifact: bool,

    /// Precomputed: Is this card an instant?
    /// Derived from types.contains(&CardType::Instant) for O(1) checks
    pub is_instant: bool,

    /// Precomputed: Is this card a sorcery?
    /// Derived from types.contains(&CardType::Sorcery) for O(1) checks
    pub is_sorcery: bool,

    /// Precomputed: Is this card an enchantment?
    /// Derived from types.contains(&CardType::Enchantment) for O(1) checks
    pub is_enchantment: bool,

    /// Precomputed: Is this card an Aura (enchantment with Aura subtype)?
    /// Eliminates subtype iteration and eq_ignore_ascii_case in hot path
    pub is_aura: bool,

    /// Precomputed: Is this card an Equipment (artifact with Equipment subtype)?
    /// Eliminates subtype iteration and eq_ignore_ascii_case in hot path
    pub is_equipment: bool,

    // ==== Precomputed function results (eliminate runtime computation) ====
    /// Precomputed mana production (upper bound, OR semantics)
    /// - Default (empty Choice) = no mana production
    /// - Fixed(color) = produces exactly one color
    /// - Choice(colors) = can produce ONE of several colors (OR logic)
    /// - AnyColor = can produce any color
    /// - Colorless = produces colorless mana
    ///
    /// This is an UPPER BOUND - it represents what the card CAN produce, not accounting
    /// for tap status, summoning sickness, or activation costs (which are always None/free for cached values).
    ///
    /// This value is derived from parsed ActivatedAbility effects (Effect::AddMana),
    /// NOT from grepping oracle text. See `derive_mana_production_from_abilities()`.
    pub mana_production: ManaProduction,

    /// Precomputed: Is this card a mana source? (produces any mana)
    /// Derived from mana_production.produces_mana() for O(1) checks in event handlers
    pub is_mana_source: bool,

    /// Precomputed: Does this spell require a target when cast?
    /// (from spell_requires_stack_target function in game_loop.rs)
    pub requires_stack_target: bool,

    /// Precomputed: Spell targeting restrictions (from oracle text analysis)
    /// These are used by get_valid_targets_for_spell() to filter valid targets
    /// Example: "Destroy target land" sets spell_targets_land = true
    pub spell_targets_land: bool,

    /// Spell targets creature(s) (e.g., "Destroy target creature")
    pub spell_targets_creature: bool,

    /// Spell targets planeswalker(s) (e.g., "deals damage to target creature or planeswalker")
    #[serde(default)]
    pub spell_targets_planeswalker: bool,

    /// Spell targets player(s) (e.g., "Target player draws three cards")
    pub spell_targets_player: bool,

    /// Spell can target "any target" (creature or player)
    /// Example: "Lightning Bolt deals 3 damage to any target"
    pub spell_targets_any: bool,

    /// Precomputed: this spell costs {1} more for each target beyond the first
    /// (CR 601.2f). Derived from a self-referential
    /// `S:Mode$ RaiseCost | ValidCard$ Card.Self | Relative$ True` line
    /// (Fireball). When set, the cast path adds `(num_targets - 1)` generic mana
    /// to the cost AFTER targets are chosen. The count is public state, so the
    /// added cost is network-deterministic.
    pub spell_relative_target_cost: bool,

    /// Precomputed: Static value of this land for AI evaluation
    /// (from evaluate_land function in game_state_evaluator.rs)
    /// Only meaningful for lands, 0 for non-lands
    pub land_evaluation_value: i32,

    // ==== Land subtype flags (eliminate eq_ignore_ascii_case in hot paths) ====
    /// Precomputed: Has "Plains" subtype (for mana production)
    pub has_plains_subtype: bool,

    /// Precomputed: Has "Island" subtype (for mana production)
    pub has_island_subtype: bool,

    /// Precomputed: Has "Swamp" subtype (for mana production)
    pub has_swamp_subtype: bool,

    /// Precomputed: Has "Mountain" subtype (for mana production)
    pub has_mountain_subtype: bool,

    /// Precomputed: Has "Forest" subtype (for mana production)
    pub has_forest_subtype: bool,

    /// Precomputed: Does this card enter the battlefield tapped?
    /// Derived from R: lines with "ReplaceWith$ ETBTapped" replacement effect
    pub enters_tapped: bool,

    /// Precomputed: While this permanent is on the battlefield, all players skip
    /// their untap steps (Stasis). Derived from a replacement of the shape
    /// `R:Event$ BeginPhase | Phase$ Untap | Skip$ True`.
    pub skips_untap_step: bool,

    /// Precomputed: Does this card require choosing a color on ETB?
    /// Derived from K:ETBReplacement:Other:ChooseColor lines
    pub etb_choose_color: bool,

    /// Colors to exclude from the choice (e.g., "green" for Thriving Grove)
    /// Derived from SVar:ChooseColor with Exclude$ parameter
    pub etb_exclude_colors: SmallVec<[Color; 1]>,

    /// Precomputed: Does this card require choosing a player on ETB?
    /// Derived from `K:ETBReplacement:Other:ChooseP` + an `SVar:ChooseP:DB$
    /// ChoosePlayer | Choices$ Player.Opponent` body (Black Vise). When set, the
    /// engine picks the chosen player deterministically at ETB (see
    /// `GameState::set_card_zone`) and stores it in `Card::chosen_player`.
    pub etb_choose_player: bool,

    /// Precomputed: While this permanent is UNTAPPED on the battlefield, players
    /// can't untap more than one land during their untap steps (Winter Orb).
    /// Derived from the static ability
    /// `S:Mode$ Continuous | Affected$ Player | AddKeyword$ UntapAdjust:Land:N |
    /// IsPresent$ Card.Self+untapped`. The runtime untap-limit lives in the
    /// untap step; the `IsPresent$ ...+untapped` self-condition is re-checked
    /// there against current board state, so the lock is rewind-safe (it is a
    /// pure function of which permanents are on the battlefield and tapped, with
    /// no per-turn flag). `N` is the per-untap-step land allowance (1 for
    /// Winter Orb).
    pub limits_land_untap: Option<u8>,

    /// Precomputed: Island Sanctuary draw-replacement enchantment.
    ///
    /// While this permanent is on the battlefield and it is the controller's
    /// draw step, the engine skips the mandatory draw and grants the controlling
    /// player "Island Sanctuary protection" for the turn (only creatures with
    /// flying or islandwalk may attack them). Derived from a replacement of the
    /// shape `R:Event$ Draw | ActivePhases$ Draw | PlayerTurn$ True | Optional$
    /// True | ...` (Island Sanctuary, 2nd Edition Alpha).
    pub is_island_sanctuary: bool,

    /// Precomputed: Does this card require choosing a named mode on ETB?
    /// Derived from `K:ETBReplacement:Other:<SVar>` where the SVar body is
    /// `DB$ GenericChoice | Choices$ <M1>,<M2>,...` (Palace Siege). When set,
    /// the engine picks the first mode choice per `etb_mode_ai_logic` at ETB and
    /// stores it in `Card::chosen_mode`.
    #[serde(default)]
    pub etb_choose_mode: bool,

    /// AI default mode for `etb_choose_mode` cards (from `AILogic$` in the SVar).
    /// `None` means pick the first choice listed. Palace Siege's SVar has
    /// `AILogic$ Dragons`, so the AI always chooses "Dragons".
    #[serde(default)]
    pub etb_mode_ai_logic: Option<String>,

    /// Ordered mode names for `etb_choose_mode` cards (from `Choices$` in the SVar).
    /// Populated from the comma-separated list in the `DB$ GenericChoice` SVar so
    /// `set_card_zone` can validate the AI choice without re-parsing the script.
    #[serde(default)]
    pub etb_mode_choices: Vec<String>,

    /// Precomputed: Does this card have an ETB "pay any amount of life" replacement?
    /// Derived from `R:Event$ Moved | Destination$ Battlefield | ReplaceWith$ PayLife`
    /// (Phyrexian Processor). When set, `set_card_zone` prompts for a life payment
    /// via AI heuristic, deducts it, and stores the amount in `Card::stored_int`.
    #[serde(default)]
    pub etb_pay_life: bool,

    /// Precomputed: Does this card require choosing a card name on ETB?
    /// Derived from `K:ETBReplacement:Other:<SVar>` where the SVar body is
    /// `DB$ NameCard | ...` (Pithing Needle).  When set, `set_card_zone` prompts
    /// the controller to name a card via AI heuristic and stores the result in
    /// `Card::chosen_name`.  The `CantBeActivatedByName` static then reads it.
    #[serde(default)]
    pub etb_choose_name: bool,
}

impl Default for CardCache {
    fn default() -> Self {
        CardCache::new("", "")
    }
}

impl CardCache {
    /// Create a new empty cache (default values)
    ///
    /// Call `update_from_abilities()` after parsing abilities to populate mana production.
    /// Call `update_from_types()` after types are set to populate type flags.
    /// This two-phase initialization is necessary because abilities/types are parsed after
    /// the Card struct is created.
    pub fn new(_text: &str, _name: &str) -> Self {
        // NOTE: text and name parameters are kept for API compatibility but no longer used.
        // Mana production is now derived from parsed abilities, not text.
        // Parse text for targeting restrictions
        let text_lower = _text.to_lowercase();

        CardCache {
            is_land: false,
            is_creature: false,
            is_artifact: false,
            is_instant: false,
            is_sorcery: false,
            is_enchantment: false,
            is_aura: false,
            is_equipment: false,
            mana_production: ManaProduction::default(),
            is_mana_source: false,
            requires_stack_target: false,

            // Spell targeting restrictions (parsed from oracle text)
            // "target land" means ONLY lands can be targeted (e.g., Sinkhole)
            spell_targets_land: text_lower.contains("target land")
                && !text_lower.contains("target creature")
                && !text_lower.contains("any target"),
            // "target creature" means ONLY creatures can be targeted (e.g., Terror)
            spell_targets_creature: (text_lower.contains("target creature")
                || text_lower.contains("target nonartifact")
                || text_lower.contains("target tapped creature")
                || text_lower.contains("target untapped creature"))
                && !text_lower.contains("any target"),
            // "target planeswalker" means planeswalkers can be targeted (Broadside Barrage)
            spell_targets_planeswalker: text_lower.contains("target planeswalker")
                || text_lower.contains("creature or planeswalker")
                || text_lower.contains("any target"),
            // "target player" for draw/life effects
            spell_targets_player: text_lower.contains("target player") || text_lower.contains("target opponent"),
            // "any target" means creatures or players (e.g., Lightning Bolt)
            spell_targets_any: text_lower.contains("any target"),

            // Relative per-target cost (Fireball). Set from the parsed RaiseCost
            // static ability in the card loader, not from oracle text.
            spell_relative_target_cost: false,

            land_evaluation_value: 0,

            // Land subtype flags (initialized to false, updated by update_from_subtypes)
            has_plains_subtype: false,
            has_island_subtype: false,
            has_swamp_subtype: false,
            has_mountain_subtype: false,
            has_forest_subtype: false,

            // ETB effects (initialized false, set from R:/K: lines in card loader)
            enters_tapped: false,
            skips_untap_step: false,
            etb_choose_color: false,
            etb_exclude_colors: SmallVec::new(),
            etb_choose_player: false,
            limits_land_untap: None,
            is_island_sanctuary: false,
            etb_choose_mode: false,
            etb_mode_ai_logic: None,
            etb_mode_choices: Vec::new(),
            etb_pay_life: false,
            etb_choose_name: false,
        }
    }

    /// Update cached type flags from the card's types vector
    ///
    /// Call this after types are set in the card loader. This pre-computes
    /// type checks to avoid Vec::contains() overhead at runtime.
    #[inline]
    pub fn update_from_types(&mut self, types: &[CardType]) {
        self.is_land = types.contains(&CardType::Land);
        self.is_creature = types.contains(&CardType::Creature);
        self.is_artifact = types.contains(&CardType::Artifact);
        self.is_instant = types.contains(&CardType::Instant);
        self.is_sorcery = types.contains(&CardType::Sorcery);
        self.is_enchantment = types.contains(&CardType::Enchantment);
    }

    /// Update cached land subtype flags from the card's subtypes and name
    ///
    /// Call this after subtypes are set in the card loader. This pre-computes
    /// land subtype checks to avoid eq_ignore_ascii_case() overhead at runtime.
    /// These flags are used in tap_for_mana_for_cost to determine mana colors.
    ///
    /// Also checks for Aura/Equipment subtypes to cache is_aura/is_equipment.
    /// And checks the card name as fallback for basic lands without explicit subtypes.
    #[inline]
    pub fn update_from_subtypes(&mut self, subtypes: &[crate::core::Subtype], card_name: &str) {
        // First check explicit subtypes
        for subtype in subtypes {
            let s = subtype.as_str();
            if s.eq_ignore_ascii_case("plains") {
                self.has_plains_subtype = true;
            } else if s.eq_ignore_ascii_case("island") {
                self.has_island_subtype = true;
            } else if s.eq_ignore_ascii_case("swamp") {
                self.has_swamp_subtype = true;
            } else if s.eq_ignore_ascii_case("mountain") {
                self.has_mountain_subtype = true;
            } else if s.eq_ignore_ascii_case("forest") {
                self.has_forest_subtype = true;
            } else if s.eq_ignore_ascii_case("aura") {
                // is_aura requires is_enchantment (set by update_from_types)
                // so is_aura will be true only if both conditions are met after both updates
                self.is_aura = self.is_enchantment;
            } else if s.eq_ignore_ascii_case("equipment") {
                // is_equipment requires is_artifact (set by update_from_types)
                self.is_equipment = self.is_artifact;
            }
        }

        // Fallback: check card name for basic lands without explicit subtypes
        // This handles test cards and basic lands that may lack subtype metadata
        //
        // IMPORTANT: Only apply this for Land cards to avoid false positives like
        // "Foggy Swamp Vinebender" (Creature with "Swamp" in name but not a land)
        if self.is_land {
            let name_lower = card_name.to_lowercase();
            if !self.has_plains_subtype && name_lower.contains("plains") {
                self.has_plains_subtype = true;
            }
            if !self.has_island_subtype && name_lower.contains("island") {
                self.has_island_subtype = true;
            }
            if !self.has_swamp_subtype && name_lower.contains("swamp") {
                self.has_swamp_subtype = true;
            }
            if !self.has_mountain_subtype && name_lower.contains("mountain") {
                self.has_mountain_subtype = true;
            }
            if !self.has_forest_subtype && name_lower.contains("forest") {
                self.has_forest_subtype = true;
            }
        }
    }

    /// Update cache based on parsed activated abilities and card name
    ///
    /// This derives mana production from Effect::AddMana in mana abilities,
    /// following the Java Forge approach where mana production comes from
    /// the structured `Produced$` parameter, not oracle text.
    ///
    /// Falls back to basic land name detection if no mana abilities exist,
    /// which handles test cards created without explicit abilities.
    ///
    /// Call this after parsing abilities in the card loader.
    pub fn update_from_abilities(&mut self, abilities: &[crate::core::ActivatedAbility]) {
        self.mana_production = Self::derive_mana_production_from_abilities(abilities);
        self.is_mana_source = self.mana_production.produces_mana();
    }

    /// Update cache based on abilities, with fallback to subtype detection
    ///
    /// This is the primary entry point for card loading. It:
    /// 1. Tries to derive mana production from parsed abilities
    /// 2. Falls back to land subtype-based detection (uses has_X_subtype flags)
    /// 3. Falls back to basic land name detection for test cards
    ///
    /// IMPORTANT: Call update_from_subtypes() BEFORE this method to ensure
    /// the subtype flags are set correctly for dual land detection.
    pub fn update_from_abilities_with_name(&mut self, abilities: &[crate::core::ActivatedAbility], name: &str) {
        self.mana_production = Self::derive_mana_production_from_abilities(abilities);

        // Fallback for lands without explicit mana abilities (basic lands, dual lands)
        // Per MTG rules 305.6, lands with basic land types have intrinsic mana abilities
        if !self.mana_production.produces_mana() {
            // First try subtype-based detection (correctly handles dual lands like Volcanic Island)
            // This uses the has_X_subtype flags set by update_from_subtypes()
            self.mana_production = self.derive_mana_production_from_subtypes();
        }

        // Final fallback for test cards (e.g., Card::new(..., "Mountain", ...) without subtypes)
        // IMPORTANT: Only apply for Land cards to avoid false positives like
        // "Foggy Swamp Vinebender" (Creature with "Swamp" in name but not a land)
        if !self.mana_production.produces_mana() && self.is_land {
            self.mana_production = Self::derive_mana_production_from_name(name);
        }

        self.is_mana_source = self.mana_production.produces_mana();
    }

    /// Derive mana production from land subtype flags
    ///
    /// Uses the has_X_subtype flags set by update_from_subtypes() to determine
    /// what colors this land can produce. Correctly handles dual lands.
    ///
    /// Per MTG rules 305.6, lands with basic land types have intrinsic mana abilities:
    /// - Plains produces {W}
    /// - Island produces {U}
    /// - Swamp produces {B}
    /// - Mountain produces {R}
    /// - Forest produces {G}
    fn derive_mana_production_from_subtypes(&self) -> ManaProduction {
        use crate::core::{ManaColor, ManaProductionKind};
        use crate::game::mana_colors::ManaColors;

        let mut colors = ManaColors::new();

        if self.has_plains_subtype {
            colors.insert(ManaColor::White);
        }
        if self.has_island_subtype {
            colors.insert(ManaColor::Blue);
        }
        if self.has_swamp_subtype {
            colors.insert(ManaColor::Black);
        }
        if self.has_mountain_subtype {
            colors.insert(ManaColor::Red);
        }
        if self.has_forest_subtype {
            colors.insert(ManaColor::Green);
        }

        let count = colors.len();
        if count == 0 {
            ManaProduction::default()
        } else if count == 1 {
            // Single color - use Fixed
            ManaProduction::free(ManaProductionKind::Fixed(colors.iter().next().unwrap()))
        } else {
            // Multiple colors - use Choice (for dual lands like Volcanic Island)
            ManaProduction::free(ManaProductionKind::Choice(colors))
        }
    }

    /// Derive mana production from basic land names (fallback for tests)
    ///
    /// This handles test cards that create lands like `Card::new(..., "Mountain", ...)`
    /// without adding explicit mana abilities.
    ///
    /// Public for use by Card::new() to enable simple test card creation.
    pub fn derive_mana_production_from_name(name: &str) -> ManaProduction {
        use crate::core::{ManaColor, ManaProductionKind};

        let name_lower = name.to_lowercase();
        if name_lower.contains("plains") {
            ManaProduction::free(ManaProductionKind::Fixed(ManaColor::White))
        } else if name_lower.contains("island") {
            ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Blue))
        } else if name_lower.contains("swamp") {
            ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Black))
        } else if name_lower.contains("mountain") {
            ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red))
        } else if name_lower.contains("forest") {
            ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green))
        } else if name_lower.contains("wastes") {
            ManaProduction::free(ManaProductionKind::Colorless)
        } else {
            ManaProduction::default()
        }
    }

    /// Set mana production directly (for tests and special cases)
    pub fn set_mana_production(&mut self, production: ManaProduction) {
        self.mana_production = production;
        self.is_mana_source = production.produces_mana();
    }

    /// Derive mana production from parsed activated abilities
    ///
    /// Scans all mana abilities (is_mana_ability = true) for Effect::AddMana
    /// and combines them into a single ManaProduction using OR semantics.
    ///
    /// Returns the upper bound of what colors this card can produce.
    ///
    /// # Panics
    ///
    /// This function does not panic. The internal unwrap() is guarded by a
    /// count check ensuring the iterator has at least one element.
    #[allow(clippy::missing_panics_doc)]
    pub fn derive_mana_production_from_abilities(abilities: &[crate::core::ActivatedAbility]) -> ManaProduction {
        use crate::core::{Effect, ManaColor, ManaProductionKind, ManaSideCost};
        use crate::game::mana_colors::ManaColors;

        let mut colors = ManaColors::new();
        let mut produces_colorless = false;
        let mut produces_any = false;
        // Maximum amount produced by any single mana ability. Sol Ring (`Amount$ 2`)
        // produces 2 colorless per activation; Black Lotus (`Amount$ 3 | Produced$ Any`)
        // produces 3 mana of one chosen color per activation. We track the OR of
        // abilities by taking the max (best single-source production).
        let mut max_amount: u8 = 0;
        // Lowest non-mana side cost across mana abilities
        // (None < Utility < PayLife < Sacrifice). We OR over abilities by
        // taking the *minimum* — if any way to tap is free, prefer that one.
        // The resolver still pays the actual ability's cost when the card is
        // activated; this is just a hint for ranking.
        let mut min_side_cost: Option<ManaSideCost> = None;
        // Does the card have any non-mana activated abilities? Used to
        // detect "utility" lands (Mishra's Factory animates, Strip Mine
        // destroys lands, etc.) so the resolver can prefer plain lands.
        let mut has_other_abilities = false;

        for ability in abilities {
            // Only consider mana abilities
            if !ability.is_mana_ability {
                if !ability.effects.is_empty() {
                    has_other_abilities = true;
                }
                continue;
            }

            // Inspect the ability cost for non-mana side effects (sacrifice / pay life).
            let ab_side_cost = derive_side_cost_from_cost(&ability.cost);
            min_side_cost = Some(match min_side_cost {
                Some(prev) => prev.min(ab_side_cost),
                None => ab_side_cost,
            });

            for effect in &ability.effects {
                if let Effect::AddMana { mana, .. } = effect {
                    // Check each color component
                    if mana.white > 0 {
                        colors.insert(ManaColor::White);
                    }
                    if mana.blue > 0 {
                        colors.insert(ManaColor::Blue);
                    }
                    if mana.black > 0 {
                        colors.insert(ManaColor::Black);
                    }
                    if mana.red > 0 {
                        colors.insert(ManaColor::Red);
                    }
                    if mana.green > 0 {
                        colors.insert(ManaColor::Green);
                    }
                    if mana.colorless > 0 {
                        produces_colorless = true;
                    }

                    // Check for "any color" - this is indicated by having all 5 colors set
                    // (from Produced$ Any which the effect converter handles)
                    // TODO(mtg-173): Track "any color" explicitly in Effect::AddMana
                    // For now, if all 5 colors are present, treat as any color
                    if mana.white > 0 && mana.blue > 0 && mana.black > 0 && mana.red > 0 && mana.green > 0 {
                        produces_any = true;
                    }

                    // The amount of mana this single activation produces is the max of
                    // any individual color/colorless component. For `Produced$ Any |
                    // Amount$ 3`, the converter fills mana with {W:3,U:3,B:3,R:3,G:3}
                    // (you choose ONE colour and get that amount of it). For
                    // `Produced$ C | Amount$ 2`, mana is {C:2}. For Plains, mana is
                    // {W:1}. Max across components captures the "per choice" amount.
                    let component_amount = mana
                        .white
                        .max(mana.blue)
                        .max(mana.black)
                        .max(mana.red)
                        .max(mana.green)
                        .max(mana.colorless);
                    if component_amount > max_amount {
                        max_amount = component_amount;
                    }
                }
            }
        }

        // Always at least 1 if we found any mana ability, otherwise 1 is the safe default.
        let amount = max_amount.max(1);
        // If the card's mana ability is itself free (no sacrifice / pay-life)
        // but the card has *other* activated abilities, treat it as Utility so
        // the resolver prefers plain lands first.
        let base_side_cost = min_side_cost.unwrap_or_default();
        let side_cost = if matches!(base_side_cost, ManaSideCost::None) && has_other_abilities {
            ManaSideCost::Utility
        } else {
            base_side_cost
        };

        // Build ManaProduction from accumulated colors
        if produces_any {
            return ManaProduction::with_amount(ManaProductionKind::AnyColor, amount).with_side_cost(side_cost);
        }

        // Cards that produce chosen color (like Thriving lands) need special handling.
        // At card loading time, we don't know the chosen color yet.
        // We DON'T mark as AnyColor because that would allow producing any color.
        // Instead, we return the static colors (e.g., Green from "Combo G Chosen").
        // At runtime, tap_for_mana_for_cost adds the card's chosen_color to available_colors.
        // The card will be classified as a complex source in the mana cache because
        // chosen_color.is_some() triggers complex source classification.
        //
        // Note: produces_chosen flag is tracked here but the actual color handling
        // happens via the Card.chosen_color field set at ETB time.

        match colors.len() {
            0 if produces_colorless => {
                ManaProduction::with_amount(ManaProductionKind::Colorless, amount).with_side_cost(side_cost)
            }
            0 => ManaProduction::default(), // No mana production
            1 => {
                // Single color - use Fixed variant
                let color = colors.iter().next().unwrap();
                ManaProduction::with_amount(ManaProductionKind::Fixed(color), amount).with_side_cost(side_cost)
            }
            _ => {
                // Multiple colors - use Choice variant (OR logic)
                ManaProduction::with_amount(ManaProductionKind::Choice(colors), amount).with_side_cost(side_cost)
            }
        }
    }
}

/// Inspect an `ActivatedAbility` cost and extract the worst non-mana side
/// cost component. Composite costs combine via max so e.g. `T Sac<1/CARDNAME>`
/// reports `Sacrifice`, while `T PayLife<1>` reports `PayLife(1)`.
///
/// Used by `derive_mana_production_from_abilities` so the resolver can rank
/// mana sources by activation expense (None < PayLife < Sacrifice).
fn derive_side_cost_from_cost(cost: &crate::core::Cost) -> crate::core::ManaSideCost {
    use crate::core::{Cost, ManaSideCost};
    match cost {
        Cost::Sacrifice { .. } => ManaSideCost::Sacrifice,
        Cost::SacrificePattern { .. } => ManaSideCost::Sacrifice,
        // Cap pay-life at u8::MAX; spells with PayLife<999> aren't realistic
        // for mana abilities and the side-cost score saturates anyway.
        Cost::PayLife { amount } => ManaSideCost::PayLife((*amount).clamp(1, i32::from(u8::MAX)) as u8),
        Cost::Composite(parts) => {
            // Combine sub-costs by taking the max (most expensive wins).
            let mut worst = ManaSideCost::None;
            for part in parts {
                let sub = derive_side_cost_from_cost(part);
                if sub > worst {
                    worst = sub;
                }
            }
            worst
        }
        // Tap, Untap, Mana, TapAndMana, Discard*, Waterbend, AddLoyalty,
        // SubLoyalty, SubCounter — none are "destructive" in the sense the
        // resolver cares about for ordering. (Discard is unusual for a mana
        // ability and is handled separately at activation time.)
        Cost::Tap
        | Cost::Untap
        | Cost::Mana(_)
        | Cost::TapAndMana(_)
        | Cost::ReturnToHand { .. }
        | Cost::Discard { .. }
        | Cost::DiscardHand
        | Cost::Waterbend { .. }
        | Cost::AddLoyalty { .. }
        | Cost::SubLoyalty { .. }
        | Cost::SubCounter { .. } => ManaSideCost::None,
    }
}

/// Represents a card in the game
///
/// Cards have a unique CardId but many cards can share the same card definition.
/// This struct represents the instance of a card during gameplay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    /// Unique ID for this card instance
    pub id: CardId,

    /// Card name (e.g., "Lightning Bolt")
    pub name: CardName,

    /// Printed name (e.g., "Lightning Bolt"), does not change when cloned
    #[serde(default)]
    pub printed_name: CardName,

    /// Mana cost
    pub mana_cost: ManaCost,

    /// Card types (a card can be multiple types)
    pub types: SmallVec<[CardType; 2]>,

    /// Card subtypes (e.g., "Goblin", "Warrior")
    pub subtypes: SmallVec<[Subtype; 3]>,

    /// Colors of the card
    pub colors: SmallVec<[Color; 2]>,

    /// Base/printed power (for creatures)
    /// PRIVATE: Use current_power() to get effective power (includes counters & bonuses)
    /// Use base_power() to read, set_base_power() to write
    base_power: Option<i8>,

    /// Base/printed toughness (for creatures)
    /// PRIVATE: Use current_toughness() to get effective toughness (includes counters & bonuses)
    /// Use base_toughness() to read, set_base_toughness() to write
    base_toughness: Option<i8>,

    /// Temporary power bonus (until end of turn)
    pub power_bonus: i32,

    /// Temporary toughness bonus (until end of turn)
    pub toughness_bonus: i32,

    /// Temporary base power override (until end of turn)
    /// PRIVATE: When Some(x), the creature's base power is x instead of its printed power
    /// Used by Animate effects like Flexible Waterbender
    /// Use set_temp_base_power() to write, clear_temp_base_stats() to reset
    temp_base_power: Option<i8>,

    /// Temporary base toughness override (until end of turn)
    /// PRIVATE: When Some(x), the creature's base toughness is x instead of its printed toughness
    /// Used by Animate effects like Flexible Waterbender
    /// Use set_temp_base_toughness() to write, clear_temp_base_stats() to reset
    temp_base_toughness: Option<i8>,

    /// Card types added by an until-end-of-turn `AB$ Animate | Types$ ...`
    /// effect (Mishra's Factory's `{1}: become a 2/2 Assembly-Worker artifact
    /// creature` is the canonical case). Removed during cleanup along with
    /// `temp_base_power` / `temp_base_toughness`.
    ///
    /// Only types that were *not already on the card* are recorded here, so
    /// cleanup can safely strip them without touching the printed type line.
    #[serde(default)]
    pub temp_animate_types: SmallVec<[CardType; 2]>,

    /// Subtypes added by an until-end-of-turn `AB$ Animate | Types$ ...`
    /// effect (e.g. `Assembly-Worker`). Cleared at end of turn.
    #[serde(default)]
    pub temp_animate_subtypes: SmallVec<[Subtype; 2]>,

    /// Subtypes that were temporarily *removed* by `RemoveCreatureTypes$ True`
    /// on an animate effect. Restored at end of turn so the card's printed
    /// creature subtype line returns intact.
    #[serde(default)]
    pub temp_removed_subtypes: SmallVec<[Subtype; 2]>,

    /// Keywords granted to this card by an "until end of turn" pump effect
    /// (`PumpCreature` / `PumpCreatureVariable` carrying `KW$` — e.g. Rockface
    /// Village's "gains haste until end of turn"). Mirrors `temp_base_power` /
    /// `temp_base_toughness`: the keyword bits are ALSO live in `keywords`, but
    /// this records which ones were granted-until-EOT so they can be removed
    /// deterministically by game position in BOTH the forward end-of-turn
    /// cleanup (`GameState::cleanup_temporary_effects`) AND the rewind
    /// per-turn-transient sweep (`UndoLog::rewind_to_turn_start`). Without this,
    /// a keyword granted on turn N and surviving until cleanup was only ever
    /// removed by the `GameAction::PumpCreature` undo, so a rewind landing at a
    /// turn boundary AFTER the grant (but the undo not unwinding past it) left
    /// the bit history-dependent across rewinds (mtg-610: Rockface Village haste
    /// at turn-12-start). PERMANENT keyword grants (Soulstone Sanctuary
    /// `Duration$ Permanent` Animate) deliberately do NOT route through here.
    ///
    /// NOTE: `#[serde(default)]` only (NOT `skip_serializing_if`): the snapshot/
    /// resume + undo-log paths serialize `Card` with bincode, a non-self-
    /// describing format that mishandles conditionally-skipped fields (a skipped
    /// field corrupts the byte stream on deserialize). `default` covers loading
    /// older JSON snapshots that predate the field.
    #[serde(default)]
    pub temp_keywords_until_eot: KeywordSet,

    /// Damage marked on this permanent (cleared at end of turn per CR 704.5g)
    /// Only meaningful for creatures on the battlefield
    pub damage: i32,

    /// Sources that have dealt damage to this card this turn. Cleared at the
    /// cleanup step alongside `damage`. Drives "Whenever a creature dealt
    /// damage by CARDNAME this turn dies, ..." triggers — Sengir Vampire,
    /// Baron Sengir, Abattoir Ghoul, Blood Cultist, Garza Zol, etc.
    /// (`ValidCard$ Creature.DamagedBy`).
    #[serde(default)]
    pub damaged_by_this_turn: SmallVec<[CardId; 2]>,

    /// Oracle text
    pub text: String,

    /// Current zone owner (player who owns this card)
    pub owner: PlayerId,

    /// Current controller (can differ from owner)
    pub controller: PlayerId,

    /// Is the card tapped?
    pub tapped: bool,

    /// Turn number when this permanent entered the battlefield
    /// Used for summoning sickness (creatures can't attack the turn they enter)
    /// None = not on battlefield yet, Some(turn) = entered on this turn
    pub turn_entered_battlefield: Option<u32>,

    /// Counters on this card (using SmallVec for efficiency)
    /// Common counters: +1/+1, -1/-1, charge, loyalty
    pub counters: SmallVec<[(CounterType, u8); 2]>,

    /// Keyword abilities (Flying, First Strike, etc.)
    /// Now uses KeywordSet for efficient O(1) simple keyword lookups
    pub keywords: KeywordSet,

    /// Effects that execute when this card resolves
    /// For spells: effects execute when spell resolves
    /// For permanents: effects may be triggered or activated abilities
    pub effects: Vec<Effect>,

    /// Triggered abilities (ETB, phase triggers, etc.)
    /// These execute automatically when their trigger condition is met
    pub triggers: Vec<Trigger>,

    /// Activated abilities (costs and effects)
    /// These can be activated by paying their cost
    pub activated_abilities: Vec<crate::core::ActivatedAbility>,

    /// Static abilities that create continuous effects
    /// Example: Equipment giving +2/+2 to equipped creature
    /// Applied via CR 613 layer system
    pub static_abilities: Vec<crate::core::StaticAbility>,

    /// Equipment/Aura attachment tracking
    /// - For Equipment/Aura: points to the creature this is attached to
    /// - For other cards: should be None
    /// - Used to track Equipment→Creature and Aura→Permanent relationships
    pub attached_to: Option<CardId>,

    /// If this permanent's controller was changed by a control-stealing Aura
    /// (`S:Mode$ Continuous | GainControl$ You` — Control Magic, Mind Control,
    /// Persuasion, ...), this records the Aura granting that control. It lets
    /// [`crate::game::GameState::recompute_aura_control`] revert control to the
    /// owner the moment that Aura leaves the battlefield, WITHOUT clobbering
    /// control gained through other mechanisms (Animate Dead's one-shot
    /// `GainControl$ True` on a ChangeZone effect, Threaten's `AB$ GainControl`).
    /// `None` for everything not under aura-granted control.
    #[serde(default)]
    pub control_from_aura: Option<CardId>,

    /// If this permanent's controller was changed by a one-shot `AB$ GainControl`
    /// with a source-dependent duration (`LoseControl$ LeavesPlay,LoseControl` —
    /// Aladdin: "for as long as you control Aladdin"), this records
    /// `(source, grantee)`: the permanent whose continued control by `grantee`
    /// sustains the grant. [`crate::game::GameState::recompute_source_control`]
    /// reverts control to the owner the moment `grantee` stops controlling
    /// `source` (it leaves the battlefield or its controller changes), mirroring
    /// the self-correcting Aura-control pass (CR 613 / 800.4a). `None` for
    /// everything not under source-duration control.
    #[serde(default)]
    pub control_grant: Option<(CardId, PlayerId)>,

    /// Chosen color for cards with "choose a color" effects (e.g., Thriving lands)
    /// Set when the card enters the battlefield, affects what mana it can produce
    pub chosen_color: Option<Color>,

    /// Chosen player for cards with an "as ~ enters, choose a player" replacement
    /// effect (Black Vise: `K:ETBReplacement:Other:ChooseP` +
    /// `DB$ ChoosePlayer | Choices$ Player.Opponent`). Set when the card enters
    /// the battlefield (see `GameState::set_card_zone`), part of serialized game
    /// state (snapshot/resume + undo + state hash), and read by the
    /// `ValidPlayer$ Player.Chosen` trigger gate and `Defined$ ChosenPlayer`
    /// damage resolution. `None` for cards without such a replacement, or before
    /// the card has entered the battlefield.
    #[serde(default)]
    pub chosen_player: Option<PlayerId>,

    /// Mode chosen by `K:ETBReplacement:Other:<SVar>` + `DB$ GenericChoice` at ETB
    /// time. Palace Siege chooses "Khans" or "Dragons"; the value is the raw
    /// mode string from `Choices$` (e.g. `"Khans"` or `"Dragons"`). Stored as
    /// serialized state so rewind/replay reconstruct the same trigger gating.
    #[serde(default)]
    pub chosen_mode: Option<String>,

    /// Card name chosen when a "choose a card name" ETB replacement fires
    /// (Pithing Needle: `K:ETBReplacement:Other:DBNameCard`).  Read by
    /// `StaticAbility::CantBeActivatedByName` to identify which sources are
    /// locked out and by `Card.NamedCard` filters in ChangeZoneAll / Exile
    /// (Cranial Extraction).  Serialized so snapshot/resume, undo, and
    /// WASM rewind reconstruct the chosen name identically.  `None` when the
    /// card never triggers a name-choice.
    #[serde(default)]
    pub chosen_name: Option<String>,

    /// Per-card integer stored by an ETB replacement (Phyrexian Processor: life paid
    /// as it entered the battlefield). Read back by a later activated ability that
    /// creates a token whose P/T equals this value (`TokenPower$ LifePaidOnETB`).
    /// Serialized so snapshot/resume + undo + rewind reconstruct the amount identically.
    /// `None` means no amount has been stored yet.
    #[serde(default)]
    pub stored_int: Option<u32>,

    /// Script variables (SVars) for SubAbility chaining
    /// Key: SVar name (e.g., "BalanceHands")
    /// Value: SVar body (e.g., "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures")
    /// Used for SubAbility$ resolution during effect execution
    pub svars: std::collections::HashMap<String, String>,

    /// Bitmask tracking which players have seen this card's identity
    /// Per NETWORK_ARCHITECTURE.md, this enables deduplication at log time.
    /// Bit 0 = PlayerId(0), Bit 1 = PlayerId(1), etc.
    /// Used to avoid logging redundant RevealCard actions.
    pub revealed_to_mask: u8,

    /// Is this a legendary permanent?
    /// Used for legendary rule (MTG CR 704.5j)
    pub is_legendary: bool,

    /// Is this card a commander? (Commander format only)
    /// Set during game initialization for cards designated as commander.
    /// Used for commander tax, commander damage tracking, and zone-change replacement.
    #[serde(default)]
    pub is_commander: bool,

    /// Is this a token? (created by effects, not in a deck)
    /// Set when tokens are created via Effect::CreateToken.
    /// Used by continuous effects (e.g., Intangible Virtue: "Creature tokens you control get +1/+1").
    #[serde(default)]
    pub is_token: bool,

    /// Has a loyalty ability been activated on this permanent this turn?
    /// MTG CR 606.3: Only one loyalty ability per planeswalker per turn.
    /// Reset at the start of each turn.
    #[serde(default)]
    pub loyalty_activated_this_turn: bool,

    /// Regeneration shields active on this permanent (cleared at end of turn)
    /// Each successful AB$ Regenerate activation adds one shield.
    /// When the creature would be destroyed, a shield is consumed instead:
    /// tap, remove all damage, remove from combat (CR 701.15a).
    pub regeneration_shields: u8,

    /// Damage prevention shield (cleared at end of turn)
    /// Prevents the next N damage that would be dealt to this permanent.
    /// Set by AB$ PreventDamage effects (CR 615.1).
    #[serde(default)]
    pub damage_prevention: i32,

    /// The value of X chosen when casting this spell (MTG CR 601.2b)
    /// Set during step 2 of the 8-step casting process for spells with X in their mana cost.
    /// Used at resolution time to determine effect amounts (damage, cards drawn, etc.)
    /// via SVar:X:Count$xPaid references in card scripts.
    #[serde(default)]
    pub x_paid: u8,

    /// The number of times Multikicker was paid when casting this spell (CR 702.33a).
    /// Set by the priority loop when the caster opts to pay the Multikicker additional
    /// cost one or more times. Used at resolution to evaluate `Count$TimesKicked` SVars
    /// (e.g. Everflowing Chalice `K:etbCounter:CHARGE:XKicked` where
    /// `SVar:XKicked:Count$TimesKicked`).
    #[serde(default)]
    pub times_kicked: u8,

    /// Whether the Bargain optional additional cost (CR 702.162) was paid when
    /// casting this spell — i.e. the caster sacrificed an artifact, enchantment,
    /// or token. Set by the priority loop just before `cast_spell_8_step`; cleared
    /// in the cleanup step. Drives `CountExpression::Bargain` evaluation (Torch
    /// the Tower `SVar:X:Count$Bargain.3.2`) and `Condition$ Bargain` sub-effects
    /// (the "you scry 1" rider on Torch the Tower).
    #[serde(default)]
    pub bargain_paid: bool,

    /// Set to `true` when the AI pays the Kicker additional cost (CR 702.32) for
    /// this spell. Cleared in the cleanup step. Drives `CountExpression::Kicked`
    /// evaluation (Firebending Lesson `SVar:X:Count$Kicked.5.2` — deals 5 when
    /// kicked, 2 when not). Serialized so network-shadow + rewind reconstruct the
    /// same decision. Distinct from `times_kicked` which tracks Multikicker
    /// payment count; this tracks the simpler single-Kicker optional cost.
    #[serde(default)]
    pub kicker_paid: bool,

    /// Extra generic mana cost paid when choosing a mode for a tiered modal spell
    /// (from `ModeCost$` in the chosen mode's SVar). Set by `apply_selected_modes`
    /// after mode selection in the priority loop, then added to effective cost in
    /// `compute_effective_cost`. Zero when no extra mode cost applies.
    /// Cleared in `reset_transient_state`. Serialized for rewind/replay.
    #[serde(default)]
    pub mode_cost_paid: u8,

    /// Set to `true` when the caster pays the Offspring additional cost (CR 702.198)
    /// for this creature spell. Cleared in the cleanup step. When `true` and the
    /// creature enters the battlefield, the engine creates a 1/1 token copy of it
    /// (CR 702.198a). Serialized so network-shadow + rewind reconstruct the same
    /// decision. Mirrors `kicker_paid`.
    #[serde(default)]
    pub offspring_paid: bool,

    /// If set, a zone-change replacement applies: should this creature die this
    /// turn, it is exiled instead of going to the graveyard (CR 614). Set by
    /// `Effect::ExileIfWouldDieThisTurn` (Disintegrate's
    /// `ReplaceDyingDefined$ ThisTargetedCard.Creature` clause) and cleared at
    /// the cleanup step. Honored by `GameState::death_destination_for_card`.
    #[serde(default)]
    pub exile_if_would_die_this_turn: bool,

    /// If set, a zone-change replacement applies: should this card go to the
    /// graveyard this turn (e.g. after resolving as an instant/sorcery), it is
    /// exiled instead (CR 614). Set by `Effect::PlayFromGraveyard` (Chandra,
    /// Acolyte of Flame's −2 `ReplaceGraveyard$ Exile` clause) and cleared at
    /// the cleanup step. Honored by `resolve_spell_finalize`.
    #[serde(default)]
    pub exile_if_would_go_to_graveyard_this_turn: bool,

    /// Maze of Ith: prevent ALL combat damage this creature would deal OR receive
    /// this turn (CR 615 replacement effect, "prevent all combat damage dealt to
    /// and dealt by CARDNAME"). Set by `Effect::PreventAllCombatDamageThisTurn`
    /// and cleared in the cleanup step.
    #[serde(default)]
    pub prevent_all_combat_damage_this_turn: bool,

    /// Set when this creature has dealt damage to an opponent (a player who is
    /// not its controller) this turn. Drives intervening-if triggers gated on
    /// `IsPresent$ Card.Self+dealtDamageToOppThisTurn` — Whirling Dervish's "at
    /// the beginning of each end step, if CARDNAME dealt damage to an opponent
    /// this turn, put a +1/+1 counter on it" (CR 603.4 intervening-if). Cleared
    /// at the cleanup step (CR 514.2). Set+logged in the combat-damage step as a
    /// reversible `GameAction::MarkDealtDamageToOpponent` so rewind/replay
    /// restores it exactly (same per-turn-state contract as `damaged_by_this_turn`).
    #[serde(default)]
    pub dealt_damage_to_opponent_this_turn: bool,

    /// Set when this creature has been declared as an attacker this turn. Drives
    /// the `Card.attackedThisTurn` intervening-if (CR 603.4) on Berserk's
    /// delayed end-step trigger — "At the beginning of the next end step,
    /// destroy that creature if it attacked this turn." Unlike combat's
    /// transient `is_attacking` (true only during the combat phase), this flag
    /// persists through the rest of the turn so the end-step trigger can read
    /// it. Cleared at the cleanup step (CR 514.2). Set+logged in the
    /// declare-attackers step as a reversible
    /// `GameAction::MarkAttackedThisTurn` so rewind/replay restores it exactly
    /// (same per-turn-state contract as `dealt_damage_to_opponent_this_turn`).
    #[serde(default)]
    pub attacked_this_turn: bool,

    /// Indices of exhausted activated abilities (can only be activated once per game)
    /// When an exhaust ability resolves, its index is added here to prevent reactivation
    pub exhausted_abilities: SmallVec<[usize; 1]>,

    /// Set while this card is being cast as its Adventure (instant/sorcery) half
    /// (CR 715). When `true`, the card's live `name`/`mana_cost`/`types`/`effects`
    /// have been swapped to the Adventure face (the creature face is preserved in
    /// the `RestoreCardState` snapshot logged at swap time). On resolution
    /// (`resolve_spell_finalize`) an Adventure spell is EXILED "on an adventure"
    /// instead of going to the graveyard, the creature face is restored, and the
    /// owner is granted permission to cast the creature half from exile.
    ///
    /// Serialized (`#[serde(default)]`) so the network-shadow / snapshot / WASM
    /// rewind paths reconstruct the in-flight Adventure cast identically — this
    /// is real, choice-spanning game state, never `#[serde(skip)]` scratch.
    #[serde(default)]
    pub cast_as_adventure: bool,

    /// Original card definition this was instantiated from
    /// Stored as owned copy for name-based card evaluation (library search, etc.)
    /// Inline storage avoids pointer indirection when accessing definition fields
    pub definition: CardDefinition,
}

/// Snapshot of the copiable characteristics a Clone overwrites (CR 707.2),
/// captured by [`Card::capture_copiable_state`] before
/// [`crate::game::GameState::apply_clone`] mutates the card and restored by
/// [`Card::restore_copiable_state`] when the undo log reverses the clone
/// (mtg-559/mtg-610). Boxed at the `GameAction::CloneCard` use-site to keep the
/// `GameAction` enum small.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardCopiableState {
    pub name: CardName,
    pub mana_cost: ManaCost,
    pub types: SmallVec<[CardType; 2]>,
    pub subtypes: SmallVec<[Subtype; 3]>,
    pub colors: SmallVec<[Color; 2]>,
    pub base_power: Option<i8>,
    pub base_toughness: Option<i8>,
    pub text: String,
    pub is_legendary: bool,
    pub keywords: KeywordSet,
    pub activated_abilities: Vec<crate::core::ActivatedAbility>,
    pub static_abilities: Vec<crate::core::StaticAbility>,
    pub triggers: Vec<Trigger>,
    pub svars: std::collections::HashMap<String, String>,
    pub definition: CardDefinition,
}

/// Snapshot of all transient and copiable fields of a Card
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardStateSnapshot {
    pub name: CardName,
    pub mana_cost: ManaCost,
    pub types: SmallVec<[CardType; 2]>,
    pub subtypes: SmallVec<[Subtype; 3]>,
    pub colors: SmallVec<[Color; 2]>,
    pub base_power: Option<i8>,
    pub base_toughness: Option<i8>,
    pub power_bonus: i32,
    pub toughness_bonus: i32,
    pub temp_base_power: Option<i8>,
    pub temp_base_toughness: Option<i8>,
    pub temp_animate_types: SmallVec<[CardType; 2]>,
    pub temp_animate_subtypes: SmallVec<[Subtype; 2]>,
    pub temp_removed_subtypes: SmallVec<[Subtype; 2]>,
    pub temp_keywords_until_eot: KeywordSet,
    pub damage: i32,
    pub damaged_by_this_turn: SmallVec<[CardId; 2]>,
    pub text: String,
    pub tapped: bool,
    pub turn_entered_battlefield: Option<u32>,
    pub counters: SmallVec<[(CounterType, u8); 2]>,
    pub keywords: KeywordSet,
    pub effects: Vec<Effect>,
    pub triggers: Vec<Trigger>,
    pub activated_abilities: Vec<crate::core::ActivatedAbility>,
    pub static_abilities: Vec<crate::core::StaticAbility>,
    pub attached_to: Option<CardId>,
    pub control_from_aura: Option<CardId>,
    pub control_grant: Option<(CardId, PlayerId)>,
    pub chosen_color: Option<Color>,
    pub chosen_player: Option<PlayerId>,
    /// Mode chosen by `K:ETBReplacement:Other:<SVar>` + `DB$ GenericChoice` at ETB
    /// time. Palace Siege chooses "Khans" or "Dragons"; the value is the raw
    /// mode string from `Choices$` (e.g. `"Khans"` or `"Dragons"`). Stored as
    /// serialized state so rewind/replay reconstruct the same trigger gating.
    #[serde(default)]
    pub chosen_mode: Option<String>,
    /// Card name chosen at ETB for "choose a card name" effects (Pithing Needle).
    /// Mirrors the same field on `Card`.  `None` for cards that never trigger a
    /// name-choice, or before the card has entered the battlefield.
    #[serde(default)]
    pub chosen_name: Option<String>,
    /// Per-card integer stored by an ETB replacement (Phyrexian Processor: life paid).
    #[serde(default)]
    pub stored_int: Option<u32>,
    pub svars: std::collections::HashMap<String, String>,
    pub is_legendary: bool,
    pub loyalty_activated_this_turn: bool,
    pub regeneration_shields: u8,
    pub damage_prevention: i32,
    pub x_paid: u8,
    pub times_kicked: u8,
    #[serde(default)]
    pub bargain_paid: bool,
    #[serde(default)]
    pub kicker_paid: bool,
    #[serde(default)]
    pub offspring_paid: bool,
    #[serde(default)]
    pub mode_cost_paid: u8,
    pub exile_if_would_die_this_turn: bool,
    pub exile_if_would_go_to_graveyard_this_turn: bool,
    pub prevent_all_combat_damage_this_turn: bool,
    pub dealt_damage_to_opponent_this_turn: bool,
    pub attacked_this_turn: bool,
    pub exhausted_abilities: SmallVec<[usize; 1]>,
    #[serde(default)]
    pub cast_as_adventure: bool,
    pub definition: CardDefinition,
}

impl Card {
    pub fn new(id: CardId, name: impl Into<CardName>, owner: PlayerId) -> Self {
        let name: CardName = name.into();
        let text = String::new();

        // Initialize cache with name-based fallback for basic lands
        // This allows test code to create lands with just Card::new(..., "Mountain", ...)
        // without needing to add explicit mana abilities
        let mut cache = CardCache::new(&text, name.as_str());
        cache.mana_production = CardCache::derive_mana_production_from_name(name.as_str());
        cache.is_mana_source = cache.mana_production.produces_mana();

        // Create definition with populated cache
        let definition = CardDefinition {
            cache,
            ..Default::default()
        };

        Card {
            id,
            printed_name: name.clone(),
            name,
            mana_cost: ManaCost::new(),
            types: SmallVec::new(),
            subtypes: SmallVec::new(),
            colors: SmallVec::new(),
            base_power: None,
            base_toughness: None,
            power_bonus: 0,
            toughness_bonus: 0,
            temp_base_power: None,
            temp_base_toughness: None,
            temp_animate_types: SmallVec::new(),
            temp_animate_subtypes: SmallVec::new(),
            temp_removed_subtypes: SmallVec::new(),
            temp_keywords_until_eot: KeywordSet::new(),
            damage: 0,
            damaged_by_this_turn: SmallVec::new(),
            text,
            owner,
            controller: owner,
            tapped: false,
            turn_entered_battlefield: None,
            counters: SmallVec::new(),
            keywords: KeywordSet::new(),
            effects: Vec::new(),
            triggers: Vec::new(),
            activated_abilities: Vec::new(),
            static_abilities: Vec::new(),
            attached_to: None,
            control_from_aura: None,
            control_grant: None,
            chosen_color: None,
            chosen_player: None,
            chosen_mode: None,
            chosen_name: None,
            stored_int: None,
            svars: std::collections::HashMap::new(),
            revealed_to_mask: 0,
            is_legendary: false,
            is_commander: false,
            is_token: false,
            loyalty_activated_this_turn: false,
            regeneration_shields: 0,
            damage_prevention: 0,
            x_paid: 0,
            times_kicked: 0,
            bargain_paid: false,
            kicker_paid: false,
            offspring_paid: false,
            mode_cost_paid: 0,
            exile_if_would_die_this_turn: false,
            exile_if_would_go_to_graveyard_this_turn: false,
            prevent_all_combat_damage_this_turn: false,
            dealt_damage_to_opponent_this_turn: false,
            attacked_this_turn: false,
            exhausted_abilities: SmallVec::new(),
            cast_as_adventure: false,
            definition,
        }
    }

    /// Capture a snapshot of all transient and copiable fields
    pub fn capture_state_snapshot(&self) -> CardStateSnapshot {
        CardStateSnapshot {
            name: self.name.clone(),
            mana_cost: self.mana_cost,
            types: self.types.clone(),
            subtypes: self.subtypes.clone(),
            colors: self.colors.clone(),
            base_power: self.base_power,
            base_toughness: self.base_toughness,
            power_bonus: self.power_bonus,
            toughness_bonus: self.toughness_bonus,
            temp_base_power: self.temp_base_power,
            temp_base_toughness: self.temp_base_toughness,
            temp_animate_types: self.temp_animate_types.clone(),
            temp_animate_subtypes: self.temp_animate_subtypes.clone(),
            temp_removed_subtypes: self.temp_removed_subtypes.clone(),
            temp_keywords_until_eot: self.temp_keywords_until_eot.clone(),
            damage: self.damage,
            damaged_by_this_turn: self.damaged_by_this_turn.clone(),
            text: self.text.clone(),
            tapped: self.tapped,
            turn_entered_battlefield: self.turn_entered_battlefield,
            counters: self.counters.clone(),
            keywords: self.keywords.clone(),
            effects: self.effects.clone(),
            triggers: self.triggers.clone(),
            activated_abilities: self.activated_abilities.clone(),
            static_abilities: self.static_abilities.clone(),
            attached_to: self.attached_to,
            control_from_aura: self.control_from_aura,
            control_grant: self.control_grant,
            chosen_color: self.chosen_color,
            chosen_player: self.chosen_player,
            chosen_mode: self.chosen_mode.clone(),
            chosen_name: self.chosen_name.clone(),
            stored_int: self.stored_int,
            svars: self.svars.clone(),
            is_legendary: self.is_legendary,
            loyalty_activated_this_turn: self.loyalty_activated_this_turn,
            regeneration_shields: self.regeneration_shields,
            damage_prevention: self.damage_prevention,
            x_paid: self.x_paid,
            times_kicked: self.times_kicked,
            bargain_paid: self.bargain_paid,
            kicker_paid: self.kicker_paid,
            offspring_paid: self.offspring_paid,
            mode_cost_paid: self.mode_cost_paid,
            exile_if_would_die_this_turn: self.exile_if_would_die_this_turn,
            exile_if_would_go_to_graveyard_this_turn: self.exile_if_would_go_to_graveyard_this_turn,
            prevent_all_combat_damage_this_turn: self.prevent_all_combat_damage_this_turn,
            dealt_damage_to_opponent_this_turn: self.dealt_damage_to_opponent_this_turn,
            attacked_this_turn: self.attacked_this_turn,
            exhausted_abilities: self.exhausted_abilities.clone(),
            cast_as_adventure: self.cast_as_adventure,
            definition: self.definition.clone(),
        }
    }

    /// Restore the card state from a snapshot
    pub fn restore_state_snapshot(&mut self, snapshot: CardStateSnapshot) {
        self.name = snapshot.name;
        self.mana_cost = snapshot.mana_cost;
        self.types = snapshot.types;
        self.subtypes = snapshot.subtypes;
        self.colors = snapshot.colors;
        self.base_power = snapshot.base_power;
        self.base_toughness = snapshot.base_toughness;
        self.power_bonus = snapshot.power_bonus;
        self.toughness_bonus = snapshot.toughness_bonus;
        self.temp_base_power = snapshot.temp_base_power;
        self.temp_base_toughness = snapshot.temp_base_toughness;
        self.temp_animate_types = snapshot.temp_animate_types;
        self.temp_animate_subtypes = snapshot.temp_animate_subtypes;
        self.temp_removed_subtypes = snapshot.temp_removed_subtypes;
        self.temp_keywords_until_eot = snapshot.temp_keywords_until_eot;
        self.damage = snapshot.damage;
        self.damaged_by_this_turn = snapshot.damaged_by_this_turn;
        self.text = snapshot.text;
        self.tapped = snapshot.tapped;
        self.turn_entered_battlefield = snapshot.turn_entered_battlefield;
        self.counters = snapshot.counters;
        self.keywords = snapshot.keywords;
        self.effects = snapshot.effects;
        self.triggers = snapshot.triggers;
        self.activated_abilities = snapshot.activated_abilities;
        self.static_abilities = snapshot.static_abilities;
        self.attached_to = snapshot.attached_to;
        self.control_from_aura = snapshot.control_from_aura;
        self.control_grant = snapshot.control_grant;
        self.chosen_color = snapshot.chosen_color;
        self.chosen_player = snapshot.chosen_player;
        self.chosen_mode = snapshot.chosen_mode;
        self.chosen_name = snapshot.chosen_name;
        self.stored_int = snapshot.stored_int;
        self.svars = snapshot.svars;
        self.is_legendary = snapshot.is_legendary;
        self.loyalty_activated_this_turn = snapshot.loyalty_activated_this_turn;
        self.regeneration_shields = snapshot.regeneration_shields;
        self.damage_prevention = snapshot.damage_prevention;
        self.x_paid = snapshot.x_paid;
        self.times_kicked = snapshot.times_kicked;
        self.bargain_paid = snapshot.bargain_paid;
        self.kicker_paid = snapshot.kicker_paid;
        self.offspring_paid = snapshot.offspring_paid;
        self.mode_cost_paid = snapshot.mode_cost_paid;
        self.exile_if_would_die_this_turn = snapshot.exile_if_would_die_this_turn;
        self.exile_if_would_go_to_graveyard_this_turn = snapshot.exile_if_would_go_to_graveyard_this_turn;
        self.prevent_all_combat_damage_this_turn = snapshot.prevent_all_combat_damage_this_turn;
        self.dealt_damage_to_opponent_this_turn = snapshot.dealt_damage_to_opponent_this_turn;
        self.attacked_this_turn = snapshot.attacked_this_turn;
        self.exhausted_abilities = snapshot.exhausted_abilities;
        self.cast_as_adventure = snapshot.cast_as_adventure;
        self.definition = snapshot.definition;
    }

    /// Reset transient state when a card leaves the battlefield
    pub fn reset_transient_state(&mut self, original_def: Option<&CardDefinition>) {
        if let Some(def) = original_def {
            self.name = def.name.clone();
            self.mana_cost = def.mana_cost;
            self.types = SmallVec::from_slice(&def.types);
            self.subtypes = def.subtypes.iter().cloned().collect();
            self.colors = SmallVec::from_slice(&def.colors);
            self.base_power = def.power;
            self.base_toughness = def.toughness;
            self.text = def.oracle.clone();
            self.is_legendary = def.is_legendary;
            self.definition = def.clone();

            // Rebuild cache
            self.definition.cache = crate::core::CardCache::new(&self.text, self.name.as_str());
            self.definition.cache.update_from_types(&self.types);
            self.definition
                .cache
                .update_from_subtypes(&self.subtypes, self.name.as_str());
            self.definition.cache.enters_tapped = def.enters_tapped;
            self.definition.cache.skips_untap_step = def.skips_untap_step();
            self.definition.cache.limits_land_untap = def.limits_land_untap();
            self.definition.cache.etb_choose_color = def.etb_choose_color;
            self.definition.cache.etb_exclude_colors = SmallVec::from_slice(&def.etb_exclude_colors);
            self.definition.cache.etb_choose_player = def.etb_choose_player;
            self.definition.cache.etb_choose_mode = def.etb_choose_mode;
            self.definition.cache.etb_mode_ai_logic = def.etb_mode_ai_logic.clone();
            self.definition.cache.etb_mode_choices = def.etb_mode_choices.clone();
            self.definition.cache.etb_pay_life = def.etb_pay_life;
            self.definition.cache.etb_choose_name = def.etb_choose_name;
            self.definition.cache.spell_relative_target_cost = def.has_relative_self_target_cost();
        } else {
            self.name = self.printed_name.clone();
        }

        self.power_bonus = 0;
        self.toughness_bonus = 0;
        self.temp_base_power = None;
        self.temp_base_toughness = None;
        self.temp_animate_types = SmallVec::new();
        self.temp_animate_subtypes = SmallVec::new();
        self.temp_removed_subtypes = SmallVec::new();
        self.temp_keywords_until_eot = KeywordSet::new();
        self.damage = 0;
        self.damaged_by_this_turn = SmallVec::new();
        self.tapped = false;
        self.turn_entered_battlefield = None;
        self.counters = SmallVec::new();
        // Clear the per-card stored integer (e.g. Phyrexian Processor's life-paid).
        // The value is tied to a specific battlefield tenure; it resets when the card
        // leaves so a future re-ETB starts fresh.
        self.stored_int = None;

        if let Some(def) = original_def {
            self.keywords = def.parse_keywords();
            self.effects = def.parse_effects();
            self.triggers = def.parse_triggers();
            self.activated_abilities = def.parse_activated_abilities();
            self.static_abilities = def.parse_static_abilities();
            self.svars = def.svars.clone();
        } else {
            self.keywords = self.definition.parse_keywords();
            self.effects = self.definition.parse_effects();
            self.triggers = self.definition.parse_triggers();
            self.activated_abilities = self.definition.parse_activated_abilities();
            self.static_abilities = self.definition.parse_static_abilities();
            self.svars = self.definition.svars.clone();
        }

        // Implicit mana abilities for land cards
        if self.is_land() && !self.activated_abilities.iter().any(|ab| ab.is_mana_ability) {
            use crate::core::{ActivatedAbility, Cost, Effect, PlayerId};

            let has_plains = self.subtypes.iter().any(|st| st.as_str() == "Plains");
            let has_island = self.subtypes.iter().any(|st| st.as_str() == "Island");
            let has_swamp = self.subtypes.iter().any(|st| st.as_str() == "Swamp");
            let has_mountain = self.subtypes.iter().any(|st| st.as_str() == "Mountain");
            let has_forest = self.subtypes.iter().any(|st| st.as_str() == "Forest");

            if has_plains {
                let mana = ManaCost::from_string("W");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                        amount_var: None,
                    }],
                    "Add {W}".to_string(),
                    true,
                );
                self.activated_abilities.push(ability);
            }
            if has_island {
                let mana = ManaCost::from_string("U");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                        amount_var: None,
                    }],
                    "Add {U}".to_string(),
                    true,
                );
                self.activated_abilities.push(ability);
            }
            if has_swamp {
                let mana = ManaCost::from_string("B");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                        amount_var: None,
                    }],
                    "Add {B}".to_string(),
                    true,
                );
                self.activated_abilities.push(ability);
            }
            if has_mountain {
                let mana = ManaCost::from_string("R");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                        amount_var: None,
                    }],
                    "Add {R}".to_string(),
                    true,
                );
                self.activated_abilities.push(ability);
            }
            if has_forest {
                let mana = ManaCost::from_string("G");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                        amount_var: None,
                    }],
                    "Add {G}".to_string(),
                    true,
                );
                self.activated_abilities.push(ability);
            }
        }

        // Implicit Equip ability for equipment
        if self.is_artifact() && self.subtypes.iter().any(|st| st.as_str() == "Equipment") {
            if let Some(KeywordArgs::Equip { cost }) = self.keywords.get_args(Keyword::Equip) {
                use crate::core::{ActivatedAbility, Cost, Effect};
                let ability_cost = Cost::Mana(*cost);
                let effects = vec![Effect::AttachEquipment {
                    source_equipment: self.id,
                    target_creature: CardId::new(0),
                }];
                let description = format!("Equip {}", cost);
                self.activated_abilities
                    .push(ActivatedAbility::new_sorcery_speed(ability_cost, effects, description));
            }
        }

        self.definition
            .cache
            .update_from_abilities_with_name(&self.activated_abilities, self.name.as_str());

        self.attached_to = None;
        self.control_from_aura = None;
        self.control_grant = None;
        self.chosen_color = None;
        self.chosen_player = None;
        self.chosen_mode = None;
        self.chosen_name = None;
        self.loyalty_activated_this_turn = false;
        self.regeneration_shields = 0;
        self.damage_prevention = 0;
        self.x_paid = 0;
        self.times_kicked = 0;
        self.bargain_paid = false;
        self.kicker_paid = false;
        self.offspring_paid = false;
        self.mode_cost_paid = 0;
        self.exile_if_would_die_this_turn = false;
        self.exile_if_would_go_to_graveyard_this_turn = false;
        self.prevent_all_combat_damage_this_turn = false;
        self.dealt_damage_to_opponent_this_turn = false;
        self.attacked_this_turn = false;
        self.exhausted_abilities = SmallVec::new();
        self.cast_as_adventure = false;
    }

    pub fn is_type(&self, card_type: &CardType) -> bool {
        self.types.contains(card_type)
    }

    /// Refresh the type cache after modifying the types vector
    ///
    /// Call this after adding/removing types via `types.push()` or `types = ...`
    /// to update the cached is_land/is_creature/is_artifact flags.
    ///
    /// Note: The card loader (CardDefinition::instantiate) calls this automatically.
    /// Only manual Card creation (e.g., in tests) needs to call this explicitly.
    #[inline]
    pub fn refresh_type_cache(&mut self) {
        self.definition.cache.update_from_types(&self.types);
    }

    /// Add a type to this card and update the cache
    ///
    /// Prefer this over `types.push()` to automatically maintain cache consistency.
    /// For Land types, also updates subtype cache based on card name.
    #[inline]
    pub fn add_type(&mut self, card_type: CardType) {
        self.types.push(card_type);
        // Update cache inline for commonly checked types
        match card_type {
            CardType::Land => {
                self.definition.cache.is_land = true;
                // Also update land subtype cache based on card name
                // This handles test cards that use add_type() without explicit subtypes
                self.definition
                    .cache
                    .update_from_subtypes(&self.subtypes, self.name.as_str());
            }
            CardType::Creature => self.definition.cache.is_creature = true,
            CardType::Artifact => self.definition.cache.is_artifact = true,
            CardType::Instant => self.definition.cache.is_instant = true,
            CardType::Sorcery => self.definition.cache.is_sorcery = true,
            CardType::Enchantment => self.definition.cache.is_enchantment = true,
            CardType::Planeswalker => {} // No cache flag for Planeswalker yet
        }
    }

    /// Set the types of this card and update the cache
    ///
    /// Prefer this over `types = SmallVec::...` to automatically maintain cache consistency.
    #[inline]
    pub fn set_types(&mut self, new_types: SmallVec<[CardType; 2]>) {
        self.types = new_types;
        self.definition.cache.update_from_types(&self.types);
    }

    /// Set the subtypes of this card and update the cache
    ///
    /// Prefer this over `subtypes = SmallVec::...` to automatically maintain cache consistency.
    /// Note: Call set_types() before set_subtypes() since is_aura/is_equipment depend on type flags.
    #[inline]
    pub fn set_subtypes(&mut self, new_subtypes: SmallVec<[Subtype; 3]>) {
        self.subtypes = new_subtypes;
        self.definition
            .cache
            .update_from_subtypes(&self.subtypes, self.name.as_str());
    }

    /// The expansion this card was originally printed in (its earliest
    /// printing), or `None` if unknown. Backs set-origin valid predicates
    /// (`setARN`, ...). See [`crate::loader::CardDefinition::origin_set`].
    #[inline]
    pub fn origin_set(&self) -> Option<&crate::core::SetCode> {
        self.definition.origin_set.as_ref()
    }

    /// True if this card was originally printed in the given set.
    /// Case-insensitive (both sides are normalized `SetCode`s).
    #[inline]
    pub fn is_from_set(&self, set: &crate::core::SetCode) -> bool {
        self.definition.origin_set.as_ref() == Some(set)
    }

    /// Check if this card is a creature (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_creature(&self) -> bool {
        self.definition.cache.is_creature
    }

    /// Check if this card is a land (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_land(&self) -> bool {
        self.definition.cache.is_land
    }

    /// Check if this card is an instant (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_instant(&self) -> bool {
        self.definition.cache.is_instant
    }

    /// Check if this card is a sorcery (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_sorcery(&self) -> bool {
        self.definition.cache.is_sorcery
    }

    /// Check if this card is an artifact (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_artifact(&self) -> bool {
        self.definition.cache.is_artifact
    }

    /// Check if this card is an enchantment (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_enchantment(&self) -> bool {
        self.definition.cache.is_enchantment
    }

    pub fn is_planeswalker(&self) -> bool {
        self.is_type(&CardType::Planeswalker)
    }

    /// Check if this card is an Aura (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_aura(&self) -> bool {
        self.definition.cache.is_aura
    }

    /// Check if this card is Equipment (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_equipment(&self) -> bool {
        self.definition.cache.is_equipment
    }

    /// Check if this Equipment/Aura is currently attached to something
    pub fn is_attached(&self) -> bool {
        self.attached_to.is_some()
    }

    /// Get the card this Equipment/Aura is attached to
    pub fn get_attached_to(&self) -> Option<CardId> {
        self.attached_to
    }

    pub fn has_keyword(&self, keyword: Keyword) -> bool {
        self.keywords.contains(keyword)
    }

    // === Reveal tracking methods (per NETWORK_ARCHITECTURE.md) ===

    /// Check if this card has been revealed to a specific player
    ///
    /// Used for deduplication: skip logging RevealCard if already revealed.
    #[inline]
    pub fn is_revealed_to(&self, player_id: PlayerId) -> bool {
        let bit = 1u8 << (player_id.as_u32() as u8);
        (self.revealed_to_mask & bit) != 0
    }

    /// Check if this card has been revealed to all players (both in 2-player game)
    #[inline]
    pub fn is_revealed_to_all(&self) -> bool {
        // For 2-player games, both players means bits 0 and 1 are set (0b11 = 3)
        self.revealed_to_mask >= 0b11
    }

    /// Mark this card as revealed to a specific player
    #[inline]
    pub fn mark_revealed_to(&mut self, player_id: PlayerId) {
        let bit = 1u8 << (player_id.as_u32() as u8);
        self.revealed_to_mask |= bit;
    }

    /// Mark this card as revealed to all players
    #[inline]
    pub fn mark_revealed_to_all(&mut self) {
        // For 2-player games, set both bits
        self.revealed_to_mask = 0b11;
    }

    /// Clear a player from the revealed mask (for undo)
    #[inline]
    pub fn clear_revealed_to(&mut self, player_id: PlayerId) {
        let bit = 1u8 << (player_id.as_u32() as u8);
        self.revealed_to_mask &= !bit;
    }

    /// Clear all reveal tracking (for undo of "reveal to all")
    #[inline]
    pub fn clear_revealed_to_all(&mut self) {
        self.revealed_to_mask = 0;
    }

    pub fn has_flying(&self) -> bool {
        self.keywords.contains(Keyword::Flying)
    }

    pub fn has_reach(&self) -> bool {
        self.keywords.contains(Keyword::Reach)
    }

    pub fn has_first_strike(&self) -> bool {
        self.keywords.contains(Keyword::FirstStrike)
    }

    pub fn has_double_strike(&self) -> bool {
        self.keywords.contains(Keyword::DoubleStrike)
    }

    /// Returns true if this creature deals damage in the normal damage step
    /// (i.e., has double strike OR doesn't have first strike)
    pub fn has_normal_strike(&self) -> bool {
        self.has_double_strike() || !self.has_first_strike()
    }

    pub fn has_trample(&self) -> bool {
        self.keywords.contains(Keyword::Trample)
    }

    pub fn has_lifelink(&self) -> bool {
        self.keywords.contains(Keyword::Lifelink)
    }

    pub fn has_deathtouch(&self) -> bool {
        self.keywords.contains(Keyword::Deathtouch)
    }

    pub fn has_menace(&self) -> bool {
        self.keywords.contains(Keyword::Menace)
    }

    pub fn has_hexproof(&self) -> bool {
        self.keywords.contains(Keyword::Hexproof)
    }

    pub fn has_indestructible(&self) -> bool {
        self.keywords.contains(Keyword::Indestructible)
    }

    pub fn has_defender(&self) -> bool {
        self.keywords.contains(Keyword::Defender)
    }

    pub fn has_shroud(&self) -> bool {
        self.keywords.contains(Keyword::Shroud)
    }

    pub fn has_fear(&self) -> bool {
        self.keywords.contains(Keyword::Fear)
    }

    pub fn has_intimidate(&self) -> bool {
        self.keywords.contains(Keyword::Intimidate)
    }

    pub fn has_shadow(&self) -> bool {
        self.keywords.contains(Keyword::Shadow)
    }

    pub fn has_skulk(&self) -> bool {
        self.keywords.contains(Keyword::Skulk)
    }

    pub fn has_horsemanship(&self) -> bool {
        self.keywords.contains(Keyword::Horsemanship)
    }

    /// Check if this card has protection from a specific color
    /// Used for blocking restrictions - a creature with protection from red
    /// can't be blocked by red creatures
    pub fn has_protection_from(&self, color: Color) -> bool {
        match color {
            Color::Red => self.keywords.contains(Keyword::ProtectionFromRed),
            Color::Blue => self.keywords.contains(Keyword::ProtectionFromBlue),
            Color::Black => self.keywords.contains(Keyword::ProtectionFromBlack),
            Color::White => self.keywords.contains(Keyword::ProtectionFromWhite),
            Color::Green => self.keywords.contains(Keyword::ProtectionFromGreen),
            Color::Colorless => false, // No protection from colorless keyword
        }
    }

    /// Check if this card is a specific color
    /// Used for blocking restrictions (Fear/Intimidate check if blocker is black/artifact)
    pub fn is_color(&self, color: Color) -> bool {
        // Check the colors field which stores the card's colors
        self.colors.contains(&color)
    }

    /// Check if this card is colorless (artifact creatures, Eldrazi, etc.)
    pub fn is_colorless(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn tap(&mut self) {
        self.tapped = true;
    }

    pub fn untap(&mut self) {
        self.tapped = false;
    }

    pub fn add_counter(&mut self, counter_type: CounterType, amount: u8) {
        if amount == 0 {
            return;
        }

        // Add the counter
        if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &counter_type) {
            *count = count.saturating_add(amount);
        } else {
            self.counters.push((counter_type, amount));
        }

        // Apply counter annihilation: +1/+1 and -1/-1 counters cancel
        let p1p1_count = self.get_counter(CounterType::P1P1);
        let m1m1_count = self.get_counter(CounterType::M1M1);

        if p1p1_count > 0 && m1m1_count > 0 {
            let to_remove = p1p1_count.min(m1m1_count);

            // Remove from +1/+1 counters
            if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &CounterType::P1P1) {
                *count -= to_remove;
                if *count == 0 {
                    self.counters.retain(|(t, _)| t != &CounterType::P1P1);
                }
            }

            // Remove from -1/-1 counters
            if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &CounterType::M1M1) {
                *count -= to_remove;
                if *count == 0 {
                    self.counters.retain(|(t, _)| t != &CounterType::M1M1);
                }
            }
        }
    }

    pub fn remove_counter(&mut self, counter_type: CounterType, amount: u8) -> u8 {
        if amount == 0 {
            return 0;
        }

        if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &counter_type) {
            let removed = (*count).min(amount);
            *count -= removed;
            if *count == 0 {
                self.counters.retain(|(t, _)| t != &counter_type);
            }
            removed
        } else {
            0
        }
    }

    pub fn get_counter(&self, counter_type: CounterType) -> u8 {
        self.counters
            .iter()
            .find(|(t, _)| t == &counter_type)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    /// Check if the card has any counters on it
    ///
    /// Returns true if the card has at least one counter of any type.
    /// Used for targeting restrictions like "creature with no counters".
    pub fn has_counters(&self) -> bool {
        self.counters.iter().any(|(_, count)| *count > 0)
    }

    /// Set the card text
    ///
    /// NOTE: This does NOT update mana production in the cache. Mana production
    /// is derived from parsed abilities, not text. Call `cache.update_from_abilities()`
    /// after adding mana abilities if needed.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// Get base/printed power (without counters or bonuses)
    /// Most code should use current_power() instead
    pub fn base_power(&self) -> Option<i8> {
        self.base_power
    }

    /// Set base/printed power
    pub fn set_base_power(&mut self, power: Option<i8>) {
        self.base_power = power;
    }

    /// Get base/printed toughness (without counters or bonuses)
    /// Most code should use current_toughness() instead
    pub fn base_toughness(&self) -> Option<i8> {
        self.base_toughness
    }

    /// Set base/printed toughness
    pub fn set_base_toughness(&mut self, toughness: Option<i8>) {
        self.base_toughness = toughness;
    }

    /// Get current power (including counters and temporary bonuses)
    /// This is the canonical method for reading creature power
    pub fn current_power(&self) -> i8 {
        // Use temp_base_power if set (from Animate effects), otherwise use printed power
        let base = self.temp_base_power.or(self.base_power).unwrap_or(0);
        let plus_counters = self.get_counter(CounterType::P1P1) as i8;
        let minus_counters = self.get_counter(CounterType::M1M1) as i8;
        let bonus = self.power_bonus as i8;
        base + plus_counters - minus_counters + bonus
    }

    /// Get current toughness (including counters and temporary bonuses)
    /// This is the canonical method for reading creature toughness
    pub fn current_toughness(&self) -> i8 {
        // Use temp_base_toughness if set (from Animate effects), otherwise use printed toughness
        let base = self.temp_base_toughness.or(self.base_toughness).unwrap_or(0);
        let plus_counters = self.get_counter(CounterType::P1P1) as i8;
        let minus_counters = self.get_counter(CounterType::M1M1) as i8;
        let bonus = self.toughness_bonus as i8;
        base + plus_counters - minus_counters + bonus
    }

    /// Get temporary base power override if set
    pub fn temp_base_power(&self) -> Option<i8> {
        self.temp_base_power
    }

    /// Get temporary base toughness override if set
    pub fn temp_base_toughness(&self) -> Option<i8> {
        self.temp_base_toughness
    }

    /// Set temporary base power override (until end of turn)
    /// Used by Animate effects like Flexible Waterbender
    pub fn set_temp_base_power(&mut self, power: i8) {
        self.temp_base_power = Some(power);
    }

    /// Set temporary base toughness override (until end of turn)
    /// Used by Animate effects like Flexible Waterbender
    pub fn set_temp_base_toughness(&mut self, toughness: i8) {
        self.temp_base_toughness = Some(toughness);
    }

    /// Clear temporary effects (called at end of turn cleanup)
    /// Resets temp base P/T overrides from Animate effects
    pub fn clear_temp_base_stats(&mut self) {
        self.temp_base_power = None;
        self.temp_base_toughness = None;
    }

    /// Restore the temp base P/T overrides to specific previous values.
    /// Used by `GameAction::SetTempBaseStats::undo` (mtg-614 hole (c)) to revert
    /// a `set_temp_base_*` / `clear_temp_base_stats` mutation exactly.
    pub fn restore_temp_base_stats(&mut self, power: Option<i8>, toughness: Option<i8>) {
        self.temp_base_power = power;
        self.temp_base_toughness = toughness;
    }

    /// Record that `keyword` was granted to this card by an until-end-of-turn
    /// pump effect, AND insert it into the live keyword set. Routing all
    /// "gains <keyword> until end of turn" grants through here lets the
    /// forward EOT cleanup and the rewind transient sweep remove exactly the
    /// granted-until-EOT keywords by game position (mtg-610).
    ///
    /// CRITICAL: only the keywords this grant *newly* adds to the live set are
    /// tracked in `temp_keywords_until_eot`. If the card ALREADY had the keyword
    /// (a printed `K:Haste` like Screaming Nemesis, or another permanent source),
    /// it is NOT tracked as temporary — so the forward cleanup / rewind sweep
    /// (`clear_temp_keywords_until_eot`) and the per-action `PumpCreature` undo
    /// never strip a printed/other-source keyword (mtg-731: a Rockface Village
    /// "+1/+0 and gains haste" on a printed-haste creature was stripping the
    /// printed Haste at EOT cleanup AND drifting the turn-start keyword set
    /// across rewinds). Mirrors the `AnimateTypeline` granted-keyword tracking.
    ///
    /// Returns `true` iff the keyword was newly added to the live set, so the
    /// caller can record exactly the reversible subset in the undo log.
    pub fn grant_keyword_until_eot(&mut self, keyword: Keyword) -> bool {
        if self.keywords.contains(keyword) {
            // Already present (printed or another source) — granting it again
            // until EOT must not make it removable.
            return false;
        }
        self.keywords.insert(keyword);
        self.temp_keywords_until_eot.insert(keyword);
        true
    }

    /// Grant a complex (parameterized) keyword (e.g., `Landwalk:Forest`) until end of turn.
    ///
    /// Uses `insert_complex` so both the keyword bit AND its parameter (land type,
    /// protection color, etc.) are stored. The tracking set records the plain
    /// `Keyword` bit for cleanup (removing the bit via `KeywordSet::remove` also
    /// removes the associated `KeywordArgs` entry).
    ///
    /// Returns `true` if the keyword was newly added, `false` if already present.
    pub fn grant_keyword_args_until_eot(&mut self, args: &crate::core::KeywordArgs) -> bool {
        let keyword = args.keyword();
        if self.keywords.contains(keyword) {
            return false;
        }
        self.keywords.insert_complex(args.clone());
        self.temp_keywords_until_eot.insert(keyword);
        true
    }

    /// Remove all until-end-of-turn granted keywords from the live keyword set
    /// and clear the tracking set. Called from the forward end-of-turn cleanup
    /// (`GameState::cleanup_temporary_effects`) and the rewind per-turn-transient
    /// sweep (`UndoLog::rewind_to_turn_start`), so the until-EOT keyword set is
    /// a deterministic function of game position on both paths (mtg-610).
    pub fn clear_temp_keywords_until_eot(&mut self) {
        for keyword in self.temp_keywords_until_eot.iter() {
            self.keywords.remove(keyword);
        }
        self.temp_keywords_until_eot.clear();
    }

    /// Snapshot exactly the copiable characteristics that
    /// [`crate::game::GameState::apply_clone`] overwrites (CR 707.2), so a Clone
    /// (Copy Artifact, Clone, Vesuvan Doppelganger, ...) can be reverted exactly
    /// by the undo log. Without this the in-resolution clone-copy transformation
    /// was a one-way mutation: a rewind+replay left the card stuck as the copied
    /// permanent (mtg-559/mtg-610: robots42 Copy Artifact -> Mishra's Factory
    /// drift across rewinds). Captured BEFORE the overwrite; restored by
    /// [`Card::restore_copiable_state`].
    pub fn capture_copiable_state(&self) -> CardCopiableState {
        CardCopiableState {
            name: self.name.clone(),
            mana_cost: self.mana_cost,
            types: self.types.clone(),
            subtypes: self.subtypes.clone(),
            colors: self.colors.clone(),
            base_power: self.base_power,
            base_toughness: self.base_toughness,
            text: self.text.clone(),
            is_legendary: self.is_legendary,
            keywords: self.keywords.clone(),
            activated_abilities: self.activated_abilities.clone(),
            static_abilities: self.static_abilities.clone(),
            triggers: self.triggers.clone(),
            svars: self.svars.clone(),
            definition: self.definition.clone(),
        }
    }

    /// Restore the copiable characteristics captured by
    /// [`Card::capture_copiable_state`], reversing an [`crate::game::GameState::apply_clone`]
    /// exactly (mtg-559/mtg-610).
    pub fn restore_copiable_state(&mut self, state: CardCopiableState) {
        self.name = state.name;
        self.mana_cost = state.mana_cost;
        self.types = state.types;
        self.subtypes = state.subtypes;
        self.colors = state.colors;
        self.base_power = state.base_power;
        self.base_toughness = state.base_toughness;
        self.text = state.text;
        self.is_legendary = state.is_legendary;
        self.keywords = state.keywords;
        self.activated_abilities = state.activated_abilities;
        self.static_abilities = state.static_abilities;
        self.triggers = state.triggers;
        self.svars = state.svars;
        self.definition = state.definition;
    }

    /// Revert an until-end-of-turn `AB$ Animate` typeline change (Mishra's
    /// Factory and friends becoming land-only again), draining
    /// `temp_animate_types` / `temp_animate_subtypes` / `temp_removed_subtypes`
    /// and refreshing the definition cache. Returns `true` if the card's
    /// typeline was actually touched (so the caller can invalidate
    /// mana-source caches), and the post-revert `is_mana_source` flag.
    ///
    /// Shared by the end-of-turn cleanup step (`GameState::cleanup_temporary_effects`)
    /// and `UndoLog::rewind_to_turn_start`: both must return an animated
    /// permanent to its printed typeline at a turn boundary, since animate is
    /// "until end of turn" (it never spans a turn boundary) and is not
    /// undo-logged (mtg-610). Centralizing the revert keeps the two paths
    /// byte-identical (DRY).
    pub fn revert_temp_animation(&mut self) -> (bool, bool) {
        let touched_types = !self.temp_animate_types.is_empty();
        let touched_subtypes = !self.temp_animate_subtypes.is_empty() || !self.temp_removed_subtypes.is_empty();
        if touched_types {
            let removed: SmallVec<[CardType; 2]> = self.temp_animate_types.drain(..).collect();
            self.types.retain(|t| !removed.contains(t));
        }
        if touched_subtypes {
            let added: SmallVec<[Subtype; 2]> = self.temp_animate_subtypes.drain(..).collect();
            self.subtypes.retain(|s| !added.contains(s));
            // Restore subtypes that RemoveCreatureTypes$ True stripped.
            let restored: SmallVec<[Subtype; 2]> = self.temp_removed_subtypes.drain(..).collect();
            self.subtypes.extend(restored);
        }
        if touched_types || touched_subtypes {
            let types = self.types.clone();
            let subtypes = self.subtypes.clone();
            let name = self.name.clone();
            self.definition.cache.update_from_types(&types);
            self.definition.cache.update_from_subtypes(&subtypes, name.as_str());
            (true, self.definition.cache.is_mana_source)
        } else {
            (false, false)
        }
    }
}

impl GameEntity<Card> for Card {
    fn id(&self) -> CardId {
        self.id
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_card_creation() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let card = Card::new(id, "Lightning Bolt", owner);

        assert_eq!(card.id, id);
        assert_eq!(card.name.as_str(), "Lightning Bolt");
        assert_eq!(card.owner, owner);
        assert_eq!(card.controller, owner);
        assert!(!card.tapped);
    }

    #[test]
    fn grant_keyword_until_eot_never_strips_printed_keyword() {
        // Regression (mtg-731): a Rockface Village "+1/+0 and gains haste
        // until EOT" on a creature with PRINTED Haste (Screaming Nemesis) was
        // stripping the printed Haste at the forward EOT cleanup AND drifting
        // the turn-start keyword set across rewinds, because the grant tracked
        // the already-present keyword as temporary.
        let owner = PlayerId::new(100);
        let mut card = Card::new(CardId::new(1), "Screaming Nemesis", owner);
        card.keywords.insert(Keyword::Haste); // printed K:Haste

        // Granting an ALREADY-present keyword must NOT track it as temporary and
        // must report it was not newly added.
        assert!(!card.grant_keyword_until_eot(Keyword::Haste));
        assert!(card.has_keyword(Keyword::Haste));
        assert!(!card.temp_keywords_until_eot.contains(Keyword::Haste));

        // Cleanup / rewind sweep must leave the printed Haste intact.
        card.clear_temp_keywords_until_eot();
        assert!(
            card.has_keyword(Keyword::Haste),
            "printed Haste must survive the until-EOT cleanup sweep"
        );

        // Granting a NEW keyword IS tracked and IS removed by the sweep.
        assert!(card.grant_keyword_until_eot(Keyword::Trample));
        assert!(card.has_keyword(Keyword::Trample));
        assert!(card.temp_keywords_until_eot.contains(Keyword::Trample));
        card.clear_temp_keywords_until_eot();
        assert!(!card.has_keyword(Keyword::Trample));
        assert!(card.has_keyword(Keyword::Haste)); // printed one still intact
    }

    #[test]
    fn test_card_counters() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        card.set_base_power(Some(2));
        card.set_base_toughness(Some(2));

        assert_eq!(card.current_power(), 2);
        assert_eq!(card.current_toughness(), 2);

        card.add_counter(CounterType::P1P1, 2);
        assert_eq!(card.current_power(), 4);
        assert_eq!(card.current_toughness(), 4);

        card.add_counter(CounterType::M1M1, 1);
        assert_eq!(card.current_power(), 3);
        assert_eq!(card.current_toughness(), 3);
    }

    #[test]
    fn test_counter_annihilation() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        card.set_base_power(Some(2));
        card.set_base_toughness(Some(2));

        // Add 3 +1/+1 counters
        card.add_counter(CounterType::P1P1, 3);
        assert_eq!(card.get_counter(CounterType::P1P1), 3);
        assert_eq!(card.get_counter(CounterType::M1M1), 0);
        assert_eq!(card.current_power(), 5);
        assert_eq!(card.current_toughness(), 5);

        // Add 2 -1/-1 counters - should annihilate with +1/+1
        card.add_counter(CounterType::M1M1, 2);
        assert_eq!(card.get_counter(CounterType::P1P1), 1); // 3 - 2 = 1
        assert_eq!(card.get_counter(CounterType::M1M1), 0); // 2 - 2 = 0
        assert_eq!(card.current_power(), 3); // 2 base + 1 counter
        assert_eq!(card.current_toughness(), 3);

        // Add 5 -1/-1 counters
        card.add_counter(CounterType::M1M1, 5);
        assert_eq!(card.get_counter(CounterType::P1P1), 0); // 1 - 1 = 0
        assert_eq!(card.get_counter(CounterType::M1M1), 4); // 5 - 1 = 4
        assert_eq!(card.current_power(), -2); // 2 base - 4 counters
        assert_eq!(card.current_toughness(), -2);
    }

    #[test]
    fn test_remove_counter() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        // Add some counters
        card.add_counter(CounterType::P1P1, 5);
        assert_eq!(card.get_counter(CounterType::P1P1), 5);

        // Remove 2 counters
        let removed = card.remove_counter(CounterType::P1P1, 2);
        assert_eq!(removed, 2);
        assert_eq!(card.get_counter(CounterType::P1P1), 3);

        // Try to remove more than exists
        let removed = card.remove_counter(CounterType::P1P1, 10);
        assert_eq!(removed, 3); // Only 3 were available
        assert_eq!(card.get_counter(CounterType::P1P1), 0);

        // Counter type should be cleaned up when it reaches 0
        assert!(!card.counters.iter().any(|(t, _)| t == &CounterType::P1P1));

        // Try to remove from non-existent counter type
        let removed = card.remove_counter(CounterType::M1M1, 5);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_exact_annihilation() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        // Add 3 +1/+1 counters
        card.add_counter(CounterType::P1P1, 3);
        assert_eq!(card.get_counter(CounterType::P1P1), 3);

        // Add exactly 3 -1/-1 counters - should cancel completely
        card.add_counter(CounterType::M1M1, 3);
        assert_eq!(card.get_counter(CounterType::P1P1), 0);
        assert_eq!(card.get_counter(CounterType::M1M1), 0);

        // Both counter types should be cleaned up
        assert!(card.counters.is_empty());
    }

    #[test]
    fn test_other_counters_not_affected() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Permanent", owner);

        // Add charge counters
        card.add_counter(CounterType::Charge, 5);
        assert_eq!(card.get_counter(CounterType::Charge), 5);

        // Add +1/+1 and -1/-1 counters
        card.add_counter(CounterType::P1P1, 2);
        card.add_counter(CounterType::M1M1, 1);

        // Charge counters should not be affected by annihilation
        assert_eq!(card.get_counter(CounterType::Charge), 5);
        assert_eq!(card.get_counter(CounterType::P1P1), 1); // 2 - 1 = 1
        assert_eq!(card.get_counter(CounterType::M1M1), 0);
    }

    #[test]
    fn test_cardcache_size() {
        // Print size for debugging allocation issues
        eprintln!("sizeof(CardCache) = {} bytes", std::mem::size_of::<CardCache>());
        eprintln!("sizeof(Card) = {} bytes", std::mem::size_of::<Card>());
    }

    #[test]
    fn test_has_counters() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        // Fresh card has no counters
        assert!(!card.has_counters(), "Fresh card should have no counters");

        // Add a +1/+1 counter
        card.add_counter(CounterType::P1P1, 1);
        assert!(card.has_counters(), "Card with +1/+1 counter should have counters");

        // Remove the counter
        card.remove_counter(CounterType::P1P1, 1);
        assert!(
            !card.has_counters(),
            "Card with removed counter should have no counters"
        );

        // Add different counter types
        card.add_counter(CounterType::Charge, 3);
        assert!(card.has_counters(), "Card with charge counters should have counters");

        card.add_counter(CounterType::P1P1, 2);
        assert!(
            card.has_counters(),
            "Card with multiple counter types should have counters"
        );
    }
}
