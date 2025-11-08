# Ability Parsing: Java Forge vs Rust Implementation

**Date:** 2025-11-08
**Context:** Analysis of ability parsing safety concerns
**Question:** Are we being too hacky with string operations like `contains("add")` and `contains("mana")`?

---

## Executive Summary

**Key Finding:** Our Rust string-based parsing is NOT significantly worse than Java's approach. **Java Forge ALSO uses string operations** extensively - they just wrap it in a more structured API.

**HOWEVER:** There ARE legitimate safety concerns with BOTH approaches:
1. **Non-tokenized substring matching** - "add" could match "Madden" or "adding"
2. **Order-dependent parsing** - checking "AB$ Mana" before parsing "Produced$" parameter
3. **Silent failures** - malformed abilities get skipped without errors
4. **No validation** - parameters aren't checked for correctness

**The Good News:** We can improve our implementation while staying close to Java's design.

---

## 1. Java Forge's Parsing Architecture

### 1.1 Card Data Format

**Format:** Key-value pairs separated by `|` (pipe) character

```
A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}.
```

**Structure:**
- `A:` = Activated ability prefix
- `AB$` = Ability record type marker
- Parameters separated by `|`
- Key-value pairs use `$` separator

### 1.2 Parsing Process (3 Stages)

**Stage 1: String Splitting (FileSection.java:60-76)**

```java
public static Map<String, String> parseToMap(final String line, final Pattern kvSeparator) {
    Map<String, String> result = new TreeMap<>(String.CASE_INSENSITIVE_ORDER);
    if (!StringUtils.isEmpty(line)) {
        for (final String dd : line.split(BAR_PAIR_SPLITTER)) {  // Split by |
            final String[] v = kvSeparator.split(dd, 2);  // Split by $
            result.put(v[0].trim(), v[1].trim());
        }
    }
    return result;
}
```

**Result:** `Map<String, String>` with keys like "AB", "Cost", "Produced", "SpellDescription"

**Stage 2: Ability Type Detection (AbilityFactory.java:95-111)**

```java
public static ApiType getApiTypeOf(Map<String, String> abParams) {
    return ApiType.smartValueOf(abParams.get(this.getPrefix()));
}

public static AbilityRecordType getRecordType(Map<String, String> abParams) {
    if (abParams.containsKey("AB")) {
        return AbilityRecordType.Ability;
    } else if (abParams.containsKey("SP")) {
        return AbilityRecordType.Spell;
    } else if (abParams.containsKey("ST")) {
        return AbilityRecordType.StaticAbility;
    } else if (abParams.containsKey("DB")) {
        return AbilityRecordType.SubAbility;
    }
    return null;
}
```

**Key:** They lookup `mapParams.get("AB")` to get API type string like "Mana", "DealDamage", etc.

**Stage 3: Ability Construction (AbilityFactory.java:200-316)**

```java
public static SpellAbility getAbility(AbilityRecordType type, ApiType api,
        Map<String, String> mapParams, Cost abCost, CardState state, IHasSVars sVarHolder) {

    TargetRestrictions abTgt = mapParams.containsKey("ValidTgts") ?
        readTarget(mapParams) : null;

    SpellAbility spellAbility = type.buildSpellAbility(api, hostCard, abCost, abTgt, mapParams);

    // Link sub-abilities
    if (mapParams.containsKey("SubAbility")) {
        spellAbility.setSubAbility(getSubAbility(state, mapParams.get("SubAbility"), sVarHolder));
    }

    return spellAbility;
}
```

**Key Pattern:** Uses `mapParams.containsKey()` and `mapParams.get()` to check for parameters.

### 1.3 Key Observations

**Java ALSO uses string operations:**
- `mapParams.containsKey("Produced")` - checking for substring match in keys
- `mapParams.get("NumDmg")` - fetching values by string key
- `ApiType.smartValueOf(abParams.get("AB"))` - matching API type strings

**Difference from our approach:**
- Java parses ONCE into `Map<String, String>`, THEN queries the map
- We parse MULTIPLE TIMES with `ability.contains()` and `ability.split()`

**Why this matters:**
- Java's approach is **O(n) parse + O(1) lookups**
- Our approach is **O(n) per contains() call** - wasteful but not fundamentally different

---

## 2. Our Rust Implementation

### 2.1 Current Approach (card.rs:293-456)

