---
title: 4-way gamelog equivalence test for NETWORK_MODE
status: open
priority: 2
issue_type: task
created_at: 2025-12-08T11:49:48.576522867+00:00
updated_at: 2025-12-08T15:22:23.676567424+00:00
---

# Description

## Goal

Create an E2E test that verifies identical GAMELOG output from 4 sources when running the same game:
1. **Local mode**: Regular `mtg tui` running the game directly
2. **Server**: The authoritative game simulation on the network server
3. **Client 1**: Shadow state gamelog from player 1's client
4. **Client 2**: Shadow state gamelog from player 2's client

This proves the networking layer is a faithful drop-in replacement for local play.

## Current Status (2025-12-08, updated)

**Completed:**
- [x] Added `--tag-gamelogs` and `--verbosity` flags to server command
- [x] Server's GameLoop now uses these settings correctly
- [x] Created test script `tests/gamelog_equivalence_e2e.sh`
- [x] Updated `scripts/mtg_tui_networked.py` to pass `--tag-gamelogs` to server
- [x] Test passes when run individually - local and network gamelogs match for first N entries

**Current State:**
- The 2-way comparison (local vs network server) is working
- The test `tests/gamelog_equivalence_e2e.sh` passes when run in isolation
- Test may be flaky in `make validate` due to resource contention with other concurrent agents
- Client shadow state gamelogs (3rd and 4th sources) not yet implemented

## Remaining Work for 4-way Comparison

- [ ] Client replays opponent choices on shadow state (currently no-op in `process_opponent_choice()`)
- [ ] Client tracks own choice results on shadow state
- [ ] Client computes local state hash (currently accepts server hash without verifying)
- [ ] Client collects gamelog entries during shadow state updates
- [ ] Extend test to compare all 4 sources

## Implementation Notes

The WIP commit (917edd5) added this test infrastructure. It was committed mid-development but the 2-way test is functional. The commit message was just 'wip' but the changes are:
- `mtg-engine/src/main.rs`: Added --tag-gamelogs and --verbosity to server command
- `mtg-engine/src/network/server.rs`: Pass settings to GameLoop
- `scripts/mtg_tui_networked.py`: Pass --tag-gamelogs to server
- `tests/gamelog_equivalence_e2e.sh`: New E2E test

## Related

- mtg-bfm38: Networking E2E tests (parent tracking issue)
- tests/tag_gamelogs_e2e.sh: Existing --tag-gamelogs test
- tests/network_game_e2e.sh: Existing network E2E test
