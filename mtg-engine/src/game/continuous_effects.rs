//! Continuous Effects System
//!
//! Implements the MTG Comprehensive Rules 613 layer system for calculating
//! the final characteristics of game objects.
//!
//! ## CR 613: Interaction of Continuous Effects
//!
//! > 613.1. The values of an object's characteristics are determined by starting
//! > with the actual object. [...] Then all applicable continuous effects are
//! > applied in a series of layers in the following order:
//!
//! This module focuses on **Layer 7: Power and Toughness Changes** (CR 613.4)
//! which has four sublayers:
//!
//! - **Layer 7a (CHARACTERISTIC)**: Characteristic-defining abilities (CR 613.4a)
//!   - Example: Tarmogoyf's "* / *" based on card types in graveyards
//!
//! - **Layer 7b (SETPT)**: Effects that SET P/T to specific values (CR 613.4b)
//!   - Example: "Target creature becomes 0/1 until end of turn"
//!   - Example: Lignify sets enchanted creature to 0/4
//!
//! - **Layer 7c (MODIFYPT)**: Effects and counters that MODIFY P/T (CR 613.4c)
//!   - Example: Equipment bonuses (+2/+2)
//!   - Example: Anthem effects ("Creatures you control get +1/+1")
//!   - Example: +1/+1 counters, -1/-1 counters
//!   - **Note**: CR 613.4c explicitly includes both effects AND counters in this layer
//!   - **Implementation**: We separate effects and counters into distinct fields
//!     (`modifypt_effects` and `modifypt_counters`) for code clarity, matching
//!     Java Forge's `StatBreakdown` structure. Both cite CR 613.4c.
//!
//! - **Layer 7d (SWITCH)**: Effects that switch power and toughness (CR 613.4d)
//!   - Example: "Switch target creature's power and toughness"
//!
//! ## Implementation Status
//!
//! - ✅ Layer 7a (CHARACTERISTIC): Stubbed (will be needed for */* creatures)
//! - ✅ Layer 7b (SETPT): Stubbed (will be needed for effects like Lignify)
//! - ✅ Layer 7c (MODIFYPT): Implemented with Equipment and counters
//! - ✅ Layer 7d (SWITCH): Stubbed (will be needed for P/T switching effects)

use crate::core::CardId;
use crate::game::GameState;
use crate::Result;

/// Power/Toughness breakdown showing contribution from each layer.
///
/// This structure implements the calculation from CR 613.4 with an explicit
/// separation of continuous effects and counters (matching Java Forge):
/// ```text
/// Final P/T = base → Layer 7a → Layer 7b → Layer 7c (effects) → Layer 7c (counters) → Layer 7d
/// ```
///
/// ## Design Choice: Separating Effects and Counters
///
/// **CR 613.4c states**: "Effects and counters that modify power and/or toughness"
/// are applied in the same layer. However, like Java Forge's `StatBreakdown`, we
/// separate them into distinct fields (`modifypt_effects` and `modifypt_counters`)
/// because:
///
/// 1. **Code clarity**: Effects (Equipment, anthems) are conceptually different from counters
/// 2. **Debugging**: Easier to see what each source contributes to final P/T
/// 3. **Java Forge compatibility**: Matches their proven architecture exactly
///
/// Both fields cite CR 613.4c and are applied sequentially within that layer.
/// The final result is identical to applying them simultaneously.
///
/// ## CR 613.5 Example (Gray Ogre)
///
/// > Gray Ogre, a 2/2 creature, is on the battlefield. An effect puts a +1/+1
/// > counter on it (layer 7c), making it 3/3. A spell targeting it that says
/// > "Target creature gets +4/+4 until end of turn" resolves (layer 7c), making
/// > it 7/7. An enchantment that says "Creatures you control get +0/+2" enters
/// > the battlefield (layer 7c), making it 7/9. An effect that says "Target
/// > creature becomes 0/1 until end of turn" is applied to it (layer 7b),
/// > making it 5/8 (0/1, with +4/+4 from the resolved spell, +0/+2 from the
/// > enchantment, and +1/+1 from the counter).
///
/// This breakdown makes each layer's contribution visible for debugging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PTBreakdown {
    /// Base power/toughness from the card's printed characteristics.
    /// This is the starting point before any layers are applied.
    pub base: (i32, i32),

    /// Layer 7a (CR 613.4a): Characteristic-defining abilities.
    /// If Some, this REPLACES base P/T (e.g., Tarmogoyf's */*).
    /// If None, layer 7a does not apply.
    pub characteristic_value: Option<(i32, i32)>,

    /// Layer 7b (CR 613.4b): Effects that SET P/T to specific value.
    /// If Some, this REPLACES the current P/T (e.g., "becomes 0/1").
    /// If None, layer 7b does not apply.
    pub setpt_value: Option<(i32, i32)>,

    /// Layer 7c (CR 613.4c): Continuous effects that MODIFY P/T.
    ///
    /// This includes Equipment bonuses, anthem effects, Giant Growth, etc.
    /// These ADD to the current P/T.
    ///
    /// **Note**: Applied BEFORE `modifypt_counters` in our implementation,
    /// though CR 613.4c technically groups them in the same layer.
    pub modifypt_effects: (i32, i32),

    /// Layer 7c (CR 613.4c): Counters that modify P/T.
    ///
    /// This includes +1/+1 counters, -1/-1 counters, etc.
    /// These ADD to the current P/T.
    ///
    /// **Implementation Note**: While CR 613.4c groups "effects and counters"
    /// together in the same layer, we separate them into distinct fields
    /// (like Java Forge's `StatBreakdown`) for code clarity. Both cite the
    /// same CR 613.4c rule. Applied AFTER `modifypt_effects`.
    pub modifypt_counters: (i32, i32),

    /// Layer 7d (CR 613.4d): Has power/toughness been switched?
    /// If true, swap power and toughness after applying all previous layers.
    pub is_switched: bool,
}

impl PTBreakdown {
    /// Calculate final power/toughness by applying all layers in order.
    ///
    /// ## Algorithm (CR 613.4)
    ///
    /// 1. Start with base P/T from printed card
    /// 2. Apply Layer 7a (characteristic-defining) if present → REPLACES base
    /// 3. Apply Layer 7b (set P/T) if present → REPLACES current value
    /// 4. Apply Layer 7c (modify P/T) → ADDS effects and counters
    /// 5. Apply Layer 7d (switch P/T) if present → SWAPS final values
    ///
    /// ## Returns
    ///
    /// `(power, toughness)` after all layers applied.
    pub fn final_pt(&self) -> (i32, i32) {
        // Layer 7a: Characteristic-defining abilities (CR 613.4a)
        // If present, this REPLACES base P/T
        let mut power = self.characteristic_value.map(|v| v.0).unwrap_or(self.base.0);
        let mut toughness = self.characteristic_value.map(|v| v.1).unwrap_or(self.base.1);

        // Layer 7b: Set P/T effects (CR 613.4b)
        // If present, this REPLACES current P/T
        if let Some((set_power, set_toughness)) = self.setpt_value {
            power = set_power;
            toughness = set_toughness;
        }

        // Layer 7c: Modify P/T effects and counters (CR 613.4c)
        // Both continuous effects and counters ADD to current P/T
        power += self.modifypt_effects.0 + self.modifypt_counters.0;
        toughness += self.modifypt_effects.1 + self.modifypt_counters.1;

        // Layer 7d: Switch P/T (CR 613.4d)
        if self.is_switched {
            (toughness, power) // Swap them
        } else {
            (power, toughness)
        }
    }

