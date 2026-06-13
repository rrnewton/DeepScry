---
title: puzzle golden game-log oracle
status: open
priority: 3
issue_type: task
created_at: 2026-06-13T23:29:17.751973909+00:00
updated_at: 2026-06-13T23:29:17.751973909+00:00
---

# Description

## puzzle golden game-log oracle (Phase 5 of PUZZLE_ASSERTION_DSL, tracks mtg-0oopj)

Adds a golden game-log snapshot oracle on top of the existing bulk puzzle runner.

### What was built
- mtg-engine/tests/puzzle_golden_check.rs - new integration test that:
  - Discovers all locally-authored .pzl files (test_puzzles/ + puzzles/, 331 total)
  - Runs each with Normal verbosity + OutputMode::Memory to capture the game log
  - Compares against committed golden files in test_puzzles/goldens/ and puzzles/goldens/
  - Mismatch = CI failure with readable unified diff
  - MTG_BLESS_GOLDEN=1 writes goldens instead of comparing (ONE-COMMAND re-bless)
  - Forge-java corpus excluded (pre-existing panics catalogued in mtg-0oopj)
  - Parallel via rayon (same pattern as puzzle_bulk_runner - DRY)

### One-command re-bless
  make puzzle-bless
(or MTG_BLESS_GOLDEN=1 make puzzle-golden-check)
After blessing, git diff test_puzzles/goldens/ puzzles/goldens/ shows what changed.
Commit the updated goldens to land the intentional format change.

### Stats (2026-06-13)
- Total local puzzles discovered: 331
- Goldens committed: 325
- Excluded (pre-existing failures, logged): 6
  - stormchasers_talent_l3_e2e.pzl: missing card Stormchaser's Talent@L3
  - test_barrels_of_blasting_jelly.pzl / test_cracked_earth_technique.pzl: missing [state] section
  - test_food_token_ability.pzl: missing card c_a_food
  - test_deserters_disciple_combat_timing.pzl / test_fire_lord_ozai_attack_trigger.pzl: unknown phase DECLAREATK
- Total golden file size: ~1MB apparent (313KB + 713KB across 325 files)
- All files are plain text (no ANSI codes, no binaries)

### Wiring
- make puzzle-golden-check / make puzzle-bless (Makefile targets)
- validate.py: puzzle.golden-check step (puzzle CI shard)
- .gitignore: !*.golden.log exception so goldens are tracked despite *.log rule

### STATUS
DONE. Branch: claude/puzzle-golden-log in slot03. Pushed after full validate.
