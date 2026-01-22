---
title: WASM network mode enters infinite loop (all AI controllers)
status: closed
priority: 3
issue_type: task
labels:
- bug
- wasm
- network
created_at: 2026-01-22T20:41:08.494913065+00:00
updated_at: 2026-01-22T21:54:31.707263882+00:00
---

# Description

## Description

## Bug Description

**FIXED in commit 0fa012e6** - WASM network AI controllers now work correctly.

When running WASM network mode with ANY AI controller (heuristic, random, zero), the game was entering an infinite loop after connection, repeatedly calling `run_network_mode` without making any game progress.

## Root Cause

The WASM client was creating its own game state with `create_game_from_database()` instead of using the server's late-binding CardIDs via `init_game_reserve_only()`. This caused a complete state mismatch where the WASM client's CardIDs didn't match the server's.

## Fix

Implemented a "direct AI response" pattern in `run_network_mode_ai_direct()`:
1. Server sends ChoiceRequest with options and choice_type
2. WASM AI picks option(s) based on controller type (random/heuristic/zero)
3. WASM submits choice indices directly back to server
4. No local game loop or shadow game state needed

Also fixed multi-select handling for choices like Discard that require multiple selections.

## Verification

```bash
python3 bug_finding/network_fuzz_test.py --wasm --configs 5
## Results: 5/5 passed (heuristic vs heuristic)
```

Server log showing successful completion:
```
[INFO] Game 1: Completed, winner = Some(0), action_count = 881
```

## Related Commits

- 0fa012e6 feat(wasm): Implement AI direct response pattern for WASM network mode

## Note

Native heuristic AI still has some desync issues that should be tracked separately.