    /// Get final power (convenience method).
    pub fn power(&self) -> i32 {
        self.final_pt().0
    }

    /// Get final toughness (convenience method).
    pub fn toughness(&self) -> i32 {
        self.final_pt().1
    }
}

impl GameState {
    /// Calculate power/toughness breakdown for a creature.
    ///
    /// Implements CR 613.4 (Layer 7: Power and Toughness Changes).
    ///
    /// ## Current Implementation Status
    ///
    /// - ✅ Layer 7a: Stubbed (returns None - no characteristic-defining abilities yet)
    /// - ✅ Layer 7b: Stubbed (returns None - no set P/T effects yet)
    /// - ✅ Layer 7c (effects): Parses and applies Equipment bonuses from static abilities
    /// - ✅ Layer 7c (counters): Calculates +1/+1 and -1/-1 counter bonuses
    /// - ✅ Layer 7d: Stubbed (returns false - no switch effects yet)
    ///
    /// ## Parameters
    ///
    /// - `creature_id`: The creature to calculate P/T for
    ///
    /// ## Returns
    ///
    /// `PTBreakdown` showing contribution from each layer, or error if card not found.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let breakdown = game.get_pt_breakdown(spider_punk_id)?;
    /// assert_eq!(breakdown.base, (2, 1));           // Printed P/T
    /// assert_eq!(breakdown.modifypt_effects, (2, 2)); // Spider-Suit
    /// assert_eq!(breakdown.modifypt_counters, (1, 1)); // +1/+1 counter
    /// assert_eq!(breakdown.final_pt(), (5, 4));     // Total: 2+2+1 / 1+2+1
    /// ```
    pub fn get_pt_breakdown(&self, creature_id: CardId) -> Result<PTBreakdown> {
        let creature = self.cards.get(creature_id)?;

        // Base P/T from printed card
        let base = (creature.current_power() as i32, creature.current_toughness() as i32);

        // Layer 7a (CR 613.4a): Characteristic-defining abilities
        // TODO: Implement for creatures like Tarmogoyf (*/* based on card types)
        let characteristic_value = None;

        // Layer 7b (CR 613.4b): Set P/T effects
        // TODO: Implement for effects like "becomes 0/1" or Lignify
        let setpt_value = None;

        // Layer 7c (CR 613.4c): Modify P/T - continuous effects
        let modifypt_effects = self.calculate_modifypt_effects(creature_id)?;

        // Layer 7c (CR 613.4c): Modify P/T - counters
        let modifypt_counters = self.calculate_modifypt_counters(creature_id)?;

        // Layer 7d (CR 613.4d): Switch P/T
        // TODO: Implement for effects like "switch power and toughness"
        let is_switched = false;

        Ok(PTBreakdown {
            base,
            characteristic_value,
            setpt_value,
            modifypt_effects,
            modifypt_counters,
            is_switched,
        })
    }

