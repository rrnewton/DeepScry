---
title: Remove engine dependence on card text (use structured ability data)
status: open
priority: 1
issue_type: task
created_at: 2025-11-30T13:26:55.601424949+00:00
updated_at: 2025-11-30T13:43:56.903642525+00:00
---

# Description

## Remove Engine Dependence on Card Text

## Problem

The Rust engine was parsing oracle text to determine card capabilities (mana production)
when this information is already available from structured ability data (`Produced$` parameter).

## Progress

### ✅ Phase 1: CardCache mana production - COMPLETED (928a862)

Removed CardCache's dependence on oracle text:
- Removed 12 unused text-derived fields
- Added `derive_mana_production_from_abilities()` to scan ActivatedAbility entries
- Card loader now calls cache update AFTER parsing abilities

### ✅ Phase 2: Runtime text parsing - COMPLETED (a615482)

Removed runtime text parsing in game/actions/mod.rs:

**activate_mana_ability() at ~line 1378:**
- Before: `card_text.to_lowercase().contains("any color")`
- After: `matches!(card.cache.mana_production.kind, ManaProductionKind::AnyColor)`

**Land mana activation at ~line 1500:**
- Before: `text_lower.contains("any color")` and `text_lower.contains("add {c}")`
- After: Uses `card.cache.mana_production.kind` for AnyColor/Colorless detection

### 🔲 Phase 3: Clean up - OPTIONAL

Lower priority remaining items:
1. AbilityCache targeting hints could be derived from ValidTgts$ instead of SpellDescription$
2. Consider removing `card.text` field entirely (keep only for TUI display)

## Summary

The main mana production path now uses structured ability data throughout:
- CardCache derives from ActivatedAbility effects
- ManaEngine uses CardCache
- ManaSourceCache uses CardCache  
- Runtime mana activation uses CardCache

No more `card.text.to_lowercase().contains(...)` calls in the mana path!

## Java Forge Approach (Reference)

Java Forge's `AbilityManaPart.java` stores `origProduced` from the `Produced$` parameter.
We now follow the same pattern via `derive_mana_production_from_abilities()`.
