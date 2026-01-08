---
title: Eliminate hacky substring operations on card scripts
status: open
priority: 3
issue_type: task
labels:
- tech-debt,parsing
created_at: 2026-01-07T15:25:44.667431794+00:00
updated_at: 2026-01-08T15:30:49.642026324+00:00
---

# Description

## Eliminate Hacky Substring Operations on Card Scripts

## Problem

We have 17+ instances of hacky `.contains()` calls on card script bodies in `card.rs` that bypass proper tokenized parsing:

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

## Audit Results (2026-01-08_#1580)

Locations needing refactoring in `mtg-engine/src/loader/card.rs`:

### extract_token_scripts() - Line 285
- `ability.contains("DB$ Token")`

### ETB Triggers (ChangesZone) - Lines 1522-1864
- Line 1522: `body.contains("DB$ Token")`
- Line 1560: `body.contains("DB$ ChangeZone")`
- Line 1589: `body.contains("AB$ Draw")` or `body.contains("DB$ Draw")`
- Line 1632: `body.contains("DB$ Earthbend")`
- Line 1694: `body.contains("DB$ Mana")`
- Line 1793: `body.contains("DB$ DealDamage")`
- Line 1829: `body.contains("DB$ GainLife")`
- Line 1864: `body.contains("DB$ Earthbend")`

### Attack Triggers (Attacks) - Lines 1934-2038
- Line 1934: `body.contains("AB$ Draw")` or `body.contains("DB$ Draw")`
- Line 1972: `sub_body.contains("DB$ PutCounter")`
- Line 1997: `body.contains("DB$ PutCounter")`
- Line 2019: `body.contains("DB$ GainLife")`
- Line 2038: `body.contains("DB$ DealDamage")`

### SpellCast Triggers - Lines 2150-2170
- Line 2150: `body.contains("DB$ PutCounter")`
- Line 2170: `body.contains("DB$ Pump")`

Total: 17 locations in card.rs

Also in `effect_converter.rs`:
- Line 359: `static_lower.contains("unblock")` or `static_lower.contains("cantblock")`

## Progress

- [x] Added coding principle to CLAUDE.md (4e39557)
- [x] Fire Lord Ozai AB$ Mana uses AbilityParams (4e39557)
- [ ] Refactor ETB trigger parsing
- [ ] Refactor Attack trigger parsing  
- [ ] Refactor SpellCast trigger parsing
- [ ] Refactor extract_token_scripts()

## References

- Coding principle: CLAUDE.md "NO HACKY STRING OPERATIONS ON STRUCTURED DATA"
- Analysis doc: `ai_docs/ability_parsing_comparison.md`
- Card script spec: `ai_docs/CARD_SCRIPT_SPEC.md`
- Proper parser: `mtg-engine/src/loader/ability_parser.rs`
