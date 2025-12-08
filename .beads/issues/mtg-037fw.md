---
title: 4-way gamelog equivalence test for NETWORK_MODE
status: open
priority: 2
issue_type: task
created_at: 2025-12-08T11:49:48.576522867+00:00
updated_at: 2025-12-08T12:42:18.030920480+00:00
---

# Description

## Goal

Create an E2E test that verifies identical GAMELOG output from 4 sources when running the same game:
1. **Local mode**: Regular `mtg tui` running the game directly
2. **Server**: The authoritative game simulation on the network server
3. **Client 1**: Shadow state gamelog from player 1's client
4. **Client 2**: Shadow state gamelog from player 2's client

This proves the networking layer is a faithful drop-in replacement for local play.

## Current Status (2025-12-08)

**Completed:**
- [x] Added `--tag-gamelogs` and `--verbosity` flags to server command
- [x] Server's GameLoop now uses these settings correctly
- [x] Created test script `tests/gamelog_equivalence_e2e.sh`
- [x] Updated `scripts/mtg_tui_networked.py` to pass `--tag-gamelogs` to server
- [x] **Fixed**: Client disconnect bug - added `choose_from_options()` to RichInputController
- [x] **2-way equivalence achieved**: Local mode vs Server output match perfectly!

**Test Results (commit 917edd5):**
- Local mode: 32 GAMELOG entries (Turn 1-31, with land plays and draws)
- Server mode: 69 GAMELOG entries (continues past local's fixed input limit)
- First 32 entries: **IDENTICAL** ✓

The test verifies:
1. Local mode produces correct GAMELOG output (M1 for land plays, DR for draws)
2. Server-side game produces identical GAMELOG entries
3. Same seed + fixed inputs = identical game progression

**Root cause of original bug:**
The `RichInputController` was missing a `choose_from_options()` implementation. The default trait
implementation reads from stdin, causing network clients with fixed controllers to hang waiting
for input instead of using their command script. Fixed in rich_input_controller.rs:400-477.

## Prerequisites for 4-way test (from mtg-bfm38)

Before the full 4-way test can work, clients need shadow state:
- [ ] Client replays opponent choices on shadow state (currently no-op in `process_opponent_choice()`)
- [ ] Client tracks own choice results on shadow state
- [ ] Client computes local state hash (currently accepts server hash without verifying)
- [ ] Client collects gamelog entries during shadow state updates

## Implementation Tasks

**Completed (2-way test):**
- [x] Add `--tag-gamelogs` and `--verbosity` to server command (main.rs, server.rs)
- [x] Pass settings to GameLoop in `run_game_loop()`
- [x] Update `mtg_tui_networked.py` to pass `--tag-gamelogs` to server
- [x] Fix RichInputController.choose_from_options() for network fixed mode
- [x] Create test script `tests/gamelog_equivalence_e2e.sh`
- [x] Add 2-way test to make validate (passes when run individually)

**Future (4-way test):**
- [ ] Implement client shadow state tracking
- [ ] Add gamelog collection to shadow state (Vec<String> or similar)
- [ ] Have clients report their gamelog at game end
- [ ] Extend test script to compare all 4 sources

## Test Design

- Use `--seed` for determinism (same game in both modes)
- Use `--tag-gamelogs` flag on server
- Use fixed controllers for deterministic choices (semicolon-separated: "1;0;0;...")
- Filter to only `[GAMELOG ...]` tagged lines for comparison
- The scripts/diff_logs.py tool may be useful for comparison

## Test Files

- `tests/gamelog_equivalence_e2e.sh` - Main test script (created)
- Server output includes GAMELOG entries when game completes normally

## Related

- mtg-bfm38: Networking E2E tests (parent tracking issue)
- tests/tag_gamelogs_e2e.sh: Existing --tag-gamelogs test
- tests/network_game_e2e.sh: Existing network E2E test
- scripts/mtg_tui_networked.py: Network drop-in replacement

## Acceptance Criteria

**Phase 1 (Completed - 2-way test):**
1. ✓ Runs identical game in local and network modes
2. ✓ Extracts gamelogs from local and server
3. ✓ Asserts they match for the duration of the test
4. ✓ Test script: `tests/gamelog_equivalence_e2e.sh`

**Phase 2 (Future - 4-way test):**
1. Runs identical game in local and network modes
2. Extracts gamelogs from all 4 sources (local, server, client1, client2)
3. Asserts all 4 match exactly
4. Fails loudly with diff output if any mismatch
