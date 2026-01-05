---
title: Audit and fix wildcard enum match patterns
status: open
priority: 1
issue_type: chore
created_at: 2026-01-05T19:27:43.276210152+00:00
updated_at: 2026-01-05T20:21:26.724859644+00:00
---

# Description

## Wildcard Enum Match Arm Audit (mtg-0et0f)

Track and eliminate wildcard (`_ =>`) pattern matches in enum matching
to ensure compile-time safety when new variants are added.

## Lint Configuration

The `clippy::wildcard_enum_match_arm` lint is enabled at the workspace
level in `Cargo.toml` with `warn` severity. Files with intentional 
wildcards have `#![allow(clippy::wildcard_enum_match_arm)]` with a 
`TODO(mtg-0et0f)` comment.

## Files with Remaining Wildcards (to be whittled away)

Progress: 14 files fixed, 21 remaining with file-level allows

### Fixed (wildcards removed or justified with function-level allows):
- [x] `core/costs.rs` - exhaustive Cost matching (7 patterns)
- [x] `game/actions/mod.rs` - exhaustive patterns + function-level allows for triggers (7 patterns)
- [x] `game/state.rs` - exhaustive Zone matching + function-level allow for undo (7 patterns)
- [x] `loader/card.rs` - converted test wildcards to `let...else` pattern (5 patterns)
- [x] `core/card.rs` - exhaustive CardType matching (1 pattern)
- [x] `core/persistent_effect.rs` - exhaustive CleanupCondition matching (3 patterns)
- [x] `core/types.rs` - function-level allow for CounterType::power_toughness_mod() (1 pattern)
- [x] `core/delayed_trigger.rs` - exhaustive DelayedTriggerCondition matching (1 pattern)
- [x] `core/effects.rs` - converted test wildcards to `let...else` pattern (3 patterns)
- [x] `core/keyword_set.rs` - converted test wildcard to `let...else` pattern (1 pattern)
- [x] `deck_builder/native.rs` - function-level allow for run_main_loop() (crossterm KeyCode/Event)
- [x] `game/actions/targeting.rs` - exhaustive Cost matching (1 pattern)
- [x] `game/controller.rs` - exhaustive ChoiceResult matching (1 pattern)
- [x] `game/continuous_effects.rs` - exhaustive CardType + function-level allow (5 patterns)

### Files with documented file-level allows (justified wildcards):
These files handle external crate enums (crossterm KeyCode/Event/MouseEventKind 25+ variants)
or large internal enums (Effect 24+ variants, AffectedSelector 70+ variants) where wildcards
are intentional - only a subset of variants is handled, and unmatched variants are safely ignored.

- [x] `game/fancy_tui_controller.rs` - crossterm KeyCode/Event/MouseEventKind
- [x] `game/fancy_tui_events.rs` - FocusedPane/Entity enums
- [x] `game/game_loop/actions.rs` - Effect enum (24+ variants)
- [x] `game/game_loop/priority.rs` - Effect enum (24+ variants)
- [x] `game/game_state_evaluator.rs` - AffectedSelector enum (70+ variants)
- [x] `game/heuristic_controller.rs` - Effect enum (24+ variants)
- [x] `game/mana_payment.rs` - ManaProductionKind enum
- [x] `game/state_hash.rs` - serde_json::Value enum
- [x] `game/test_spider_suit.rs` - test file

### Source files (10 remaining):
- [ ] `loader/effect_converter.rs` (3 warnings)
- [ ] `loader/svar_parser.rs` (1 warning)
- [ ] `network/client.rs` (1 warning)
- [ ] `network/controller.rs` (1 warning)
- [ ] `network/local_controller.rs` (1 warning)
- [ ] `network/protocol.rs` (1 warning)
- [ ] `network/server.rs` (1 warning)
- [ ] `puzzle/loader.rs` (1 warning)
- [ ] `puzzle/state.rs` (2 warnings)
- [ ] `undo.rs` (1 warning)
- [ ] `zones.rs` (2 warnings)

### WASM-only files (2):
- [ ] `wasm/deck_builder.rs` (KeyCode/Entity wildcards)
- [ ] `wasm/fancy_tui.rs` (KeyCode/KeyInput/EventResult wildcards)

### Test files (5):
- [ ] `tests/card_loading.rs`
- [ ] `tests/human_input_e2e.rs`
- [ ] `tests/network_e2e.rs`
- [ ] `tests/undo_e2e.rs`
- [ ] `tests/wasm_rich_input_e2e.rs`

### Example files (3):
- [ ] `examples/activated_abilities_expanded_demo.rs`
- [ ] `examples/basic_land_demo.rs`
- [ ] `examples/combat_demo.rs`

## Approach for Each File

1. Remove the file-level `#![allow(...)]`
2. Run `cargo clippy` to see exact warnings
3. For each wildcard:
   - If variants should be handled differently: add exhaustive pattern
   - If truly intentional (e.g., "all other effects"): add function-level allow with comment
4. Run tests, commit
