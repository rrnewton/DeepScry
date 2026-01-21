---
title: Consolidate remaining placeholder validation checks
status: open
priority: 3
issue_type: task
labels:
- draft
- refactoring
created_at: 2026-01-21T00:27:33.552291262+00:00
updated_at: 2026-01-21T00:27:33.552291262+00:00
---

# Description

## Problem

Despite the recent TriggerContext refactoring, there are still ~39+ places in the codebase that check for placeholder values like `PlayerId::new(0)` or `CardId::new(0)`.

### Current State

The recent refactoring (commit e57eb9d) consolidated placeholder resolution for triggered effects into `resolve_effect_placeholder()`. However, placeholder patterns still appear in:

1. **Activated abilities**: Same patterns but not using TriggerContext
2. **Cost payment**: Checking if costs use placeholder values
3. **Target resolution**: Checking for CardId::new(0) in target lists
4. **Effect execution**: Direct checks in execute_effect() arms

### Patterns Found

```rust
// Pattern 1: Player placeholder check
if player.as_u32() == 0 { ... }

// Pattern 2: Card placeholder check  
if target.as_u32() == 0 { ... }

// Pattern 3: Reuse previous target sentinel
if card_id == CardId::reuse_previous() { ... }
```

### Proposed Fix

1. Add named constants for placeholder values:
```rust
impl PlayerId {
    pub const PLACEHOLDER: PlayerId = PlayerId(0);
    pub fn is_placeholder(&self) -> bool { self.0 == 0 }
}
```

2. Extend the placeholder resolution to cover activated abilities, not just triggers

3. Create a unified `resolve_placeholders_in_effect()` that handles both trigger and activated ability contexts

## Benefits

- Clearer intent: `player.is_placeholder()` vs `player.as_u32() == 0`
- Single point of resolution for all effect types
- Easier to audit placeholder handling

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing:
- No performance regressions allowed - run benchmarks before/after
- `is_placeholder()` must be `#[inline]` and compile to a single comparison
- Placeholder resolution is in hot paths (effect execution) - must be zero-cost
- Constants should be const, not lazy_static or runtime computed

## Status

DRAFT - awaiting approval before implementation
