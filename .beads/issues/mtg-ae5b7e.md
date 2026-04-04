---
title: Snapshot phase/step tracking stuck on 'Untap'
status: open
priority: 3
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:23:07.440895790+00:00
updated_at: 2026-04-03T21:23:07.440895790+00:00
---

# Description

## Context
- Date: 2026-04-03
- Decks: decks/combat_test_4ed.dck (mirror)
- Seed: 42
- Mode: heuristic-vs-heuristic, also agent-vs-heuristic

## Steps to Reproduce
1. Run: `./target/release/mtg tui decks/combat_test_4ed.dck decks/combat_test_4ed.dck --p1=fixed --p2=heuristic --p1-fixed-inputs= --stop-on-choice=1 --snapshot-output=/tmp/test_snapshot.json --json --seed=42 --verbosity=3`
2. Inspect the snapshot JSON: `python3 -c "import json; s = json.load(open('/tmp/test_snapshot.json')); print(s.get('game_state',s).get('turn',{}).get('current_step'))"`
3. Result: "Untap"

## Expected Behavior
The snapshot's `current_step` field should report the actual phase/step when the choice was presented. The game log clearly shows "Main Phase 1" when land play choices are offered, so the snapshot should say "Main1" or similar.

## Actual Behavior
The snapshot always reports `current_step: "Untap"` regardless of what phase the game is actually in. The game log output correctly shows phase transitions (Untap → Upkeep → Draw → Main Phase 1 → Combat → Main Phase 2 → End → Cleanup), but the snapshot metadata does not track these transitions.

The agentplay enriched_log.md inherits this bug, labeling all choice points as "Turn N Untap" even when choices happen during Main Phase.

## Rules Notes
- Not a gameplay rules violation per se, but causes incorrect metadata in agent-driven games
- The game engine phases themselves operate correctly (draw step, main phase, combat are all properly sequenced)

## Impact
Agent players receive incorrect phase information in the snapshot, which can lead to false BUG_REPORT alerts (the agent correctly flagged that it shouldn't be offered land plays during Untap step — but the engine IS in Main Phase, the snapshot just says otherwise).

## Evidence
- Command used: see above
- Game directory: agentplay/013.game/
- enriched_log.md shows: `## Choice Turn 1 Untap` with choices `play Plains, play Forest` — this should be `## Choice Turn 1 Main1`
