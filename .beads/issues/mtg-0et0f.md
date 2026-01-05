---
title: Audit and fix wildcard enum match patterns
status: open
priority: 1
issue_type: chore
created_at: 2026-01-05T19:27:43.276210152+00:00
updated_at: 2026-01-05T19:28:53.244490572+00:00
---

# Description

## Wildcard Enum Match Pattern Audit

This tracking issue tracks the audit and replacement of wildcard enum match patterns (`_ => {}`) with exhaustive pattern matching for type safety.

## Background

Wildcard patterns in match expressions on enums can silently fail when new enum variants are added. This was the root cause of the RemoveCounter/PutCounter targeting bug (mtg-29crm) where new Effect variants weren't handled in `get_valid_targets_for_spell()`.

## Lint Configuration

Clippy provides `clippy::wildcard_enum_match_arm` which warns on these patterns. We have enabled this lint project-wide (commit pending) and require explicit `#[allow(clippy::wildcard_enum_match_arm)]` for intentional wildcards.

**Lint documentation:** https://rust-lang.github.io/rust-clippy/master/index.html#wildcard_enum_match_arm

## Files to Audit (Clippy warnings) - 68 total

The following files have **enum wildcard patterns** that trigger clippy warnings:

**High priority (7 warnings):**
- [ ] mtg-engine/src/game/state.rs (7 warnings)
- [ ] mtg-engine/src/game/actions/mod.rs (7 warnings)
- [ ] mtg-engine/src/core/costs.rs (7 warnings)

**Medium priority (5-6 warnings):**
- [ ] mtg-engine/src/game/game_state_evaluator.rs (6 warnings)
- [ ] mtg-engine/src/game/fancy_tui_controller.rs (5 warnings)
- [ ] mtg-engine/src/game/continuous_effects.rs (5 warnings)

**Lower priority (1-3 warnings):**
- [ ] mtg-engine/src/game/state_hash.rs (3)
- [ ] mtg-engine/src/game/fancy_tui_events.rs (3)
- [ ] mtg-engine/src/deck_builder/native.rs (3)
- [ ] mtg-engine/src/core/persistent_effect.rs (3)
- [ ] mtg-engine/src/zones.rs (2)
- [ ] mtg-engine/src/main.rs (2)
- [ ] mtg-engine/src/game/mana_payment.rs (2)
- [ ] mtg-engine/src/game/game_loop/priority.rs (2)
- [ ] mtg-engine/src/undo.rs (1)
- [ ] mtg-engine/src/puzzle/state.rs (1)
- [ ] mtg-engine/src/puzzle/loader.rs (1)
- [ ] mtg-engine/src/loader/effect_converter.rs (1)
- [ ] mtg-engine/src/game/heuristic_controller.rs (1)
- [ ] mtg-engine/src/game/game_loop/actions.rs (1)
- [ ] mtg-engine/src/game/controller.rs (1)
- [ ] mtg-engine/src/game/actions/targeting.rs (1)
- [ ] mtg-engine/src/core/types.rs (1)
- [ ] mtg-engine/src/core/delayed_trigger.rs (1)
- [ ] mtg-engine/src/core/card.rs (1)

## All Files with Any Wildcard Patterns (50 files, 195 patterns)

These include struct/tuple wildcards which clippy doesn't warn on but may warrant manual review. See original breakdown for full list.

## Progress

- [x] targeting.rs - Fixed 4 exhaustive patterns (commit fb86695)
- [x] Enable `clippy::wildcard_enum_match_arm` as warn in Cargo.toml
- [ ] Fix remaining 68 clippy warnings (or add explicit allow with comments)
- [ ] Review critical Effect enum match sites manually

## Strategy

1. ✅ **Enable lint project-wide** - Added to Cargo.toml workspace lints
2. **Fix warnings by priority** - Start with high-warning files
3. **Manual review** - Check Effect enum matches for logic bugs
4. **Document exceptions** - Use `#[allow(...)]` with comment for intentional wildcards
