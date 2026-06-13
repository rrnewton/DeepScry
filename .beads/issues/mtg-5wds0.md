---
title: Bulk puzzle runner double-runs puzzles already covered by dedicated per-puzzle e2e tests
status: open
priority: 3
issue_type: task
created_at: 2026-06-13T23:31:14.500134940+00:00
updated_at: 2026-06-13T23:31:14.500134940+00:00
---

# Description

## Problem

The bulk puzzle runner (`mtg-engine/tests/puzzle_bulk_runner.rs`) runs ALL ~694 `.pzl` files in a single test step (puzzle.bulk-check). However, ~20 dedicated per-puzzle e2e test scripts in `tests/` ALSO run specific `.pzl` files — so those puzzles execute twice per validate run.

## Known double-run puzzles (dedicated tests that also appear in bulk runner)

Shell scripts in `tests/` that run specific .pzl files:
- animate_dead_reanimate_triskelion_e2e.sh
- dark_ritual_mana_e2e.sh
- fecundity_creature_dies_e2e.sh
- attunement_return_draw_discard_e2e.sh
- cranial_extraction_exile_all_zones_e2e.sh
- balance_equalize_e2e.sh
- city_of_brass_mana_damage_e2e.sh
- black_lotus_sac_mana_e2e.sh
- enduring_ideal_epic_e2e.sh
- power_sink_drain_mana_e2e.sh
- donatello_token_bonus_e2e.sh
- (and others)

Also: `mtg-engine/tests/puzzle_assert_e2e.rs` runs specific .pzl files.

## Why we can't simply delete the dedicated tests now

The shell tests assert on GAME LOG TEXT (stdout/stderr patterns) that inline `[assertions]` in the .pzl file cannot fully express yet. Specifically:
- Event-order assertions
- Trigger-fire assertions
- Negative assertions

The bulk runner only evaluates final-state `[assertions]` sections; it does not yet capture a structured event log for oracle comparison.

## Consolidation plan (do NOT delete dedicated tests yet)

As the migration wave (slot02, claude/puzzle-migration) moves final-state assertions inline and the golden-log + trigger-oracle pieces land (slot03, claude/puzzle-golden-log), replace each dedicated .sh/Rust per-puzzle test with bulk-runner coverage:
1. Migrate final-state assertions to [assertions] DSL in the .pzl
2. Migrate event-log assertions to golden-log oracle (when that lands)
3. Delete the dedicated .sh/.rs test once its coverage is fully in the bulk runner

## Cross-references

- Bulk runner issue: mtg-gkmze
- Migration wave: claude/puzzle-migration (slot02)
- Golden log oracle: claude/puzzle-golden-log (slot03)
- Perf fix (sibling): claude/puzzle-runner-perf (slot01) — fixes multi-thread runtime overhead per puzzle

## Status

Open: duplicate execution wastes validate time (~2x per covered puzzle).
Blocked until migration + golden-log land.
Do NOT close until ALL dedicated per-puzzle tests have been retired into bulk-runner + golden-log coverage.
