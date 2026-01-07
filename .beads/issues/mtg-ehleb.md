---
title: Eliminate hacky substring operations on card scripts
status: open
priority: 3
issue_type: task
labels:
- tech-debt,parsing
created_at: 2026-01-07T15:25:44.667431794+00:00
updated_at: 2026-01-07T15:25:44.667431794+00:00
---

# Description

## Eliminate Hacky Substring Operations on Card Scripts

## Problem

We have 30+ instances of hacky `.contains()` calls on card script bodies in `card.rs` that bypass proper tokenized parsing:

```rust
// BAD - what we have now:
if body.contains("AB$ Draw") || body.contains("DB$ Draw") { ... }
if body.contains("DB$ PutCounter") { ... }
if body.contains("DB$ GainLife") { ... }
```

These are fragile because:
- `contains("Damage")` matches "DealDamage", "PreventDamage", "AllDamage"
- `contains("add")` would match "Madden", "adding"
- O(n) per call vs O(1) map lookup after tokenized parse

## Solution

We ALREADY have proper parsing infrastructure in `ability_parser.rs`:
- `AbilityParams::parse()` - tokenizes by `|` and `$`
- Returns structured data with `api_type: ApiType`, `params: HashMap`

Refactor all card script parsing in `card.rs` to use `AbilityParams::parse()` instead of raw substring matching.

## Audit Results (2026-01-07)

Locations needing refactoring in `mtg-engine/src/loader/card.rs`:
- Line 1502: `body.contains("DB$ Token")`
- Line 1540: `body.contains("DB$ ChangeZone")`
- Line 1569-1570: `body.contains("AB$ Draw")`, `body.contains("DB$ Draw")`, `body.contains("Cost$ Discard")`
- Line 1612: `body.contains("DB$ Earthbend")`
- Line 1674: `body.contains("DB$ Mana")`
- Line 1773: `body.contains("DB$ DealDamage")`
- Line 1809: `body.contains("DB$ GainLife")`
- Line 1844: `body.contains("DB$ Earthbend")`
- Line 1914: `body.contains("AB$ Draw")` or `body.contains("DB$ Draw")`
- Line 1952: `body.contains("DB$ PutCounter")`
- Line 1977: `body.contains("DB$ PutCounter")`
- Line 1999: `body.contains("DB$ GainLife")`
- Line 2018: `body.contains("DB$ DealDamage")`
- Line 2087: `body.contains("DB$ PutCounter")`
- Line 2107: `body.contains("DB$ Pump")`
- Line 2844: `ability.contains("Mode$ Continuous")`

Total: ~20+ locations in card.rs alone

Also in `effect_converter.rs`:
- Line 359: `static_lower.contains("unblock")` or `static_lower.contains("cantblock")`

## References

- Coding principle: CLAUDE.md "NO HACKY STRING OPERATIONS ON STRUCTURED DATA"
- Analysis doc: `ai_docs/ability_parsing_comparison.md`
- Card script spec: `ai_docs/CARD_SCRIPT_SPEC.md`
- Proper parser: `mtg-engine/src/loader/ability_parser.rs`
