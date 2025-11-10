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
    /// - ✅ Layer 7c (effects): Calculates Equipment bonuses (hardcoded Spider-Suit for now)
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
        let base = (
            creature.power.unwrap_or(0) as i32,
            creature.toughness.unwrap_or(0) as i32,
        );

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
    /// - Finds all Equipment attached to the creature
    /// - Hardcoded: Spider-Suit grants +2/+2
    /// - TODO (Phase 3): Parse static abilities from card data instead of hardcoding
    ///
    /// ## Returns
    ///
    /// `(power_bonus, toughness_bonus)` from all continuous effects.
    fn calculate_modifypt_effects(&self, creature_id: CardId) -> Result<(i32, i32)> {
        let mut power_bonus = 0;
        let mut toughness_bonus = 0;

        // Get all Equipment attached to this creature
        let equipment_list = self.get_attached_equipment(creature_id);

        for equip_id in equipment_list {
            let equipment = self.cards.get(equip_id)?;

            // TODO(mtg-98df7d): Replace with parsed static abilities
            // Instead of hardcoding Spider-Suit, parse:
            //   S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2
            if equipment.name.as_str().eq_ignore_ascii_case("Spider-Suit") {
                power_bonus += 2;
                toughness_bonus += 2;
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
}