    /// Check if an AffectedSelector applies to a given creature from a given source.
    ///
    /// Used primarily to support `AffectedSelector::Any` by enabling recursive checking.
    ///
    /// ## Arguments
    /// - `selector` - The selector to check
    /// - `creature_id` - The creature being evaluated
    /// - `source_id` - The permanent with the static ability
    ///
    /// ## Returns
    /// `true` if the selector matches the creature, `false` otherwise.
    fn selector_applies_to_creature(
        &self,
        selector: &crate::core::AffectedSelector,
        creature_id: CardId,
        source_id: CardId,
    ) -> bool {
        use crate::core::{AffectedSelector, CardType};

        let creature = match self.cards.get(creature_id) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let source = match self.cards.get(source_id) {
            Ok(s) => s,
            Err(_) => return false,
        };

        match selector {
            AffectedSelector::CreatureEquippedBy => self.get_attached_equipment(creature_id).contains(&source_id),
            AffectedSelector::CreaturesYouControl => creature.controller == source.controller,
            AffectedSelector::CreaturesYouControlOther => {
                creature_id != source_id && creature.controller == source.controller
            }
            AffectedSelector::AllCreatures => creature.is_creature(),
            AffectedSelector::Self_ => creature_id == source_id,
            AffectedSelector::LandAttachedBy => false, // Not relevant for creature P/T
            AffectedSelector::CreatureTypesOtherYouControl { types } => {
                creature_id != source_id
                    && creature.controller == source.controller
                    && types.iter().any(|subtype| creature.subtypes.contains(subtype))
            }
            AffectedSelector::CreatureTypeYouControl { subtype } => {
                creature.controller == source.controller && creature.subtypes.contains(subtype)
            }
            AffectedSelector::CreatureTypeOtherYouControl { subtype } => {
                creature_id != source_id
                    && creature.controller == source.controller
                    && creature.subtypes.contains(subtype)
            }
            AffectedSelector::CreatureEnchantedBy => self.get_attached_auras(creature_id).contains(&source_id),
            AffectedSelector::CreatureCardTypeOtherYouControl { card_type } => {
                creature_id != source_id
                    && creature.controller == source.controller
                    && match card_type {
                        CardType::Artifact => creature.cache.is_artifact,
                        _ => false,
                    }
            }
            AffectedSelector::CreatureCardTypeYouControl { card_type } => {
                creature.controller == source.controller
                    && match card_type {
                        CardType::Artifact => creature.cache.is_artifact,
                        _ => false,
                    }
            }
            AffectedSelector::LandCreaturesYouControl => creature.controller == source.controller && creature.is_land(),
            AffectedSelector::CreatureNonTypeOtherYouControl { excluded_subtype } => {
                creature_id != source_id
                    && creature.controller == source.controller
                    && !creature.subtypes.contains(excluded_subtype)
            }
            AffectedSelector::SelfWhenEquipped => {
                creature_id == source_id && !self.get_attached_equipment(creature_id).is_empty()
            }
            AffectedSelector::SelfWhenEnchanted => {
                creature_id == source_id && !self.get_attached_auras(creature_id).is_empty()
            }
            AffectedSelector::EquippedCreaturesYouControl => {
                creature.controller == source.controller && !self.get_attached_equipment(creature_id).is_empty()
            }
            AffectedSelector::EnchantedCreaturesYouControl => {
                creature.controller == source.controller && !self.get_attached_auras(creature_id).is_empty()
            }
            AffectedSelector::AllCreaturesOfType { subtype } => creature.subtypes.contains(subtype),
            AffectedSelector::CreaturesOpponentControls => creature.controller != source.controller,
            AffectedSelector::You | AffectedSelector::Player => false, // Player targets, not creatures
            AffectedSelector::LandsYouControl => false,                // Land targets, not creatures
            AffectedSelector::TopCardOfLibrary => false,               // Library, not battlefield
            AffectedSelector::CreatureAttachedBy => source.attached_to == Some(creature_id),
            AffectedSelector::ArtifactsYouControl => {
                creature.controller == source.controller && creature.cache.is_artifact
            }
            AffectedSelector::ArtifactsYouControlOther => {
                creature_id != source_id && creature.controller == source.controller && creature.cache.is_artifact
            }
            AffectedSelector::AllLands => creature.is_land(),
            AffectedSelector::PermanentsYouControl => creature.controller == source.controller,
            // TODO(mtg-147): Implement TokenCreaturesYouControl when is_token field is added to Card
            AffectedSelector::TokenCreaturesYouControl => false, // Not yet implemented - need is_token field
            // TODO(mtg-147): Implement TokenCreatureTypeYouControl when is_token field is added
            AffectedSelector::TokenCreatureTypeYouControl { subtype } => {
                // When is_token is added: creature.is_token && creature.controller == source.controller && creature.subtypes.contains(subtype)
                let _ = subtype; // Suppress unused warning
                false // Not yet implemented
            }
            AffectedSelector::AttackingCreaturesYouControl => {
                creature.controller == source.controller && self.combat.is_attacking(creature_id)
            }
            AffectedSelector::AllAttackingCreatures => self.combat.is_attacking(creature_id),
            AffectedSelector::Opponent => false, // Player target
            AffectedSelector::SelfWhenAttacking => creature_id == source_id && self.combat.is_attacking(creature_id),
            AffectedSelector::SelfWhenUntapped => creature_id == source_id && !creature.tapped,
            // TODO(mtg-147): SelfWhenMonstrous requires tracking monstrous state on cards
            AffectedSelector::SelfWhenMonstrous => false, // Not yet implemented - need monstrous flag
            AffectedSelector::ArtifactEnchantedBy
            | AffectedSelector::PlaneswalkerEnchantedBy
            | AffectedSelector::EquipmentEnchantedBy => self.get_attached_auras(creature_id).contains(&source_id),
            AffectedSelector::CardAttachedBy => source.attached_to == Some(creature_id),
            AffectedSelector::LandsYouOwn => creature.is_land() && creature.owner == source.controller,
            // Tapped/untapped state selectors
            AffectedSelector::TappedCreaturesYouControlOther => {
                creature_id != source_id && creature.controller == source.controller && creature.tapped
            }
            AffectedSelector::UntappedCreaturesYouControlOther => {
                creature_id != source_id && creature.controller == source.controller && !creature.tapped
            }
            // Non-land permanents
            AffectedSelector::NonLandPermanentsYouControl => {
                creature.controller == source.controller && !creature.is_land()
            }
            AffectedSelector::NonLandCardsYouOwn => creature.owner == source.controller && !creature.is_land(),
            // Generic selectors
            AffectedSelector::AllPermanents => true, // All permanents, including creatures
            AffectedSelector::AllCards => true,      // All cards
            AffectedSelector::CardsYouControl => creature.controller == source.controller,
            AffectedSelector::CardsOpponentOwns => creature.owner != source.controller,
            // Counter-based selectors - check if source has enough counters
            AffectedSelector::SelfWithCounters { counter_type, minimum } => {
                if creature_id != source_id {
                    return false;
                }
                // Check counter count on the source card
                // Note: This is a simplified check - need to map counter type strings to CounterType
                let count = match counter_type.as_str() {
                    "CHARGE" => source.get_counter(crate::core::CounterType::Charge),
                    "P1P1" => source.get_counter(crate::core::CounterType::P1P1),
                    "DIVINITY" => source.get_counter(crate::core::CounterType::Divinity),
                    _ => 0, // Unknown counter type
                };
                count >= *minimum as u8
            }
            AffectedSelector::NonBasicLands => {
                // Check if it's a land that's not a basic land (Plains, Island, Swamp, Mountain, Forest)
                if !creature.is_land() {
                    return false;
                }
                let name = creature.name.as_str();
                !(name == "Plains" || name == "Island" || name == "Swamp" || name == "Mountain" || name == "Forest")
            }
            AffectedSelector::CreatureColorOther { color } => {
                if creature_id == source_id {
                    return false;
                }
                // Check if creature has the specified color
                // Note: This is simplified - we'd need to check the card's color identity
                creature.is_creature()
                    && match color.as_str() {
                        "White" => creature.mana_cost.white > 0,
                        "Blue" => creature.mana_cost.blue > 0,
                        "Black" => creature.mana_cost.black > 0,
                        "Red" => creature.mana_cost.red > 0,
                        "Green" => creature.mana_cost.green > 0,
                        _ => false,
                    }
            }
            AffectedSelector::AllCreaturesOfColor { color } => {
                // All creatures of this color (including self) - used by Crusade
                creature.is_creature()
                    && match color.as_str() {
                        "White" => creature.mana_cost.white > 0,
                        "Blue" => creature.mana_cost.blue > 0,
                        "Black" => creature.mana_cost.black > 0,
                        "Red" => creature.mana_cost.red > 0,
                        "Green" => creature.mana_cost.green > 0,
                        _ => false,
                    }
            }
            AffectedSelector::HumanEquippedBy => {
                self.get_attached_equipment(creature_id).contains(&source_id)
                    && creature.subtypes.contains(&crate::core::Subtype::new("Human"))
            }
            AffectedSelector::SelfThisTurnEntered => {
                creature_id == source_id && creature.turn_entered_battlefield == Some(self.turn.turn_number)
            }
            AffectedSelector::Any(selectors) => {
                // Recursively check if ANY inner selector matches
                selectors
                    .iter()
                    .any(|s| self.selector_applies_to_creature(s, creature_id, source_id))
            }
            // New selectors - many are not directly applicable to creature P/T modification
            // but need placeholder matches to avoid exhaustiveness errors
            AffectedSelector::CardExiledWithSource => false, // Not applicable to creature P/T
            AffectedSelector::TopOfLibrary => false,         // Library cards, not battlefield
            AffectedSelector::LandTopOfLibrary => false,     // Library cards
            AffectedSelector::CreatureTopOfLibraryNonLand => false, // Library cards
            AffectedSelector::CommanderYouControl => {
                // Commander cards are creatures, but we don't track commander status yet
                // TODO(mtg-147): Add commander tracking
                false
            }
            AffectedSelector::EquippedByLegendary => {
                // Check if equipped by a legendary equipment
                // Note: We check if the equipment has "Legendary" subtype since supertypes
                // are currently parsed as subtypes
                let equipment = self.get_attached_equipment(creature_id);
                equipment.iter().any(|&eq_id| {
                    self.cards
                        .get(eq_id)
                        .map(|eq| eq.subtypes.contains(&crate::core::Subtype::new("Legendary")))
                        .unwrap_or(false)
                })
            }
            AffectedSelector::TopOfLibraryYouOwn => false, // Library cards
            AffectedSelector::PermanentAttachedBy => source.attached_to == Some(creature_id),
            AffectedSelector::ArtifactsNonCreature => creature.cache.is_artifact && !creature.is_creature(),
            AffectedSelector::AllArtifacts => creature.cache.is_artifact,
            AffectedSelector::BasicLandsYouControl => {
                creature.is_land()
                    && creature.controller == source.controller
                    && (creature.name.as_str() == "Plains"
                        || creature.name.as_str() == "Island"
                        || creature.name.as_str() == "Swamp"
                        || creature.name.as_str() == "Mountain"
                        || creature.name.as_str() == "Forest")
            }
            AffectedSelector::SpecificLandType { land_type } => {
                creature.is_land() && creature.subtypes.contains(&crate::core::Subtype::new(land_type))
            }
            AffectedSelector::NonLandCmcLE { max_cmc } => {
                !creature.is_land() && creature.mana_cost.cmc() as i32 <= *max_cmc
            }
            AffectedSelector::CreatureWithFlyingOppCtrl => {
                creature.is_creature()
                    && creature.controller != source.controller
                    && self.has_keyword_with_effects(creature_id, crate::core::Keyword::Flying)
            }
            AffectedSelector::CreatureTypeOther { subtype } => {
                creature_id != source_id && creature.subtypes.contains(subtype)
            }
            AffectedSelector::SliversYouControl => {
                creature.controller == source.controller
                    && creature.subtypes.contains(&crate::core::Subtype::new("Sliver"))
            }
            AffectedSelector::PermanentEquippedBy => self.get_attached_equipment(creature_id).contains(&source_id),
            AffectedSelector::VehicleAttachedBy => {
                source.attached_to == Some(creature_id)
                    && creature.subtypes.contains(&crate::core::Subtype::new("Vehicle"))
            }
            AffectedSelector::NonLandCardsYouOwnWithoutForetell => {
                // TODO(mtg-147): Track foretell status
                creature.owner == source.controller && !creature.is_land()
            }
            AffectedSelector::TopOfLibraryNonLand => false, // Library cards
            AffectedSelector::RememberedCards => false,     // TODO(mtg-147): Track remembered cards
            AffectedSelector::CreatureYouControlWasCast => {
                // TODO(mtg-147): Track whether creature was cast vs put into play
                creature.is_creature() && creature.controller == source.controller
            }
            // Ownership-based selectors for non-battlefield zones (graveyard, exile)
            // Not relevant for P/T modifications of creatures on battlefield
            AffectedSelector::CardTypeYouOwn { .. } => false,
            AffectedSelector::SubtypeYouOwn { .. } => false,
        }
    }