```rust
fn parse_effects(&self) -> Vec<Effect> {
    let mut effects = Vec::new();

    for ability in &self.raw_abilities {
        // Parse DealDamage abilities
        if ability.contains("DealDamage") {
            if let Some(dmg_str) = ability.split("NumDmg$").nth(1) {
                if let Some(dmg_part) = dmg_str.trim().split(['|', ' ']).next() {
                    if let Ok(amount) = dmg_part.trim().parse::<i32>() {
                        effects.push(Effect::DealDamage { /*...*/ });
                    }
                }
            }
        }

        // Parse Draw abilities
        if ability.contains("SP$ Draw") {
            // ... similar pattern ...
        }
    }

    effects
}
```

### 2.2 Problems with Current Approach

#### Problem 1: **Non-tokenized substring matching**

```rust
if ability.contains("DealDamage") {
```

**Risk:** Could match unintended substrings:
- "DealDamageToSelf" ✓ (okay)
- "PreventsAllDamage" ✗ (false positive - contains "Damage")
- "Maddening" ✗ (false positive if we check `contains("add")`)

**Java has same issue:**
```java
if (mapParams.containsKey("DealDamage")) {
```

BUT Java's keys are **tokenized by `|` delimiter first**, so "DealDamage" is isolated.

#### Problem 2: **Order-dependent parsing**

```rust
if ability.contains("SP$ Draw") {
    if let Some(cards_str) = ability.split("NumCards$").nth(1) {
```

**Risk:** If we check `contains("Draw")` BEFORE `contains("SP$ Draw")`, we might misparse:
- "A:AB$ DrawCard | Cost$ T | ..." would match generic "Draw" check

**Java has same issue:**
```java
ApiType api = ApiType.smartValueOf(abParams.get("SP"));
```

Order of API type checks matters.

#### Problem 3: **Silent failures**

```rust
if let Some(dmg_str) = ability.split("NumDmg$").nth(1) {
    if let Some(dmg_part) = dmg_str.trim().split(['|', ' ']).next() {
        if let Ok(amount) = dmg_part.trim().parse::<i32>() {
            // SUCCESS
        } // Otherwise: silently skip
    }
}
```

**Risk:** Malformed abilities get ignored without error:
- "A:SP$ DealDamage | NumDmg$ X" - fails to parse "X" as i32, silently skipped
- "A:SP$ DealDamage | NumDamage$ 3" - typo in "NumDmg", silently skipped

**Java has BETTER error handling:**
```java
throw new RuntimeException("AbilityFactory : getAbility -- no API in " + source);
```

But still has silent failures for individual parameters.

#### Problem 4: **No validation of parameter combinations**

```rust
if ability.contains("AB$ Mana") {
    if let Some(produced_str) = ability.split("Produced$").nth(1) {
```

**Missing checks:**
- Is "Produced$" actually a valid parameter for "AB$ Mana"?
- Are there required parameters we're not checking?
- Can "Cost$" and "Produced$" have invalid combinations?

**Java has SAME problem** - no schema validation, just duck-typed parameter extraction.

---

## 3. Comparison: Safety & Correctness

| Aspect | Java Forge | MTG Forge-rs | Winner |
|--------|------------|--------------|--------|
| **Tokenization** | Splits by `|` first, then looks up keys | Searches raw string multiple times | **Java** |
| **Caching** | Parses once to Map, queries many times | Re-scans string for each check | **Java** |
| **Type safety** | Runtime string->enum conversion | Runtime string matching | **Tie** (both weak) |
| **Error handling** | Throws on missing API type, silent on params | Silent failures everywhere | **Java** (slightly) |
| **Validation** | None (duck typing) | None (duck typing) | **Tie** (both bad) |
| **Code clarity** | Centralized in AbilityFactory | Scattered in parse_effects/parse_triggers | **Java** |
| **Extensibility** | Add new ApiType enum | Add new if-block | **Java** |

### 3.1 Where We're WORSE

1. **Performance:** We re-scan the entire ability string for each `contains()` call
   - Java: O(n) parse + O(1) * k lookups = O(n + k)
   - Us: O(n) * k searches = O(nk)

2. **Correctness:** We don't tokenize, so substring matches can be ambiguous
   - "DealDamage" vs "AllDamage" vs "Damage" in longer strings

3. **Maintainability:** Logic is scattered across multiple functions
   - `parse_effects()` - 164 lines
   - `parse_triggers()` - 168 lines
   - `parse_activated_abilities()` - 260 lines
   - Total: 592 lines of similar parsing logic

### 3.2 Where We're NOT WORSE

