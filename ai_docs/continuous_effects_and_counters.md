# Continuous Effects and Counters in Java Forge

## How Java Forge Calculates Final Power/Toughness

Based on analysis of `Card.java` (lines 4640-4748), Java Forge uses a **layered calculation system** that separates different sources of P/T modification:

### The Calculation Formula

```java
// StatBreakdown structure (lines 4699-4720)
public static class StatBreakdown {
    public final int currentValue;      // Base P/T after layer effects
    public final int tempBoost;         // Continuous effects (Equipment, enchantments, etc)
    public final int bonusFromCounters; // +1/+1 counters, -1/-1 counters, etc

    public int getTotal() {
        return currentValue + tempBoost + bonusFromCounters;
    }
}
```

### Final Power Calculation (lines 4659-4687)

```java
public final StatBreakdown getUnswitchedPowerBreakdown() {
    // Step 1: Get current power (after SETPT and CHARACTERISTIC layers)
    int currentValue = getCurrentPower();

    // Step 2: Get temp boost (from MODIFYPT layer - continuous effects)
    int tempBoost = getTempPowerBoost();

    // Step 3: Get counter bonuses (from actual counters on card)
    int bonusFromCounters = getPowerBonusFromCounters();

    return new StatBreakdown(currentValue, tempBoost, bonusFromCounters);
}

public final int getNetPower() {
    return getUnswitchedPowerBreakdown().getTotal();
}
```

## The Three Components Explained

### 1. **currentValue**: Base P/T After Layer Effects

This is calculated by `getCurrentPower()` (lines 4649-4657):

```java
public final int getCurrentPower() {
    int total = getBasePower();  // Start with printed P/T

    // Apply layer effects that SET P/T (not add)
    for (Pair<Integer, Integer> p : getPTIterable()) {
        if (p.getLeft() != null) {
            total = p.getLeft();  // REPLACE base with new value
        }
    }
    return total;
}
```