    /// Calculate Layer 7c continuous effects (Equipment, anthems, etc).
    ///
    /// ## CR 613.4c
    ///
    /// > Layer 7c: Effects and counters that modify power and/or toughness
    /// > (but don't set power and/or toughness to a specific number or value)
    /// > are applied.
    ///
    /// ## Current Implementation
    ///
    /// - Checks all permanents on the battlefield for static ModifyPT abilities
    /// - Applies Equipment bonuses (Creature.EquippedBy)
    /// - Applies anthem effects (CreaturesYouControl, AllCreatures, CreatureTypesOtherYouControl)
    ///
    /// ## Returns
    ///
    /// `(power_bonus, toughness_bonus)` from all continuous effects.
    fn calculate_modifypt_effects(&self, creature_id: CardId) -> Result<(i32, i32)> {
        use crate::core::{AffectedSelector, CardType, StaticAbility};

        let mut power_bonus = 0;
        let mut toughness_bonus = 0;

        // Check all permanents on the battlefield for static abilities
        // This includes Equipment, enchantments, creatures (like Spider-Ham), etc.
        for &source_id in &self.battlefield.cards {
            let source = self.cards.get(source_id)?;

            // Process all static abilities on this permanent
            for ability in &source.static_abilities {
                match ability {
                    StaticAbility::ModifyPT {
                        affected,
                        power,
                        toughness,
                        description: _,
                    } => {
                        // Check if this ability affects the target creature
                        match affected {
                            AffectedSelector::CreatureEquippedBy => {
                                // This Equipment grants bonuses to the creature it's attached to
                                // Check if this Equipment is attached to creature_id
                                let attached_equipment = self.get_attached_equipment(creature_id);
                                if attached_equipment.contains(&source_id) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreaturesYouControl => {
                                // Check if creature is controlled by the source's owner
                                // Example: Glorious Anthem giving +1/+1 to creatures you control
                                let creature = self.cards.get(creature_id)?;
                                if creature.controller == source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreaturesYouControlOther => {
                                // Check if creature is controlled by source's owner AND not the source
                                // Example: Elesh Norn giving +2/+2 to other creatures you control
                                let creature = self.cards.get(creature_id)?;
                                if creature_id != source_id && creature.controller == source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::AllCreatures => {
                                // Apply to all creatures on the battlefield
                                // Example: Global effects like "All creatures get -1/-1"
                                let creature = self.cards.get(creature_id)?;
                                if creature.is_creature() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::Self_ => {
                                // Equipment affecting itself (not the equipped creature)
                                // Skip - not relevant for this creature's P/T
                            }
                            AffectedSelector::LandAttachedBy => {
                                // This Aura grants abilities to the land it's attached to
                                // Not relevant for creature P/T calculation - skip
                            }
                            AffectedSelector::CreatureTypesOtherYouControl { types } => {
                                // Check if this affects the creature:
                                // 1. Creature must match one of the listed types
                                // 2. Creature must be controlled by the source's controller
                                // 3. Creature must NOT be the source itself (Other qualifier)

                                let creature = self.cards.get(creature_id)?;

                                // Check "Other" - exclude the source card itself
                                if creature_id == source_id {
                                    continue;
                                }

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature has one of the listed types
                                let has_matching_type = types.iter().any(|subtype| creature.subtypes.contains(subtype));

                                if has_matching_type {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreatureTypeYouControl { subtype } => {
                                // Check if this affects the creature:
                                // 1. Creature must have the specified subtype
                                // 2. Creature must be controlled by the source's controller
                                // Example: Goblin Chieftain granting +1/+1 to all Goblins you control

                                let creature = self.cards.get(creature_id)?;

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature has the specified subtype
                                if creature.subtypes.contains(subtype) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreatureTypeOtherYouControl { subtype } => {
                                // Check if this affects the creature:
                                // 1. Creature must have the specified subtype
                                // 2. Creature must be controlled by the source's controller
                                // 3. Creature must NOT be the source itself (Other qualifier)
                                // Example: Death Baron granting +1/+1 to other Zombies you control

                                let creature = self.cards.get(creature_id)?;

                                // Check "Other" - exclude the source card itself
                                if creature_id == source_id {
                                    continue;
                                }

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature has the specified subtype
                                if creature.subtypes.contains(subtype) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreatureEnchantedBy => {
                                // This Aura grants bonuses to the creature it's attached to
                                // Check if this Aura is attached to creature_id
                                let attached_auras = self.get_attached_auras(creature_id);
                                if attached_auras.contains(&source_id) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreatureCardTypeOtherYouControl { card_type } => {
                                // Check if this affects the creature:
                                // 1. Creature must have the specified card type (e.g., Artifact)
                                // 2. Creature must be controlled by the source's controller
                                // 3. Creature must NOT be the source itself (Other qualifier)
                                // Example: Master of Etherium granting +1/+1 to other artifact creatures

                                let creature = self.cards.get(creature_id)?;

                                // Check "Other" - exclude the source card itself
                                if creature_id == source_id {
                                    continue;
                                }

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature has the specified card type
                                // Use the cached type flags for efficiency where available
                                let has_type = match card_type {
                                    CardType::Artifact => creature.cache.is_artifact,
                                    CardType::Land => creature.cache.is_land,
                                    CardType::Creature => creature.is_creature(),
                                    // Enchantment and other types not cached, use direct check
                                    _ => creature.types.contains(card_type),
                                };

                                if has_type {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreatureCardTypeYouControl { card_type } => {
                                // Check if this affects the creature:
                                // 1. Creature must have the specified card type (e.g., Artifact)
                                // 2. Creature must be controlled by the source's controller
                                // (No "Other" qualifier - source can buff itself)

                                let creature = self.cards.get(creature_id)?;

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature has the specified card type
                                let has_type = match card_type {
                                    CardType::Artifact => creature.cache.is_artifact,
                                    CardType::Land => creature.cache.is_land,
                                    CardType::Creature => creature.is_creature(),
                                    _ => creature.types.contains(card_type),
                                };

                                if has_type {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::LandCreaturesYouControl => {
                                // Check if this affects the creature:
                                // 1. Creature must be a Land (type Creature + type Land)
                                // 2. Creature must be controlled by the source's controller
                                // Example: "Land creatures you control have trample"

                                let creature = self.cards.get(creature_id)?;

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature is both a Creature and a Land
                                // (animated lands like Dryad Arbor or man-lands)
                                if creature.is_creature() && creature.cache.is_land {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreatureNonTypeOtherYouControl { excluded_subtype } => {
                                // Check if this affects the creature:
                                // 1. Creature must NOT have the specified subtype (e.g., not Human)
                                // 2. Creature must be controlled by the source's controller
                                // 3. Creature must NOT be the source itself (Other qualifier)
                                // Example: Mikaeus, the Unhallowed - "Other non-Human creatures you control get +1/+1"

                                let creature = self.cards.get(creature_id)?;

                                // Check "Other" - exclude the source card itself
                                if creature_id == source_id {
                                    continue;
                                }

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if creature does NOT have the excluded subtype
                                if !creature.subtypes.contains(excluded_subtype) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::SelfWhenEquipped => {
                                // Check if this affects the creature:
                                // 1. Must be the source card itself
                                // 2. Must have at least one equipment attached
                                // Example: Leonin Lightbringer - "As long as ~ is equipped, it gets +1/+1"

                                // Only affects the source itself
                                if creature_id != source_id {
                                    continue;
                                }

                                // Check if the creature is equipped (any equipment attached to it)
                                let attached_equipment = self.get_attached_equipment(creature_id);
                                if !attached_equipment.is_empty() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::SelfWhenEnchanted => {
                                // Check if this affects the creature:
                                // 1. Must be the source card itself
                                // 2. Must have at least one aura attached
                                // Example: Thran Golem - "As long as ~ is enchanted, it gets +2/+2"

                                // Only affects the source itself
                                if creature_id != source_id {
                                    continue;
                                }

                                // Check if the creature is enchanted (any aura attached to it)
                                let attached_auras = self.get_attached_auras(creature_id);
                                if !attached_auras.is_empty() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::EquippedCreaturesYouControl => {
                                // Check if this affects the creature:
                                // 1. Creature must be controlled by the source's controller
                                // 2. Creature must have at least one equipment attached
                                // Example: Kemba, Kha Enduring - "Equipped creatures you control get +1/+1"

                                let creature = self.cards.get(creature_id)?;

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if the creature is equipped
                                let attached_equipment = self.get_attached_equipment(creature_id);
                                if !attached_equipment.is_empty() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::EnchantedCreaturesYouControl => {
                                // Check if this affects the creature:
                                // 1. Creature must be controlled by the source's controller
                                // 2. Creature must have at least one aura attached
                                // Example: Similar to EquippedCreaturesYouControl but for auras

                                let creature = self.cards.get(creature_id)?;

                                // Check "YouCtrl" - creature must be controlled by source's controller
                                if creature.controller != source.controller {
                                    continue;
                                }

                                // Check if the creature is enchanted
                                let attached_auras = self.get_attached_auras(creature_id);
                                if !attached_auras.is_empty() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::AllCreaturesOfType { subtype } => {
                                // Check if this affects the creature:
                                // 1. Creature must have the specified subtype
                                // 2. Affects ALL creatures of that type (global, not just yours)
                                // Example: Sliver lords - "All Slivers have/get..."
                                //
                                // This is different from CreatureTypeYouControl which only
                                // affects your own creatures.

                                let creature = self.cards.get(creature_id)?;

                                // Check if creature has the subtype
                                if creature.subtypes.contains(subtype) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CreaturesOpponentControls => {
                                // Check if this affects the creature:
                                // 1. Creature must be controlled by an opponent of the source's controller
                                // Example: Debuff effects like "Creatures your opponents control get -1/-1"

                                let creature = self.cards.get(creature_id)?;

                                // Check "OppCtrl" - creature must be controlled by opponent
                                if creature.controller != source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Player-targeted selectors don't affect creature P/T
                            AffectedSelector::You | AffectedSelector::Player => {
                                // These selectors affect players (e.g., granting Protection to you)
                                // Not relevant for creature P/T calculation
                            }
                            // Land selectors don't affect creature P/T (unless the land is a creature)
                            AffectedSelector::LandsYouControl => {
                                // This grants abilities/bonuses to lands you control
                                // Not relevant for creature P/T calculation
                            }
                            // Top card of library doesn't affect creature P/T
                            AffectedSelector::TopCardOfLibrary => {
                                // Affects visibility/playability of top library card
                                // Not relevant for creature P/T calculation
                            }
                            AffectedSelector::CreatureAttachedBy => {
                                // Check if this affects the creature:
                                // 1. This is from an Aura or Equipment
                                // 2. The source must be attached to the creature
                                // Example: Equipment/Auras with "Equipped/Enchanted creature gets +X/+Y"

                                // Check if source is attached to this creature
                                if let Some(attached_to) = source.attached_to {
                                    if attached_to == creature_id {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            // Artifact selectors don't affect creature P/T directly
                            // (unless the artifact is also a creature)
                            AffectedSelector::ArtifactsYouControl | AffectedSelector::ArtifactsYouControlOther => {
                                // This grants abilities/bonuses to artifacts you control
                                // Only relevant if the creature is also an artifact
                                let creature = self.cards.get(creature_id)?;
                                if creature.is_artifact() {
                                    // For ArtifactsYouControlOther, exclude self
                                    if matches!(affected, AffectedSelector::ArtifactsYouControlOther)
                                        && creature_id == source_id
                                    {
                                        continue;
                                    }
                                    // Check controller match
                                    if creature.controller == source.controller {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            // Land selector doesn't affect creature P/T (unless land is animated)
                            AffectedSelector::AllLands => {
                                // This grants abilities/bonuses to lands
                                // Only relevant if the creature is also a land
                                let creature = self.cards.get(creature_id)?;
                                if creature.is_land() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Permanents you control
                            AffectedSelector::PermanentsYouControl => {
                                // Affects all permanents you control (creatures, artifacts, etc.)
                                let creature = self.cards.get(creature_id)?;
                                if creature.controller == source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Token creatures you control
                            // TODO(mtg-147): Implement when is_token field is added to Card
                            AffectedSelector::TokenCreaturesYouControl => {
                                // Would need is_token field on Card struct
                                // For now, this selector is parsed but not evaluated
                            }
                            // Token creatures of a specific type you control
                            // TODO(mtg-147): Implement when is_token field is added to Card
                            AffectedSelector::TokenCreatureTypeYouControl { .. } => {
                                // Would need: creature.is_token && creature.controller == source.controller && creature.subtypes.contains(subtype)
                                // For now, this selector is parsed but not evaluated
                            }
                            // Attacking creatures you control
                            AffectedSelector::AttackingCreaturesYouControl => {
                                // Only affects creatures you control that are attacking
                                let creature = self.cards.get(creature_id)?;
                                if creature.controller == source.controller && self.combat.is_attacking(creature_id) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // All attacking creatures
                            AffectedSelector::AllAttackingCreatures => {
                                // Affects all attacking creatures regardless of controller
                                if self.combat.is_attacking(creature_id) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Opponent player - doesn't affect creature P/T
                            AffectedSelector::Opponent => {
                                // This affects players, not creatures
                                // Not relevant for creature P/T calculation
                            }
                            // Self while attacking - grants bonuses while this creature attacks
                            AffectedSelector::SelfWhenAttacking => {
                                // Only applies to self and only while attacking
                                if creature_id == source.id && self.combat.is_attacking(creature_id) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Enchanted permanent selectors - these are for Auras that grant effects
                            // to the enchanted permanent. For creature P/T calculations, these only
                            // apply if the enchanted permanent is a creature.
                            AffectedSelector::ArtifactEnchantedBy
                            | AffectedSelector::PlaneswalkerEnchantedBy
                            | AffectedSelector::EquipmentEnchantedBy => {
                                // These auras typically enchant non-creatures
                                // For P/T calculation, they would only matter if the artifact/etc
                                // becomes animated into a creature. Check if this aura is attached
                                // to the creature we're calculating P/T for.
                                if let Some(attached_to) = source.attached_to {
                                    if attached_to == creature_id {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            // Generic "Card.AttachedBy" - any permanent this aura is attached to
                            AffectedSelector::CardAttachedBy => {
                                // Check if this aura is attached to the creature
                                if let Some(attached_to) = source.attached_to {
                                    if attached_to == creature_id {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            // Land.YouOwn - affects lands you own (for graveyard effects)
                            // Not relevant for creature P/T unless land is animated
                            AffectedSelector::LandsYouOwn => {
                                // These are typically "may play" effects, not P/T modifiers
                                // Only relevant if creature is also a land
                                let creature = self.cards.get(creature_id)?;
                                if creature.is_land() && creature.owner == source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Self while untapped - grants bonuses while this card is untapped
                            AffectedSelector::SelfWhenUntapped => {
                                let creature = self.cards.get(creature_id)?;
                                if creature_id == source.id && !creature.tapped {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Self while monstrous - grants abilities when Monstrosity has been activated
                            // TODO(mtg-147): Requires tracking monstrous state on cards
                            AffectedSelector::SelfWhenMonstrous => {
                                // Not yet implemented - need is_monstrous flag on Card
                                // Would check: creature_id == source.id && source.is_monstrous
                            }
                            // Tapped creatures you control (other than self)
                            AffectedSelector::TappedCreaturesYouControlOther => {
                                let creature = self.cards.get(creature_id)?;
                                if creature_id != source.id
                                    && creature.controller == source.controller
                                    && creature.tapped
                                {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Untapped creatures you control (other than self)
                            AffectedSelector::UntappedCreaturesYouControlOther => {
                                let creature = self.cards.get(creature_id)?;
                                if creature_id != source.id
                                    && creature.controller == source.controller
                                    && !creature.tapped
                                {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Non-land permanents you control
                            AffectedSelector::NonLandPermanentsYouControl => {
                                let creature = self.cards.get(creature_id)?;
                                if creature.controller == source.controller && !creature.is_land() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Non-land cards you own
                            AffectedSelector::NonLandCardsYouOwn => {
                                let creature = self.cards.get(creature_id)?;
                                if creature.owner == source.controller && !creature.is_land() {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Generic selectors
                            AffectedSelector::AllPermanents => {
                                power_bonus += power;
                                toughness_bonus += toughness;
                            }
                            AffectedSelector::AllCards => {
                                power_bonus += power;
                                toughness_bonus += toughness;
                            }
                            AffectedSelector::CardsYouControl => {
                                let creature = self.cards.get(creature_id)?;
                                if creature.controller == source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::CardsOpponentOwns => {
                                let creature = self.cards.get(creature_id)?;
                                if creature.owner != source.controller {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // Counter-based self selectors
                            AffectedSelector::SelfWithCounters { counter_type, minimum } => {
                                if creature_id == source.id {
                                    // Check counter count
                                    let count = match counter_type.as_str() {
                                        "CHARGE" => source.get_counter(crate::core::CounterType::Charge),
                                        "P1P1" => source.get_counter(crate::core::CounterType::P1P1),
                                        "DIVINITY" => source.get_counter(crate::core::CounterType::Divinity),
                                        _ => 0,
                                    };
                                    if count >= *minimum as u8 {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            AffectedSelector::NonBasicLands => {
                                let creature = self.cards.get(creature_id)?;
                                if creature.is_land() {
                                    // Check if it's not a basic land
                                    let name = creature.name.as_str();
                                    let is_basic = name == "Plains"
                                        || name == "Island"
                                        || name == "Swamp"
                                        || name == "Mountain"
                                        || name == "Forest";
                                    if !is_basic {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            AffectedSelector::CreatureColorOther { color } => {
                                if creature_id != source.id {
                                    let creature = self.cards.get(creature_id)?;
                                    let matches = creature.is_creature()
                                        && match color.as_str() {
                                            "White" => creature.mana_cost.white > 0,
                                            "Blue" => creature.mana_cost.blue > 0,
                                            "Black" => creature.mana_cost.black > 0,
                                            "Red" => creature.mana_cost.red > 0,
                                            "Green" => creature.mana_cost.green > 0,
                                            _ => false,
                                        };
                                    if matches {
                                        power_bonus += power;
                                        toughness_bonus += toughness;
                                    }
                                }
                            }
                            AffectedSelector::AllCreaturesOfColor { color } => {
                                // All creatures of this color (including self) - used by Crusade
                                let creature = self.cards.get(creature_id)?;
                                let matches = creature.is_creature()
                                    && match color.as_str() {
                                        "White" => creature.mana_cost.white > 0,
                                        "Blue" => creature.mana_cost.blue > 0,
                                        "Black" => creature.mana_cost.black > 0,
                                        "Red" => creature.mana_cost.red > 0,
                                        "Green" => creature.mana_cost.green > 0,
                                        _ => false,
                                    };
                                if matches {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::HumanEquippedBy => {
                                let creature = self.cards.get(creature_id)?;
                                if self.get_attached_equipment(creature_id).contains(&source_id)
                                    && creature.subtypes.contains(&crate::core::Subtype::new("Human"))
                                {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            AffectedSelector::SelfThisTurnEntered => {
                                let creature = self.cards.get(creature_id)?;
                                if creature_id == source.id
                                    && creature.turn_entered_battlefield == Some(self.turn.turn_number)
                                {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // OR combination - match if ANY inner selector matches
                            AffectedSelector::Any(selectors) => {
                                // Use the helper to check if any selector applies
                                if selectors
                                    .iter()
                                    .any(|s| self.selector_applies_to_creature(s, creature_id, source_id))
                                {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                            // New selectors - use the unified helper
                            AffectedSelector::CardExiledWithSource
                            | AffectedSelector::TopOfLibrary
                            | AffectedSelector::LandTopOfLibrary
                            | AffectedSelector::CreatureTopOfLibraryNonLand
                            | AffectedSelector::CommanderYouControl
                            | AffectedSelector::EquippedByLegendary
                            | AffectedSelector::TopOfLibraryYouOwn
                            | AffectedSelector::PermanentAttachedBy
                            | AffectedSelector::ArtifactsNonCreature
                            | AffectedSelector::AllArtifacts
                            | AffectedSelector::BasicLandsYouControl
                            | AffectedSelector::SpecificLandType { .. }
                            | AffectedSelector::NonLandCmcLE { .. }
                            | AffectedSelector::CreatureWithFlyingOppCtrl
                            | AffectedSelector::CreatureTypeOther { .. }
                            | AffectedSelector::SliversYouControl
                            | AffectedSelector::PermanentEquippedBy
                            | AffectedSelector::VehicleAttachedBy
                            | AffectedSelector::NonLandCardsYouOwnWithoutForetell
                            | AffectedSelector::TopOfLibraryNonLand
                            | AffectedSelector::RememberedCards
                            | AffectedSelector::CreatureYouControlWasCast
                            | AffectedSelector::CardTypeYouOwn { .. }
                            | AffectedSelector::SubtypeYouOwn { .. } => {
                                // Use the unified selector_applies_to_creature helper
                                if self.selector_applies_to_creature(affected, creature_id, source_id) {
                                    power_bonus += power;
                                    toughness_bonus += toughness;
                                }
                            }
                        }
                    }
                    StaticAbility::GrantKeyword { .. } => {
                        // GrantKeyword abilities don't affect P/T
                        // They are handled in get_granted_keywords() instead
                    }
                }
            }
        }

        Ok((power_bonus, toughness_bonus))
    }

    /// Calculate Layer 7c counter bonuses (+1/+1, -1/-1, etc).
    ///
    /// ## CR 613.4c
    ///
    /// > Layer 7c: Effects and counters that modify power and/or toughness
    /// > (but don't set power and/or toughness to a specific number or value)
    /// > are applied.
    ///
    /// ## Implementation
    ///
    /// Matches Java Forge's `getPowerBonusFromCounters()` logic:
    /// - +1/+1 counters: +1 power each
    /// - -1/-1 counters: -1 power each
    /// - Other counter types as needed
    ///
    /// ## Returns
    ///
    /// `(power_bonus, toughness_bonus)` from all counters.
    fn calculate_modifypt_counters(&self, creature_id: CardId) -> Result<(i32, i32)> {
        use crate::core::CounterType;

        let creature = self.cards.get(creature_id)?;

        // Count +1/+1 and -1/-1 counters
        let plus_counters = creature.get_counter(CounterType::P1P1) as i32;
        let minus_counters = creature.get_counter(CounterType::M1M1) as i32;

        let power_bonus = plus_counters - minus_counters;
        let toughness_bonus = plus_counters - minus_counters;

        Ok((power_bonus, toughness_bonus))
    }

    /// Get all keywords granted to a creature by static abilities (Layer 6).
    ///
    /// ## CR 613 Layer 6: Ability Adding or Removing Effects
    ///
    /// This calculates keywords granted by continuous effects from other permanents.
    /// For example, Spider-Punk granting Riot to other Spiders.
    ///
    /// ## Returns
    ///
    /// A KeywordSet containing all keywords granted to the creature by static abilities.
    pub fn get_granted_keywords(&self, creature_id: CardId) -> crate::core::KeywordSet {
        use crate::core::effects::AffectedSelector;
        use crate::core::KeywordSet;

        let mut granted = KeywordSet::new();

        // Get the target creature
        let creature = match self.cards.get(creature_id) {
            Ok(c) => c,
            Err(_) => return granted,
        };

        // Check all permanents on the battlefield for GrantKeyword abilities
        for &source_id in &self.battlefield.cards {
            let source = match self.cards.get(source_id) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Process GrantKeyword abilities
            for ability in &source.static_abilities {
                if let crate::core::StaticAbility::GrantKeyword {
                    affected,
                    keyword,
                    description: _,
                } = ability
                {
                    // Check if this ability affects the target creature
                    let affects_creature = match affected {
                        AffectedSelector::CreatureTypeOtherYouControl { subtype } => {
                            // Creature must: match subtype, be controlled by source controller, not be source
                            creature_id != source_id
                                && creature.controller == source.controller
                                && creature.subtypes.contains(subtype)
                        }
                        AffectedSelector::CreatureTypesOtherYouControl { types } => {
                            // Creature must: match any type, be controlled by source controller, not be source
                            creature_id != source_id
                                && creature.controller == source.controller
                                && types.iter().any(|t| creature.subtypes.contains(t))
                        }
                        AffectedSelector::CreaturesYouControl => creature.controller == source.controller,
                        AffectedSelector::CreaturesYouControlOther => {
                            // Creature must: be controlled by source controller, not be source
                            creature_id != source_id && creature.controller == source.controller
                        }
                        AffectedSelector::CreatureEquippedBy => {
                            // Grant keyword to equipped creature
                            self.get_attached_equipment(creature_id).contains(&source_id)
                        }
                        AffectedSelector::CreatureEnchantedBy => {
                            // Grant keyword to enchanted creature
                            self.get_attached_auras(creature_id).contains(&source_id)
                        }
                        AffectedSelector::AllCreatures => creature.is_creature(),
                        AffectedSelector::AllCreaturesOfType { subtype } => {
                            // Grant keyword to all creatures with this subtype (global)
                            // Used by Sliver lords: "All Slivers have..."
                            creature.subtypes.contains(subtype)
                        }
                        AffectedSelector::SelfWhenAttacking => {
                            // Grant keyword to self only while attacking
                            // Used by cards like Soltari Lancer
                            creature_id == source_id && self.combat.is_attacking(creature_id)
                        }
                        AffectedSelector::Any(selectors) => {
                            // OR combination - match if ANY inner selector matches
                            selectors
                                .iter()
                                .any(|s| self.selector_applies_to_creature(s, creature_id, source_id))
                        }
                        _ => false, // Other selectors not yet supported for keywords
                    };

                    if affects_creature {
                        granted.insert(*keyword);
                    }
                }
            }
        }

        granted
    }

    /// Check if a creature has a keyword, including granted keywords from continuous effects.
    ///
    /// This method should be used when checking keywords for gameplay purposes (combat,
    /// ability resolution, etc.) as it accounts for keywords granted by static abilities
    /// from other permanents.
    ///
    /// ## CR 613 Layer 6
    ///
    /// Per the layer system, abilities are granted before combat-related effects are resolved.
    ///
    /// ## Example
    ///
    /// Spider-Punk grants Riot to other Spiders:
    /// ```ignore
    /// let creature_id = spider_token;
    /// if game.has_keyword_with_effects(creature_id, Keyword::Riot) {
    ///     // Spider has Riot from Spider-Punk
    /// }
    /// ```
    #[inline]
    pub fn has_keyword_with_effects(&self, creature_id: CardId, keyword: crate::core::Keyword) -> bool {
        // First check the card's static keywords (fast path)
        if let Ok(card) = self.cards.get(creature_id) {
            if card.has_keyword(keyword) {
                return true;
            }
        }

        // Then check for granted keywords from continuous effects
        self.get_granted_keywords(creature_id).contains(keyword)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pt_breakdown_base_only() {
        let breakdown = PTBreakdown {
            base: (2, 1),
            characteristic_value: None,
            setpt_value: None,
            modifypt_effects: (0, 0),
            modifypt_counters: (0, 0),
            is_switched: false,
        };

        assert_eq!(breakdown.final_pt(), (2, 1));
        assert_eq!(breakdown.power(), 2);
        assert_eq!(breakdown.toughness(), 1);
    }

    #[test]
    fn test_pt_breakdown_with_equipment() {
        // Spider-Punk (2/1) with Spider-Suit (+2/+2)
        let breakdown = PTBreakdown {
            base: (2, 1),
            characteristic_value: None,
            setpt_value: None,
            modifypt_effects: (2, 2),
            modifypt_counters: (0, 0),
            is_switched: false,
        };

        assert_eq!(breakdown.final_pt(), (4, 3));
    }

    #[test]
    fn test_pt_breakdown_with_counters() {
        // Grizzly Bears (2/2) with +1/+1 counter
        let breakdown = PTBreakdown {
            base: (2, 2),
            characteristic_value: None,
            setpt_value: None,
            modifypt_effects: (0, 0),
            modifypt_counters: (1, 1),
            is_switched: false,
        };

        assert_eq!(breakdown.final_pt(), (3, 3));
    }

    #[test]
    fn test_pt_breakdown_equipment_and_counters() {
        // Spider-Punk (2/1) with Spider-Suit (+2/+2) and +1/+1 counter
        let breakdown = PTBreakdown {
            base: (2, 1),
            characteristic_value: None,
            setpt_value: None,
            modifypt_effects: (2, 2),
            modifypt_counters: (1, 1),
            is_switched: false,
        };

        assert_eq!(breakdown.final_pt(), (5, 4));
    }

    #[test]
    fn test_pt_breakdown_with_setpt() {
        // Gray Ogre (2/2) with +1/+1 counter and "becomes 0/1" effect
        // Per CR 613.5 example: 0/1 base, +1/+1 from counter = 1/2
        let breakdown = PTBreakdown {
            base: (2, 2),
            characteristic_value: None,
            setpt_value: Some((0, 1)), // Layer 7b REPLACES base
            modifypt_effects: (0, 0),
            modifypt_counters: (1, 1),
            is_switched: false,
        };

        assert_eq!(breakdown.final_pt(), (1, 2));
    }

    #[test]
    fn test_pt_breakdown_cr_613_5_gray_ogre_example() {
        // CR 613.5 Example: Gray Ogre (2/2) with:
        // - +1/+1 counter (layer 7c) → 3/3
        // - +4/+4 spell (layer 7c) → 7/7
        // - +0/+2 enchantment (layer 7c) → 7/9
        // - "becomes 0/1" effect (layer 7b) → 5/8
        //
        // Calculation: 0/1 (setpt) + 4/4 (spell) + 0/2 (enchantment) + 1/1 (counter) = 5/8
        let breakdown = PTBreakdown {
            base: (2, 2),
            characteristic_value: None,
            setpt_value: Some((0, 1)), // Layer 7b
            modifypt_effects: (4, 6),  // Layer 7c: +4/+4 spell + +0/+2 enchantment
            modifypt_counters: (1, 1), // Layer 7c: +1/+1 counter
            is_switched: false,
        };

        assert_eq!(breakdown.final_pt(), (5, 8));
    }

    #[test]
    fn test_pt_breakdown_switch() {
        // 2/1 creature with switched P/T
        let breakdown = PTBreakdown {
            base: (2, 1),
            characteristic_value: None,
            setpt_value: None,
            modifypt_effects: (0, 0),
            modifypt_counters: (0, 0),
            is_switched: true,
        };

        assert_eq!(breakdown.final_pt(), (1, 2)); // Swapped
    }

    #[test]
    fn test_pt_breakdown_switch_with_buffs() {
        // CR 613.4d Example: 1/3 creature with +0/+1, then switch P/T
        // "Unswitched" would be 1/4, so switched is 4/1
        let breakdown = PTBreakdown {
            base: (1, 3),
            characteristic_value: None,
            setpt_value: None,
            modifypt_effects: (0, 1),
            modifypt_counters: (0, 0),
            is_switched: true,
        };

        // Unswitched: 1+0 / 3+1 = 1/4
        // Switched: 4/1
        assert_eq!(breakdown.final_pt(), (4, 1));
    }

    #[test]
    fn test_creature_has_keyword_inherent() {
        use crate::core::{Card, CardId, CardType, Keyword};

        // Create a simple game state
        let mut game = crate::game::GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let player_id = game.players[0].id;

        // Create a creature with flying
        let creature_id: CardId = game.next_id();
        let mut creature = Card::new(creature_id, "Bird", player_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.keywords.insert(Keyword::Flying);

        game.cards.insert(creature_id, creature);

        // Test inherent keyword detection
        assert!(game.has_keyword_with_effects(creature_id, Keyword::Flying));
        assert!(!game.has_keyword_with_effects(creature_id, Keyword::Haste));
    }

    #[test]
    fn test_creature_has_keyword_granted() {
        use crate::core::effects::AffectedSelector;
        use crate::core::StaticAbility;
        use crate::core::{Card, CardId, CardType, Keyword};

        // Create a simple game state
        let mut game = crate::game::GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let player_id = game.players[0].id;

        // Create a creature without haste
        let creature_id: CardId = game.next_id();
        let mut creature = Card::new(creature_id, "Spider", player_id);
        creature.add_type(CardType::Creature);
        creature.subtypes.push("Spider".into());
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = player_id;
        game.cards.insert(creature_id, creature);

        // Create a lord that grants haste to other Spiders (like Spider-Punk)
        let lord_id: CardId = game.next_id();
        let mut lord = Card::new(lord_id, "Spider Lord", player_id);
        lord.add_type(CardType::Creature);
        lord.subtypes.push("Spider".into());
        lord.controller = player_id;
        lord.static_abilities.push(StaticAbility::GrantKeyword {
            affected: AffectedSelector::CreatureTypeOtherYouControl {
                subtype: "Spider".into(),
            },
            keyword: Keyword::Haste,
            description: "Other Spiders you control have haste".into(),
        });
        game.cards.insert(lord_id, lord);

        // Add both to battlefield
        game.battlefield.add(creature_id);
        game.battlefield.add(lord_id);

        // The creature should have haste from the lord's grant
        assert!(!game.cards.get(creature_id).unwrap().has_keyword(Keyword::Haste)); // Inherent: no
        assert!(game.has_keyword_with_effects(creature_id, Keyword::Haste)); // With grants: yes

        // The lord itself should NOT have haste (ability says "other")
        assert!(!game.has_keyword_with_effects(lord_id, Keyword::Haste));
    }
}
