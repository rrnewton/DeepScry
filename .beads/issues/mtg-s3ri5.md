---
title: Remove engine dependence on card text (use structured ability data)
status: open
priority: 1
issue_type: task
created_at: 2025-11-30T13:26:55.601424949+00:00
updated_at: 2025-11-30T13:37:05.073069903+00:00
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

## Progress

### ✅ Phase 1: CardCache mana production - COMPLETED (928a862)

Commit 928a862 removed CardCache's dependence on oracle text:
- Removed 12 unused text-derived fields
- Added `derive_mana_production_from_abilities()` to scan ActivatedAbility entries
- Added `update_from_abilities_with_name()` with fallback for test cards
- Card loader now calls cache update AFTER parsing abilities

### 🔲 Phase 2: Fix runtime text parsing - REMAINING

Two locations in actions/mod.rs still grep card text at runtime:

#### 2a. activate_mana_ability() at line ~1378
```rust
let card_text = self.cards.get(card_id)?.text.to_lowercase();
let is_any_color = card_text.contains("any color");
```
**Fix**: Use `card.cache.mana_production.kind` which already has `AnyColor` variant.

#### 2b. Land mana activation at line ~1498-1509
```rust
let text_lower = card.text.to_lowercase();
text_lower.contains("any color"),
text_lower.contains("add {c}") || card_name.eq_ignore_ascii_case("wastes"),
```
**Fix**: Use the pre-computed `card.cache.mana_production` field.

### 🔲 Phase 3: Clean up - REMAINING

1. Remove any remaining `card.text` usage in game logic
2. Consider removing `card.text` field entirely (keep only for display in TUI)
3. AbilityCache targeting hints could be derived from ValidTgts$ instead of SpellDescription$

## Java Forge Approach (Reference)

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

## NOT a Problem

- `Cost::parse()` in costs.rs parses structured `Cost$` notation - this is correct
- AbilityCache parses `SpellDescription$` for AI hints - lower priority