**Examples**:
- Normal creature: `currentValue = printed power` (e.g., Grizzly Bears = 2)
- With "set P/T" effect: `currentValue = set value` (e.g., Lignify sets to 0/4)
- Characteristic defining: `currentValue = calculated value` (e.g., Tarmogoyf = # card types)

### 2. **tempBoost**: Continuous Effects from Static Abilities

This is calculated by `getTempPowerBoost()` (lines 4763-4772):

```java
public final int getTempPowerBoost() {
    int result = 0;
    for (Pair<Integer, Integer> pair : boostPT.values()) {
        if (pair.getLeft() != null) {
            result += pair.getLeft();  // ADD all boosts together
        }
    }
    return result;
}
```

**How Equipment adds to this** (from StaticAbilityContinuous.java:687-697):

```java
// In MODIFYPT layer
if (layer == StaticAbilityLayer.MODIFYPT) {
    if (params.containsKey("AddPower")) {
        powerBonus = AbilityUtils.calculateAmount(hostCard, addP, stAb, true);
    }
    if (params.containsKey("AddToughness")) {
        toughnessBonus = AbilityUtils.calculateAmount(hostCard, addT, stAb, true);
    }
    affectedCard.addPTBoost(powerBonus, toughnessBonus, se.getTimestamp(), stAb.getId());
}
```

**Examples**:
- Equipment (+2/+2): `tempBoost += 2`
- Enchantment (+1/+1): `tempBoost += 1`
- Anthem effect (+1/+1): `tempBoost += 1`
- **Multiple effects stack**: Spider-Suit (+2/+2) + another Equipment (+1/+1) = `tempBoost = 3`

### 3. **bonusFromCounters**: Physical Counters on the Card

This is calculated by `getPowerBonusFromCounters()` (lines 4670-4674):

```java
public final int getPowerBonusFromCounters() {
    return getCounters(CounterEnumType.P1P1) +      // +1/+1 counters
           getCounters(CounterEnumType.P1P2) +      // +1/+2 counters (power only)
           getCounters(CounterEnumType.P1P0) +      // +1/+0 counters
           2 * getCounters(CounterEnumType.P2P2) +  // +2/+2 counters (worth 2 power each)
           2 * getCounters(CounterEnumType.P2P0) +  // +2/+0 counters
           - getCounters(CounterEnumType.M1M1) +    // -1/-1 counters (subtract)
           - getCounters(CounterEnumType.M1M0) +    // -1/-0 counters
           - 2 * getCounters(CounterEnumType.M2M1) + // -2/-1 counters
           - 2 * getCounters(CounterEnumType.M2M2);  // -2/-2 counters
}
```

**Examples**:
- 3x +1/+1 counters: `bonusFromCounters = 3`
- 1x +2/+2 counter: `bonusFromCounters = 2`
- 2x +1/+1 and 1x -1/-1: `bonusFromCounters = 2 + (-1) = 1`

## Complete Example: Spider-Punk with Spider-Suit and Counters

```
Card: Spider-Punk
Printed P/T: 2/1

Modifications:
- Equipped with Spider-Suit (+2/+2)
- Has 1x +1/+1 counter
- Anthem effect from Honor of the Pure (+1/+1)

Calculation:
- currentValue:        2        (base power, no SETPT effects)
- tempBoost:           +3       (Spider-Suit +2, Anthem +1)
- bonusFromCounters:   +1       (one +1/+1 counter)
- TOTAL:               6 power

Breakdown: 2 (base) + 3 (continuous effects) + 1 (counters) = 6 power
```

## How This Relates to MTG Comprehensive Rules (CR 613)

The three components map to **CR 613's layer system**:

### Layer Order (simplified):

1. **Layer 7a (CHARACTERISTIC)**: Characteristic-defining abilities (e.g., Tarmogoyf)
   - Maps to: `currentValue` calculation

2. **Layer 7b (SETPT)**: Effects that set P/T to specific value (e.g., Lignify)
   - Maps to: `currentValue` calculation (replaces base)

3. **Layer 7c (MODIFYPT)**: Effects that modify P/T (e.g., Equipment, anthems)
   - Maps to: `tempBoost` calculation

4. **Layer 7d (COUNTERS)**: Counters that modify P/T
   - Maps to: `bonusFromCounters` calculation

**Key Rule**: Each layer is applied in order, and all effects within a layer are applied simultaneously (with timestamp ordering for conflicts).

## Rust Implementation Strategy

Our current Rust implementation (`get_effective_power()` in actions.rs:454-495) hardcodes Equipment buffs:

```rust
pub fn get_effective_power(&self, creature_id: CardId) -> Result<i32> {
    let creature = self.cards.get(creature_id)?;
    let mut power = creature.current_power() as i32;  // currentValue

    // Add Equipment buffs (tempBoost layer)
    let equipment_list = self.get_attached_equipment(creature_id);
    for equip_id in equipment_list {
        let equipment = self.cards.get(equip_id)?;
        if equipment.name.as_str().eq_ignore_ascii_case("Spider-Suit") {
            power += 2;  // Hardcoded for now
        }
    }

    Ok(power)
}
```

### To Match Java Forge, We Should Add:

1. **Counter support** (already exists in `Card.counters` field):
   ```rust
   // Add counter bonus
   let counter_bonus = creature.get_counter_pt_bonus();
   power += counter_bonus;
   ```

2. **Generic continuous effects** (Phase 3 - TODO):
   ```rust
   // Parse from card data: S:Mode$ Continuous | AddPower$ 2
   // Instead of hardcoding Spider-Suit, look up static abilities
   let continuous_effects = self.get_continuous_effects_for(creature_id);
   for effect in continuous_effects {
       if let ContinuousEffect::ModifyPT { power: bonus, .. } = effect {
           power += bonus;
       }
   }
   ```

3. **Layer system for complex interactions** (Future - Phase 3+):
   ```rust
   pub struct StatBreakdown {
       pub current_value: i32,      // Base after SETPT layer
       pub temp_boost: i32,          // MODIFYPT layer (Equipment, anthems)
       pub bonus_from_counters: i32, // COUNTERS layer
   }

   impl StatBreakdown {
       pub fn total(&self) -> i32 {
           self.current_value + self.temp_boost + self.bonus_from_counters
       }
   }
   ```

## Key Insights for Phase 3 Implementation

1. **Equipment effects are in the MODIFYPT layer** - they ADD to power/toughness, they don't SET it
2. **Counters are calculated separately** - they stack additively with continuous effects
3. **Multiple Equipment stack additively** - 2x Spider-Suit = +4/+4 (which our tests already verify)
4. **Order matters for complex effects**:
   - SETPT effects override base P/T
   - MODIFYPT effects (Equipment) apply to the current value
   - COUNTERS apply last
5. **Timestamps matter within layers** - if two Equipment give conflicting effects in the same layer, the one that entered/attached most recently takes precedence (though for additive effects like +2/+2, this doesn't matter)

## References

- Java Forge: `Card.java` lines 4640-4780 (P/T calculation)
- Java Forge: `StaticAbilityContinuous.java` lines 156-166, 688-697 (MODIFYPT layer)
- MTG Comprehensive Rules: Section 613 (Interaction of Continuous Effects)
- Our implementation: `mtg-engine/src/game/actions.rs` lines 454-495 (get_effective_power/toughness)