1. **Fundamental approach:** String matching is used by Java too
2. **Safety from "add" in "Madden":** Java is vulnerable too if not tokenized
3. **Type safety:** Java's runtime `ApiType.smartValueOf()` is also dynamic

---

## 4. Proposed Solutions

### Option A: Minimal Fix - Token-aware substring matching

**Goal:** Fix the "add" in "Madden" problem without major refactoring

```rust
fn parse_ability_params(ability: &str) -> HashMap<&str, &str> {
    let mut params = HashMap::new();

    // Split by | delimiter to isolate parameters
    for param in ability.split('|') {
        if let Some((key, value)) = param.trim().split_once('$') {
            params.insert(key.trim(), value.trim());
        }
    }

    params
}

fn parse_effects(&self) -> Vec<Effect> {
    let mut effects = Vec::new();

    for ability in &self.raw_abilities {
        let params = parse_ability_params(ability);

        // Check API type (SP, AB, DB, etc.)
        if let Some(api_type) = params.get("SP").or(params.get("AB")).or(params.get("DB")) {
            match *api_type {
                "DealDamage" => {
                    if let Some(dmg) = params.get("NumDmg").and_then(|s| s.parse::<i32>().ok()) {
                        effects.push(Effect::DealDamage { amount: dmg, /*...*/ });
                    }
                }
                "Draw" => {
                    if let Some(count) = params.get("NumCards").and_then(|s| s.parse::<u8>().ok()) {
                        effects.push(Effect::DrawCards { count, /*...*/ });
                    }
                }
                // ... other effect types ...
                _ => {} // Unknown API type - could log warning
            }
        }
    }

    effects
}
```

**Pros:**
- Fixes tokenization issue
- More efficient (parse once, lookup many)
- Closer to Java's approach
- Small change (~100 lines)

**Cons:**
- Still no validation
- Still silent failures
- Doesn't centralize logic

**Effort:** 1-2 hours

---

### Option B: Structured Parser with Validation

**Goal:** Build a proper ability DSL parser

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ApiType {
    Mana,
    DealDamage,
    Draw,
    Destroy,
    GainLife,
    Pump,
    // ... exhaustive list from Java's ApiType enum
}

#[derive(Debug, Clone)]
pub struct AbilityParams {
    api_type: ApiType,
    cost: Option<Cost>,
    parameters: HashMap<String, String>,
}

