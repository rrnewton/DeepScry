# Strongly-Typed Keyword Arguments Design

## Overview

Refactor keyword system to have:
- Single `Keyword` enum with ALL keywords (no arguments)
- Separate `KeywordArgs` enum with strongly-typed fields
- `KeywordSet` using `EnumSet<Keyword>` + `SmallVec<KeywordArgs, 2>` for args

## Architecture

### Before (Current)
```rust
enum KeywordSimple { Flying, Haste, ... }  // 92 simple keywords
enum KeywordComplex {
    Madness(String),    // "1 R" as string
    Flashback(String),  // "3 R" as string
    ...
}

struct KeywordSet {
    simple: EnumSet<KeywordSimple>,
    complex: Vec<KeywordComplex>,
}
```

### After (Target)
```rust
enum Keyword {
    Flying, Haste, ...  // All 92 simple keywords
    Madness, Flashback, Kicker, ...  // All complex keywords (no args)
}

enum KeywordArgs {
    Madness { cost: ManaCost },
    Flashback { cost: ManaCost },
    Kicker { cost: ManaCost },
    Cycling { cost: ManaCost },
    Equip { cost: ManaCost },
    Morph { cost: ManaCost },
    Evoke { cost: ManaCost },
    Buyback { cost: ManaCost },
    Echo { cost: ManaCost },
    Suspend { time_counters: u8, cost: ManaCost },

    // Type-based
    Enchant { card_type: Subtype },  // "Creature", "Artifact", etc.
    Landwalk { land_type: Subtype },  // "Island", "Swamp", etc.
    Affinity { card_type: Subtype },  // "Artifact", "Island", etc.
    Protection { from: Subtype },     // "Red", "Artifacts", "Dragons", etc.
    Offering { creature_type: Subtype },  // "Spirit", "Goblin", etc.
    Champion { creature_type: Subtype },  // "Goblin", "Elf", etc.

    // Amount-based
    Amplify { amount: u8, creature_type: Subtype },
    Annihilator { amount: u8 },
    Bushido { amount: u8 },
    Fading { counters: u8 },
    Vanishing { counters: u8 },
    Dredge { amount: u8 },
    Modular { counters: u8 },
    Absorb { amount: u8 },

    // String-based (not yet fully typed)
    HexproofFrom { from: String },  // TODO: parse into Color | CardType
    PartnerWith { card_name: CardName },
    Companion { restriction: String },  // TODO: parse restriction
}

struct KeywordSet {
    keywords: EnumSet<Keyword>,  // O(1) membership for ALL keywords
    args: SmallVec<[KeywordArgs; 2]>,  // Strongly-typed arguments
}

impl Keyword {
    pub fn is_complex(&self) -> bool {
        matches!(self, Keyword::Madness | Keyword::Flashback | ...)
    }
}

impl KeywordSet {
    pub fn get_args(&self, keyword: Keyword) -> Option<&KeywordArgs> {
        // Find args for this keyword (requires keyword.is_complex() == true)
    }
}
```

## Field Types

| Keyword | Fields | Types |
|---------|--------|-------|
| Madness | cost | ManaCost |
| Flashback | cost | ManaCost |
| Kicker | cost | ManaCost |
| Cycling | cost | ManaCost |
| Equip | cost | ManaCost |
| Morph | cost | ManaCost |
| Evoke | cost | ManaCost |
| Buyback | cost | ManaCost |
| Echo | cost | ManaCost |
| Suspend | time_counters, cost | u8, ManaCost |
| Enchant | card_type | Subtype |
| Landwalk | land_type | Subtype |
| Affinity | card_type | Subtype |
| Protection | from | Subtype |
| Offering | creature_type | Subtype |
| Champion | creature_type | Subtype |
| Amplify | amount, creature_type | u8, Subtype |
| Annihilator | amount | u8 |
| Bushido | amount | u8 |
| Fading | counters | u8 |
| Vanishing | counters | u8 |
| Dredge | amount | u8 |
| Modular | counters | u8 |
| Absorb | amount | u8 |
| HexproofFrom | from | String (TODO: Color \| Subtype) |
| PartnerWith | card_name | CardName |
| Companion | restriction | String (TODO: parse) |

## Benefits

1. **Type Safety**: Can't mix up cost vs amount vs card type
2. **O(1) Membership**: `has(Keyword::Madness)` is bitset check, not vector scan
3. **Efficient Args Lookup**: SmallVec avoids heap allocation for ≤2 complex keywords (common case)
4. **Clear API**: `get_args(Keyword::Madness)` returns `Option<&KeywordArgs::Madness>`
5. **Parsing at Load Time**: Convert strings → typed fields once during card load

## Migration Path

1. Create new `Keyword` enum (all keywords, no args)
2. Create `KeywordArgs` enum with strongly-typed fields
3. Update `KeywordSet` to use new types
4. Add `is_complex()` and `get_args()` methods
5. Update card loader to parse args into typed fields
6. Update all usages (replace `KeywordSimple`/`KeywordComplex` with new API)
7. Delete old enums
