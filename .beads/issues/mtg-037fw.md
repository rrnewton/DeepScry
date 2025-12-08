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
- [x] Server's GameLoop now uses these settings correctly (verified with debug logging)
- [x] Created test script `tests/gamelog_equivalence_e2e.sh`
- [x] Updated `scripts/mtg_tui_networked.py` to pass `--tag-gamelogs` to server

**Findings:**
- Server correctly outputs GAMELOG entries for draws (DR) when game runs
- Server verbosity and tag_gamelogs are correctly set (verified via debug logging)
- **Blocker**: Network client has issues completing game loop - clients disconnect immediately after receiving opening hand, before making any choices
- The issue manifests as "WebSocket protocol error: Connection reset without closing handshake"
- This appears to be a pre-existing network client bug that prevents the full game from running

**Test demonstrates:**
- Local mode: Produces correct GAMELOG output (M1 for land plays, DR for draws)
- Network mode: Only captures DR entries because game ends prematurely due to client disconnect

## Prerequisites (blockers from mtg-bfm38)

Before the 4-way test can work, clients need to:
- [ ] **FIX: Network client game loop completion bug** - clients disconnect before making choices
- [ ] Client replays opponent choices on shadow state (currently no-op in `process_opponent_choice()`)
- [ ] Client tracks own choice results on shadow state
- [ ] Client computes local state hash (currently accepts server hash without verifying)
- [ ] Client collects gamelog entries during shadow state updates

## Implementation Tasks

- [x] Add `--tag-gamelogs` and `--verbosity` to server command (main.rs, server.rs)
- [x] Pass settings to GameLoop in `run_game_loop()` 
- [x] Update `mtg_tui_networked.py` to pass `--tag-gamelogs` to server
- [ ] Fix network client game loop to not disconnect prematurely
- [ ] Add gamelog collection to shadow state (Vec<String> or similar)
- [ ] Have clients report their gamelog to server at game end (or to a file)
- [ ] Create test script that compares all 4 sources
- [ ] Add test to `make validate`

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

A test in `make validate` that:
1. Runs identical game in local and network modes
2. Extracts gamelogs from all 4 sources
3. Asserts all 4 match exactly
4. Fails loudly with diff output if any mismatch
