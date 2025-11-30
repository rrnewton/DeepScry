//! Card types and definitions

use crate::core::{
    CardId, CardName, Color, CounterType, Effect, GameEntity, Keyword, KeywordSet, ManaCost, ManaProduction, PlayerId,
    Subtype, Trigger,
};
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

    /// Spell targets player(s) (e.g., "Target player draws three cards")
    pub spell_targets_player: bool,

    /// Spell can target "any target" (creature or player)
    /// Example: "Lightning Bolt deals 3 damage to any target"
    pub spell_targets_any: bool,

    /// Precomputed: Static value of this land for AI evaluation
    /// (from evaluate_land function in game_state_evaluator.rs)
    /// Only meaningful for lands, 0 for non-lands
    pub land_evaluation_value: i32,
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
            // "target player" for draw/life effects
            spell_targets_player: text_lower.contains("target player") || text_lower.contains("target opponent"),
            // "any target" means creatures or players (e.g., Lightning Bolt)
            spell_targets_any: text_lower.contains("any target"),

            land_evaluation_value: 0,
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

    /// Update cache based on abilities, with fallback to land name detection
    ///
    /// This is the primary entry point for card loading. It:
    /// 1. Tries to derive mana production from parsed abilities
    /// 2. Falls back to basic land name detection for test cards
    pub fn update_from_abilities_with_name(&mut self, abilities: &[crate::core::ActivatedAbility], name: &str) {
        self.mana_production = Self::derive_mana_production_from_abilities(abilities);

        // Fallback for test cards that create basic lands without explicit abilities
        if !self.mana_production.produces_mana() {
            self.mana_production = Self::derive_mana_production_from_name(name);
        }

        self.is_mana_source = self.mana_production.produces_mana();
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
    fn derive_mana_production_from_abilities(abilities: &[crate::core::ActivatedAbility]) -> ManaProduction {
        use crate::core::{Effect, ManaColor, ManaProductionKind};
        use crate::game::mana_colors::ManaColors;

        let mut colors = ManaColors::new();
        let mut produces_colorless = false;
        let mut produces_any = false;

        for ability in abilities {
            // Only consider mana abilities
            if !ability.is_mana_ability {
                continue;
            }

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
                    // TODO(mtg-s3ri5): Track "any color" explicitly in Effect::AddMana
                    // For now, if all 5 colors are present, treat as any color
                    if mana.white > 0 && mana.blue > 0 && mana.black > 0 && mana.red > 0 && mana.green > 0 {
                        produces_any = true;
                    }
                }
            }
        }

        // Build ManaProduction from accumulated colors
        if produces_any {
            return ManaProduction::free(ManaProductionKind::AnyColor);
        }

        match colors.len() {
            0 if produces_colorless => ManaProduction::free(ManaProductionKind::Colorless),
            0 => ManaProduction::default(), // No mana production
            1 => {
                // Single color - use Fixed variant
                let color = colors.iter().next().unwrap();
                ManaProduction::free(ManaProductionKind::Fixed(color))
            }
            _ => {
                // Multiple colors - use Choice variant (OR logic)
                ManaProduction::free(ManaProductionKind::Choice(colors))
            }
        }
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

    /// Mana cost
    pub mana_cost: ManaCost,

    /// Card types (a card can be multiple types)
    pub types: SmallVec<[CardType; 2]>,

    /// Card subtypes (e.g., "Goblin", "Warrior")
    pub subtypes: SmallVec<[Subtype; 3]>,

    /// Colors of the card
    pub colors: SmallVec<[Color; 2]>,

    /// Power (for creatures)
    /// PRIVATE: Use current_power() to get effective power (includes counters & bonuses)
    /// Only accessed directly inside current_power() implementation
    power: Option<i8>,

    /// Toughness (for creatures)
    /// PRIVATE: Use current_toughness() to get effective toughness (includes counters & bonuses)
    /// Only accessed directly inside current_toughness() implementation
    toughness: Option<i8>,

    /// Temporary power bonus (until end of turn)
    pub power_bonus: i32,

    /// Temporary toughness bonus (until end of turn)
    pub toughness_bonus: i32,

    /// Damage marked on this permanent (cleared at end of turn per CR 704.5g)
    /// Only meaningful for creatures on the battlefield
    pub damage: i32,

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

    /// Cache for expensive string operations (computed at load time)
    /// Avoids repeated to_lowercase() and contains() allocations during gameplay
    pub cache: CardCache,
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

        Card {
            id,
            cache,
            name,
            mana_cost: ManaCost::new(),
            types: SmallVec::new(),
            subtypes: SmallVec::new(),
            colors: SmallVec::new(),
            power: None,
            toughness: None,
            power_bonus: 0,
            toughness_bonus: 0,
            damage: 0,
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
        }
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
        self.cache.update_from_types(&self.types);
    }

    /// Add a type to this card and update the cache
    ///
    /// Prefer this over `types.push()` to automatically maintain cache consistency.
    #[inline]
    pub fn add_type(&mut self, card_type: CardType) {
        self.types.push(card_type);
        // Update cache inline for commonly checked types
        match card_type {
            CardType::Land => self.cache.is_land = true,
            CardType::Creature => self.cache.is_creature = true,
            CardType::Artifact => self.cache.is_artifact = true,
            _ => {}
        }
    }

    /// Set the types of this card and update the cache
    ///
    /// Prefer this over `types = SmallVec::...` to automatically maintain cache consistency.
    #[inline]
    pub fn set_types(&mut self, new_types: SmallVec<[CardType; 2]>) {
        self.types = new_types;
        self.cache.update_from_types(&self.types);
    }

    /// Check if this card is a creature (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_creature(&self) -> bool {
        self.cache.is_creature
    }

    /// Check if this card is a land (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_land(&self) -> bool {
        self.cache.is_land
    }

    pub fn is_instant(&self) -> bool {
        self.is_type(&CardType::Instant)
    }

    /// Check if this card is an artifact (uses cached value for O(1) lookup)
    #[inline]
    pub fn is_artifact(&self) -> bool {
        self.cache.is_artifact
    }

    pub fn is_enchantment(&self) -> bool {
        self.is_type(&CardType::Enchantment)
    }

    pub fn is_planeswalker(&self) -> bool {
        self.is_type(&CardType::Planeswalker)
    }

    pub fn is_aura(&self) -> bool {
        self.is_enchantment() && self.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("aura"))
    }

    pub fn is_equipment(&self) -> bool {
        self.is_artifact()
            && self
                .subtypes
                .iter()
                .any(|s| s.as_str().eq_ignore_ascii_case("equipment"))
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

    /// Set the card text
    ///
    /// NOTE: This does NOT update mana production in the cache. Mana production
    /// is derived from parsed abilities, not text. Call `cache.update_from_abilities()`
    /// after adding mana abilities if needed.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// Get base power (without counters or bonuses)
    /// Most code should use current_power() instead
    pub fn base_power(&self) -> Option<i8> {
        self.power
    }

    /// Set base power
    pub fn set_power(&mut self, power: Option<i8>) {
        self.power = power;
    }

    /// Get base toughness (without counters or bonuses)
    /// Most code should use current_toughness() instead
    pub fn base_toughness(&self) -> Option<i8> {
        self.toughness
    }

    /// Set base toughness
    pub fn set_toughness(&mut self, toughness: Option<i8>) {
        self.toughness = toughness;
    }

    /// Get current power (including counters and temporary bonuses)
    /// This is the canonical method for reading creature power
    pub fn current_power(&self) -> i8 {
        let base = self.power.unwrap_or(0);
        let plus_counters = self.get_counter(CounterType::P1P1) as i8;
        let minus_counters = self.get_counter(CounterType::M1M1) as i8;
        let bonus = self.power_bonus as i8;
        base + plus_counters - minus_counters + bonus
    }

    /// Get current toughness (including counters and temporary bonuses)
    /// This is the canonical method for reading creature toughness
    pub fn current_toughness(&self) -> i8 {
        let base = self.toughness.unwrap_or(0);
        let plus_counters = self.get_counter(CounterType::P1P1) as i8;
        let minus_counters = self.get_counter(CounterType::M1M1) as i8;
        let bonus = self.toughness_bonus as i8;
        base + plus_counters - minus_counters + bonus
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
    fn test_card_counters() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        card.set_power(Some(2));
        card.set_toughness(Some(2));

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

        card.set_power(Some(2));
        card.set_toughness(Some(2));

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
}