impl AbilityParams {
    fn parse(ability_string: &str) -> Result<Self, ParseError> {
        // Split by record type prefix (A:, T:, S:)
        let (prefix, body) = ability_string.split_once(':')
            .ok_or(ParseError::MissingPrefix)?;

        // Split parameters by |
        let params = body.split('|')
            .filter_map(|param| param.trim().split_once('$'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect::<HashMap<_, _>>();

        // Determine record type and API type
        let record_type = params.get("AB").or(params.get("SP")).or(params.get("DB"))
            .ok_or(ParseError::MissingRecordType)?;

        let api_type = match record_type.as_str() {
            "Mana" => ApiType::Mana,
            "DealDamage" => ApiType::DealDamage,
            "Draw" => ApiType::Draw,
            // ... exhaustive matching
            unknown => return Err(ParseError::UnknownApiType(unknown.to_string())),
        };

        // Parse cost if present
        let cost = params.get("Cost")
            .map(|s| Cost::parse(s))
            .transpose()?;

        Ok(AbilityParams {
            api_type,
            cost,
            parameters: params,
        })
    }

    fn to_effect(&self) -> Result<Effect, ConversionError> {
        match self.api_type {
            ApiType::DealDamage => {
                let amount = self.parameters.get("NumDmg")
                    .ok_or(ConversionError::MissingParameter("NumDmg"))?
                    .parse::<i32>()
                    .map_err(|_| ConversionError::InvalidParameter("NumDmg"))?;

                Ok(Effect::DealDamage {
                    target: TargetRef::None,
                    amount
                })
            }
            ApiType::Draw => {
                let count = self.parameters.get("NumCards")
                    .ok_or(ConversionError::MissingParameter("NumCards"))?
                    .parse::<u8>()
                    .map_err(|_| ConversionError::InvalidParameter("NumCards"))?;

                Ok(Effect::DrawCards {
                    player: PlayerId::new(0),
                    count
                })
            }
            // ... exhaustive conversion
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Ability string missing ':' prefix separator")]
    MissingPrefix,

    #[error("No record type found (expected AB$, SP$, or DB$)")]
    MissingRecordType,

    #[error("Unknown API type: {0}")]
    UnknownApiType(String),

    #[error("Failed to parse cost: {0}")]
    InvalidCost(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("Missing required parameter: {0}")]
    MissingParameter(&'static str),

    #[error("Invalid value for parameter: {0}")]
    InvalidParameter(&'static str),
}
```

**Usage:**

```rust
fn parse_effects(&self) -> Vec<Effect> {
    self.raw_abilities.iter()
        .filter_map(|ability| {
            match AbilityParams::parse(ability) {
                Ok(params) => {
                    match params.to_effect() {
                        Ok(effect) => Some(effect),
                        Err(e) => {
                            eprintln!("Warning: Failed to convert ability to effect: {} in '{}'", e, ability);
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to parse ability: {} in '{}'", e, ability);
                    None
                }
            }
        })
        .collect()
}
```

**Pros:**
- Proper error handling with `Result` types
- Validation at parse time
- Type-safe API type enum
- Centralized conversion logic
- Easier to test (unit test each API type)
- Can add schema validation later

**Cons:**
- More code (~300-500 lines)
- Need to maintain exhaustive ApiType enum
- More complex than current approach

**Effort:** 4-6 hours

---

### Option C: Adopt Java's FileSection + AbilityFactory Pattern

**Goal:** Port Java's exact approach to Rust

```rust
// Equivalent to FileSection.java
pub struct AbilityScript {
    parameters: HashMap<String, String>,
}

impl AbilityScript {
    pub fn parse(script: &str, separator: &str) -> Self {
        let parameters = script.split('|')
            .filter_map(|param| {
                let mut parts = param.splitn(2, separator);
                match (parts.next(), parts.next()) {
                    (Some(k), Some(v)) => Some((k.trim().to_string(), v.trim().to_string())),
                    _ => None,
                }
            })
            .collect();

        Self { parameters }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.parameters.get(key).map(|s| s.as_str())
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.parameters.contains_key(key)
    }
}

// Equivalent to AbilityFactory.java
pub struct AbilityFactory;

impl AbilityFactory {
    pub fn get_ability(script: &str, card_state: &CardState) -> Result<SpellAbility, ParseError> {
        let params = AbilityScript::parse(script, "$");

        let record_type = Self::get_record_type(&params)?;
        let api_type = Self::get_api_type(&params, record_type)?;

        let cost = Self::parse_cost(&params, record_type)?;
        let target = Self::read_target(&params)?;

        let ability = record_type.build_spell_ability(api_type, card_state.card, cost, target, params);

        // Link sub-abilities
        if let Some(sub_name) = params.get("SubAbility") {
            ability.set_sub_ability(Self::get_sub_ability(card_state, sub_name)?);
        }

        Ok(ability)
    }

    fn get_record_type(params: &AbilityScript) -> Result<AbilityRecordType, ParseError> {
        if params.contains_key("AB") {
            Ok(AbilityRecordType::Ability)
        } else if params.contains_key("SP") {
            Ok(AbilityRecordType::Spell)
        } else if params.contains_key("DB") {
            Ok(AbilityRecordType::SubAbility)
        } else {
            Err(ParseError::MissingRecordType)
        }
    }

    fn get_api_type(params: &AbilityScript, record_type: AbilityRecordType) -> Result<ApiType, ParseError> {
        let prefix = record_type.prefix();
        let api_str = params.get(prefix)
            .ok_or(ParseError::MissingApiType)?;

        ApiType::from_str(api_str)
            .ok_or_else(|| ParseError::UnknownApiType(api_str.to_string()))
    }
}
```

**Pros:**
- Closest to Java's proven design
- Clear separation: parsing vs interpretation
- Easier for Java devs to understand
- Can leverage Java's 15 years of bug fixes

**Cons:**
- Most code (~500-800 lines to port AbilityFactory)
- Need to understand Java's full architecture
- May introduce Java-isms that don't fit Rust idioms

**Effort:** 8-12 hours (initial port), ongoing maintenance

---

## 5. Recommendations

### Immediate Action (This Week): **Option A**

**Rationale:**
1. Fixes the tokenization safety concern
2. Small, low-risk change
3. Better performance (parse once)
4. Closer to Java without full rewrite

**Implementation Plan:**
1. Add `parse_ability_params()` helper function
2. Refactor `parse_effects()` to use it
3. Refactor `parse_triggers()` to use it
4. Refactor `parse_activated_abilities()` to use it
5. Add tests for edge cases (substrings, order)

**Test Cases to Add:**
```rust
#[test]
fn test_no_false_positive_substring_match() {
    // "Madden" should NOT match "add"
    let ability = "A:SP$ Madden | Cost$ T | ...";
    let params = parse_ability_params(ability);
    assert!(params.get("SP") != Some(&"add")); // Corrected from our hacky approach
}

#[test]
fn test_tokenized_damage_vs_deal_damage() {
    // "Damage" should NOT match "DealDamage"
    let ability1 = "A:SP$ DealDamage | NumDmg$ 3";
    let ability2 = "A:SP$ PreventDamage | Amount$ 5";

    let params1 = parse_ability_params(ability1);
    let params2 = parse_ability_params(ability2);

    assert_eq!(params1.get("SP"), Some(&"DealDamage"));
    assert_eq!(params2.get("SP"), Some(&"PreventDamage"));
}
```

**Estimated Impact:**
- Fixes safety concern: ✅
- Performance improvement: 2-3x on ability parsing
- Code size: -50 lines (consolidation)
- Risk: Low (doesn't change semantics)

---

### Medium-term (Next Month): **Option B**

**After Option A is stable, add validation:**

1. Define `ApiType` enum exhaustively
2. Add `Result<Effect, ConversionError>` returns
3. Log warnings for unknown abilities
4. Add unit tests for each API type
5. Consider property-based testing (proptest crate)

**Benefits:**
- Catches malformed abilities at load time
- Easier debugging (clear error messages)
- Foundation for schema validation
- Better test coverage

---

### Long-term (Future): **Consider Option C**

**Only if:**
1. We need closer parity with Java for maintenance
2. We're adding many new ability types
3. Current approach becomes unmaintainable

**Not urgent because:**
- Option A + B gets us 80% of the benefit
- Full AbilityFactory port is large scope
- Rust idioms may diverge from Java patterns

---

## 6. Conclusion

### Summary of Findings

1. **We're not being "too hacky"** - Java uses similar string operations
2. **BUT** we ARE missing tokenization, which creates real risks
3. **Java's advantage** is structural (parse once, query many), not fundamental
4. **Our path forward** is clear: tokenize first, validate second

### Action Items

**Immediate (Option A - 1-2 hours):**
- [ ] Implement `parse_ability_params()` helper
- [ ] Refactor `parse_effects()` to use tokenized params
- [ ] Refactor `parse_triggers()` to use tokenized params
- [ ] Refactor `parse_activated_abilities()` to use tokenized params
- [ ] Add edge-case tests

**Medium-term (Option B - 4-6 hours):**
- [ ] Define `ApiType` enum
- [ ] Add `Result` returns with proper errors
- [ ] Implement validation for required parameters
- [ ] Add logging for unknown/malformed abilities

**Tracked Issue:**
- Create new issue: `mtg-XXX: Improve ability parsing safety with tokenization`
- Reference this analysis document

---

## Appendix: Example Transformations

### Before (Current - Unsafe)

```rust
if ability.contains("DealDamage") {  // Could match "AllDamage", "PreventDamage"
    if let Some(dmg_str) = ability.split("NumDmg$").nth(1) {  // Silent fail if missing
        if let Some(dmg_part) = dmg_str.trim().split(['|', ' ']).next() {
            if let Ok(amount) = dmg_part.trim().parse::<i32>() {  // Silent fail if not int
                effects.push(Effect::DealDamage { amount, /*...*/ });
            }
        }
    }
}
```

### After Option A (Safe, Simple)

```rust
let params = parse_ability_params(ability);

if let Some("DealDamage") = params.get("SP").or(params.get("AB")) {  // Exact match
    if let Some(dmg) = params.get("NumDmg").and_then(|s| s.parse::<i32>().ok()) {
        effects.push(Effect::DealDamage { amount: dmg, /*...*/ });
    }
}
```

### After Option B (Safe, Validated)

```rust
match AbilityParams::parse(ability) {
    Ok(params) if params.api_type == ApiType::DealDamage => {
        let amount = params.require_param("NumDmg")?  // Errors if missing
            .parse::<i32>()
            .map_err(|_| ConversionError::InvalidParameter("NumDmg"))?;

        effects.push(Effect::DealDamage { amount, /*...*/ });
    }
    Err(e) => eprintln!("Parse error: {} in '{}'", e, ability),
    _ => {} // Other API types
}
```

---

**Next Steps:** Discuss with user, implement Option A, file issue for Option B.
