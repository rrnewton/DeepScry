---
title: Extract repeated mana color iteration into helper function
status: open
priority: 4
issue_type: task
labels:
- draft
- refactoring
created_at: 2026-01-21T00:27:07.283560560+00:00
updated_at: 2026-01-21T00:27:07.283560560+00:00
---

# Description

## Problem

There are multiple places in the codebase that iterate over mana colors with the same pattern:

```rust
for color in [Color::White, Color::Blue, Color::Black, Color::Red, Color::Green] {
    // do something with color
}
```

This pattern appears in:
- `game/actions/mod.rs` - Mana payment iteration
- `game/mana.rs` - Mana pool operations
- Various affordability checks

### Current Issues

1. **Inconsistent ordering**: Some places use WUBRG order, others might use different orders
2. **Magic numbers**: Repeated literal color arrays
3. **Missing colorless**: Sometimes colorless should be included, sometimes not
4. **No central iteration helper**: Each callsite repeats the array construction

### Proposed Fix

Add a central helper in `core/colors.rs`:

```rust
impl Color {
    /// Returns iterator over the five colors in WUBRG order
    pub fn all_colors() -> impl Iterator<Item = Color> {
        [Color::White, Color::Blue, Color::Black, Color::Red, Color::Green].into_iter()
    }
    
    /// Returns iterator including colorless
    pub fn all_colors_and_colorless() -> impl Iterator<Item = Color> {
        // ...
    }
}
```

Then replace all inline color arrays with calls to these methods.

## Benefits

- Single source of truth for color ordering
- Clearer intent at callsites
- Easier to add special handling (e.g., for snow mana) in one place

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing:
- No performance regressions allowed - run benchmarks before/after
- The helper must NOT allocate - use a const array that returns `impl Iterator`
- This is a zero-cost abstraction - compiled code should be identical to inline arrays

## Status

DRAFT - awaiting approval before implementation
