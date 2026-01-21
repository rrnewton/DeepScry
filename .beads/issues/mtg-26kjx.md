---
title: Centralize targeting validation - use is_legal_target() consistently
status: closed
priority: 3
issue_type: task
created_at: 2026-01-21T00:24:58.963624667+00:00
updated_at: 2026-01-21T10:29:32.388292692+00:00
closed_at: 2026-01-21T10:29:32.388292622+00:00
---

# Description

## Centralize Targeting Validation

## Problem

A canonical `is_legal_target()` function exists in `targeting.rs:24-36`:
```rust
pub(crate) fn is_legal_target(card: &Card, source_controller: PlayerId) -> bool {
    if card.has_shroud() {
        return false;
    }
    if card.has_hexproof() && card.owner != source_controller {
        return false;
    }
    true
}
```

But this function is **bypassed** in several places with inline reimplementations:

### Duplicated Locations

1. **actions/mod.rs:2969** - Inline check that differs slightly:
   ```rust
   if card.controller != controller && (card.has_hexproof() || card.has_shroud()) {
       return None;
   }
   ```
   Note: Uses `card.controller` instead of `card.owner` - potential bug!

2. **heuristic_controller.rs:1111 and 2041** - Protection checks duplicated:
   ```rust
   if attacker.has_protection_from(*color) {
       return false;
   }
   ```

3. **triggers.rs** - Various trigger effect targeting doesn't call the shared function

## Proposed Solution

1. Expand `is_legal_target()` to handle all targeting checks including protection
2. Add variants for different targeting contexts:
   - `is_legal_spell_target()` - For spells targeting permanents
   - `is_legal_ability_target()` - For activated/triggered abilities
   - `is_legal_attack_target()` - For combat (protection from color)

3. Replace all inline checks with calls to centralized functions

4. Fix the `owner` vs `controller` inconsistency - hexproof should check controller for targeting purposes

## Acceptance Criteria
- [ ] Single source of truth for "can this permanent be targeted"
- [ ] All inline hexproof/shroud checks removed
- [ ] Protection checks centralized
- [ ] owner vs controller semantics clarified and consistent
- [ ] All tests pass

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing this refactoring:
- No performance regressions allowed - run benchmarks before/after
- Targeting checks are called frequently - functions must be `#[inline]` or very small
- Avoid allocations in the targeting path (no Vec, String, Box)
- Prefer early returns over complex boolean expressions
