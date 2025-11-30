---
title: Remove engine dependence on card text (use structured ability data)
status: open
priority: 1
issue_type: task
created_at: 2025-11-30T13:26:55.601424949+00:00
updated_at: 2025-11-30T13:28:38.286960210+00:00
---

# Description

## Remove Engine Dependence on Card Text

## Problem

The Rust engine currently parses oracle text to determine card capabilities (mana production, keywords, etc.)
when this information is already available in structured form from the card files' ability notation.

Java Forge card files have explicit structured notation:
```
A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}.
```

The `Produced$` parameter explicitly declares mana production. We should NEVER need to grep card text.

## Java Forge Approach (THE RIGHT WAY)

Java Forge's `AbilityManaPart.java` stores `origProduced` from the `Produced$` parameter.
The `canProduce()` method checks this structured data:

```java
public final boolean canProduce(final String s, final SpellAbility sa) {
    if (isAnyMana() && !s.equals("C")) {
        return true;
    }
    return mana(sa).contains(s);  // Uses origProduced, not card text!
}
```

Cards expose `getManaAbilities()` which returns parsed `SpellAbility` objects with all mana
information derived from the structured `Produced$`, `Amount$`, `Combo`, etc. parameters.

## Current Rust State

The Rust loader ALREADY parses `Produced$` correctly in `effect_converter.rs:150-185`:
```rust
ApiType::Mana => {
    let produced_str = params.get("Produced")?;
    // ...properly parses all cases
}
```

But several places ignore the parsed abilities and re-derive info from oracle text.

## Occurrences to Fix

### 1. CardCache mana production - PRIORITY (mtg-engine/src/core/card.rs)

**Location**: `compute_mana_production()` at line 158

**Problem**: Greps text for "{t}: add {X}" patterns instead of using parsed abilities:
```rust
if text_lower.contains("any color") { ... }
if text_lower.contains("{t}: add {w}") || text_lower.contains("add {w}") { ... }
```

**Also stores redundant text-derived flags** (lines 124-135):
- `text_contains_add`, `text_contains_tap_colon`, `text_contains_mana`, `text_contains_any_color`
- `text_produces_white/blue/black/red/green/colorless`
- `text_lowercase` (full lowercase copy of oracle text!)

**Fix**: Derive `ManaProduction` from `ActivatedAbility` entries with `is_mana_ability = true`.
Set cache fields after abilities are parsed, not from text.

### 2. Mana ability activation - actions/mod.rs:1378

**Location**: `activate_mana_ability()` method

**Problem**: Re-greps text at runtime to check "any color":
```rust
let card_text = self.cards.get(card_id)?.text.to_lowercase();
let is_any_color = card_text.contains("any color");
```

**Fix**: Use `card.cache.mana_production.kind` which should already have `AnyColor` variant.

### 3. Land mana activation - actions/mod.rs:1498-1509

**Location**: Land tapping for mana

**Problem**: Re-greps text at runtime for colorless/any detection:
```rust
let text_lower = card.text.to_lowercase();
text_lower.contains("any color"),
text_lower.contains("add {c}") || card_name.eq_ignore_ascii_case("wastes"),
```

**Fix**: Use the pre-computed `card.cache.mana_production` field.

### 4. AbilityCache in effects.rs:306-310

**Location**: `AbilityCache::new()` for targeting info

**Current behavior**: Parses `SpellDescription$` for targeting hints:
```rust
targets_tapped: desc_lower.contains("tapped"),
targets_untapped: desc_lower.contains("untapped"),
```

**Assessment**: This is LESS critical because:
- It parses `SpellDescription$` (structured field), not oracle text
- These are hints for AI, not game rules
- BUT: Still could be derived from `ValidTgts$` parameter

### 5. NOT a problem: costs.rs

The `Cost::parse()` function parses the structured `Cost$` string from card files.
This is the correct approach - parsing structured notation, not oracle text.

## Implementation Plan

### Phase 1: Fix CardCache (this issue)
1. Remove `compute_mana_production()` function
2. Add `CardCache::set_mana_production(production: ManaProduction)` method  
3. In card loader, after parsing abilities, derive ManaProduction from mana abilities
4. Remove redundant `text_produces_*` and `text_contains_*` fields
5. Keep `text_lowercase` temporarily for any remaining uses

### Phase 2: Fix runtime text parsing
1. Update `activate_mana_ability()` to use cache
2. Update land mana activation to use cache
3. Remove any remaining `card.text` usage in game logic

### Phase 3: Clean up
1. Remove `text_lowercase` from CardCache
2. Consider removing `card.text` field entirely (keep only for display)
3. Ensure all game logic uses structured data

## Testing
- All existing tests should continue to pass
- Cards with "any color" mana (Birds of Paradise, etc.) should work
- Dual lands should work
- Colorless sources (Wastes, Sol Ring) should work
