---
title: Centralize zone and color string parsing in effect_converter.rs
status: closed
priority: 4
issue_type: task
labels:
- draft
- refactoring
created_at: 2026-01-21T00:27:19.235309428+00:00
updated_at: 2026-01-21T10:11:38.912964912+00:00
closed_at: 2026-01-21T10:11:38.912964852+00:00
---

# Description

## Problem

The `loader/effect_converter.rs` file has duplicated string parsing logic for zones and colors in multiple places.

### Duplicated Patterns

1. **Zone parsing**: Multiple match statements like:
```rust
match zone_str {
    "Graveyard" | "graveyard" => Zone::Graveyard,
    "Hand" | "hand" => Zone::Hand,
    "Library" | "library" => Zone::Library,
    // ...
}
```

2. **Color parsing**: Multiple match statements like:
```rust
match color_str {
    "White" | "white" | "W" => Color::White,
    "Blue" | "blue" | "U" => Color::Blue,
    // ...
}
```

These patterns are repeated across:
- `parse_defined_target()`
- `convert_effect()`
- `parse_token_script()`
- Various helper functions

### Issues

1. **Inconsistent casing handling**: Some places handle lowercase, others don't
2. **Different error handling**: Some return None, some use default values
3. **Maintenance burden**: Adding a new zone/color requires updating multiple places
4. **No validation**: Invalid strings silently fall through to defaults

### Proposed Fix

Create centralized parsing functions:

```rust
impl Zone {
    pub fn from_str_lenient(s: &str) -> Option<Zone> {
        // Handle all case variants in one place
    }
}

impl Color {
    pub fn from_str_lenient(s: &str) -> Option<Color> {
        // Handle W/U/B/R/G and full names
    }
}
```

Then replace all inline match statements with calls to these functions.

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing:
- No performance regressions allowed - run benchmarks before/after
- Zone/color parsing happens at card load time, not during gameplay - less critical
- Still avoid allocations where possible (return Option<Zone> not Result<Zone, String>)
- Use match over string comparisons for efficiency

## Status

DRAFT - awaiting approval before implementation
