---
title: 'Split large files: actions/mod.rs and loader/card.rs'
status: open
priority: 3
issue_type: task
created_at: 2026-01-21T00:24:30.185211813+00:00
updated_at: 2026-01-21T00:24:30.185211813+00:00
---

# Description

## Split Large Files (4000+ lines)

## Problem

Two files significantly exceed the 1500-line guideline from CLAUDE.md:
- `mtg-engine/src/game/actions/mod.rs`: 4,587 lines
- `mtg-engine/src/loader/card.rs`: 4,017 lines

These large files make navigation difficult and indicate lack of modularity.

## Proposed Solution

### actions/mod.rs → Split into:
1. `spell_casting.rs` - `cast_spell_8_step()`, mana tapping logic (~300 lines)
2. `effect_execution.rs` - `execute_effect()` and helpers (~1200 lines)
3. `triggers.rs` - Already created, trigger checking (~500 lines)
4. `combat.rs` - Already exists, combat actions
5. `targeting.rs` - Already exists, target validation
6. `mod.rs` - Thin dispatcher, imports, shared utilities (~500 lines)

### loader/card.rs → Split into:
1. `card_definition.rs` - CardDefinition struct and core methods
2. `ability_parsing.rs` - parse_effects, parse_triggers, SVar handling
3. `keyword_parsing.rs` - Keyword string parsing
4. `card_loader.rs` - File I/O, CardDatabase

## Acceptance Criteria
- [ ] No file exceeds 1500 lines
- [ ] All tests pass
- [ ] No functionality changes
- [ ] Imports are clean and organized

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing this refactoring:
- No performance regressions allowed - run benchmarks before/after
- Avoid introducing new allocations (clone, collect, Box, Vec creation in hot paths)
- Prefer references over owned types
- Splitting files should not change any runtime behavior or add overhead

## Related
- CLAUDE.md: "You also dislike long files. Whenever a file grows longer than 1500 lines you propose ideas for breaking it into separate modules."
- OPTIMIZATION.md: Performance guidelines for the project
