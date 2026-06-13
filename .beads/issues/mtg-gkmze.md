---
title: 'Puzzle bulk runner Phase 4: all .pzl files in parallel, JUnit XML, wired into validate'
status: open
priority: 2
issue_type: task
created_at: 2026-06-13T22:51:28.730546644+00:00
updated_at: 2026-06-13T22:51:28.730546644+00:00
---

# Description

## Status
Phase 4 of the puzzle assertion DSL (PUZZLE_ASSERTION_DSL.md), implementing
the bulk parallel puzzle runner. Tracks the runner implementation and the
known-failing puzzle inventory from the first full corpus run.

## What was built (2026-06-13, branch claude/puzzle-bulk-runner)

### Entry point
`cargo nextest run --test puzzle_bulk_runner --features network`
OR via `make puzzle-bulk-check`

Integration test: mtg-engine/tests/puzzle_bulk_runner.rs

### How to run
- Manually: `make puzzle-bulk-check`
- Via validate: `python3 scripts/validate.py --only puzzle.bulk-check`
- Full validate: `make validate` (included as puzzle.bulk-check step)

### What it does
1. Discovers ALL .pzl files under test_puzzles/, puzzles/, forge-java/forge-gui/res/puzzle/
2. Loads card DB ONCE, shared across all rayon threads
3. Runs every puzzle to endpoint with HeuristicController (both players)
4. Evaluates [assertions] section via evaluate_assertions() where present
5. Puzzles with no [assertions]: smoke/crash check only
6. Parallelizes via rayon pool sized to num_cpus::get()
7. Writes JUnit XML to validate_logs/puzzle_bulk_runner.xml
8. Prints one-line summary

## First corpus run results (2026-06-13)
- Total puzzles discovered: 694
- Wall-clock: ~1.9s (debug) / ~6s (nextest debug mode)
- Threads: 16 (num_cpus)
- OK: 639 (637 smoke, 2 assert)
- FAIL: 55 total
  - panics (engine errors): 36
  - load errors: 19
  - assertion failures: 0

## Known failure inventory (pre-existing brokenness, NOT regressions)

### Load errors (19): parse-time failures
- "Invalid difficulty: Common" — forge-java corpus uses "Common" difficulty,
  our parser requires Easy/Medium/Hard. Affects PP00.pzl, PP01.pzl, PP29.pzl,
  PP30.pzl, PS_AER1.pzl, PS_AKH1.pzl, PS_AKH4.pzl, PS_J221.pzl, MTGP_02.pzl,
  MTGP_09.pzl, forge_tutorial01-03.pzl, Spellslinger.pzl
- "Unknown phase: DECLAREATK" — test_deserters_disciple_combat_timing.pzl,
  test_fire_lord_ozai_attack_trigger.pzl
- "Unknown counter type: TIME" — PS_SPM4.pzl
- "Missing [state] section" — test_barrels_of_blasting_jelly.pzl,
  test_cracked_earth_technique.pzl (WIP/template puzzle stubs)

### Panics/engine errors (36): game-loop failures
- "Token support not yet implemented" — ~28 puzzles (forge-java corpus uses
  tokens e.g. Food, Treasure, creature tokens that our engine doesn't support yet)
- "Card not found" — specific cards missing from cardsfolder (stormchasers_talent_l3,
  c_a_food, Ragnarok_Divine_Deliverance, Voldaren_Thrillseeker, Urza_Planeswalker)

## Validate step status
- Step: puzzle.bulk-check
- Group: puzzle (new group, sharded in CI)
- Baseline: MAX_PANICS=50, MAX_ASSERT_FAIL=10, MAX_LOAD_ERRORS=30
- Status: GREEN (known-bad within tolerances)

## TODO for next phases
- Fix "Invalid difficulty: Common" in parser to accept it (accept forge-java values)
- Implement token support to unblock the 28 token-using puzzles
- Fix card-not-found cases (missing cards in cardsfolder)
- Track per-card fixes as they land; tighten baseline tolerances
- Phase 2 (structured game events): enables log-derived assertions
