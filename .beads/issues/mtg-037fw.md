---
title: 4-way gamelog equivalence test for NETWORK_MODE
status: open
priority: 2
issue_type: task
created_at: 2025-12-08T11:49:48.576522867+00:00
updated_at: 2025-12-29T19:59:29.825623908+00:00
---

# Description

## Goal

Create an E2E test that verifies identical GAMELOG output from 4 sources when running the same game:
1. **Local mode**: Regular `mtg tui` running the game directly
2. **Server**: The authoritative game simulation on the network server
3. **Client 1**: Shadow state gamelog from player 1's client
4. **Client 2**: Shadow state gamelog from player 2's client

This proves the networking layer is a faithful drop-in replacement for local play.

## Current Status (2025-12-29)

**Network Debug Infrastructure Complete!**
- [x] Added `--network-debug` flag to server command
- [x] Server is single source of truth - clients learn debug mode from GameStarted message
- [x] Server populates DebugSyncInfo in ChoiceRequest when enabled
- [x] Client computes and sends client_state_hash + debug_info in SubmitChoice
- [x] Added `compute_view_hash()` for network-safe state hashing
- [x] Added `build_debug_sync_info()` for structured debug info

**Architecture change: Removed message-based mode**
- Deleted `run_game_message_based()` from NetworkClient
- Removed `--message-based` CLI flag from `mtg connect`
- All network clients now use synchronized GameLoop exclusively
- This is the correct architecture for verifiable client shadow state

**Protocol enhancements for debugging:**
- Added `timestamp_ms` to ChoiceRequest, OpponentChoice, SubmitChoice, ChoiceAccepted
- Added `for_player: PlayerId` to ChoiceRequest
- Added `player: PlayerId` to OpponentChoice
- Added `now_ms()` utility function for wall-clock timestamps
- Added `network_debug: bool` to GameStarted message
- Added `--merge-logs` utility to `mtg_tui_networked.py` for unified log analysis

**2-way equivalence test is working!**
- Local mode vs Server gamelogs: **IDENTICAL** (verified)
- Test script: `tests/gamelog_equivalence_e2e.sh`

**Implementation completed for 4-way infrastructure:**
- [x] Added `--tag-gamelogs` and `--verbosity` flags to server command
- [x] Added `--tag-gamelogs` and `--gamelog-output` flags to connect command
- [x] Client's `NetworkClient` has `set_tag_gamelogs()` and `set_gamelog_output()` setters
- [x] Client's `run_game()` configures its GameLoop logger with `tag_gamelogs`
- [x] `mtg_tui_networked.py` supports `MTG_GAMELOG_DIR` for capturing per-process output
- [x] `mtg_tui_networked.py` passes `--tag-gamelogs` to clients

**Client GameLoop sync fix implemented:**

The sync issues have been addressed by modifying `NetworkLocalController` to wait for
`ChoiceRequest` from the server before making each choice:

1. Added `LocalControllerMessage::ChoiceRequest { action_count, choice_seq }` variant
2. WebSocket handler forwards `ChoiceRequest` to `NetworkLocalController` via channel
3. All choice methods in `NetworkLocalController` now call `wait_for_choice_request()` first
4. Controller uses server's `action_count` (not local shadow state) when sending choices

This ensures:
- Client's GameLoop blocks until server reaches the same choice point
- action_count always matches server's authoritative value
- choice_seq is properly synchronized

**Network tests temporarily skipped (mtg-037fw):**
- `tests/network_game_e2e.sh` - SKIPPED
- `tests/gamelog_equivalence_e2e.sh` - SKIPPED
- `test_run_game_with_random_controllers` in network_e2e.rs - #[ignore]

These tests hang around Turn 5-7 due to client/server synchronization desync.

## Prerequisites for 4-way test

Core synchronization is now implemented:

- [x] Client GameLoop must wait for server ChoiceRequest before each choice
- [x] The `NetworkLocalController` needs to block until server sends ChoiceRequest
- [x] Consider adding explicit sync points between client and server GameLoops

Lower priority prerequisites (from mtg-bfm38):
- [ ] Client replays opponent choices on shadow state (currently no-op in `process_opponent_choice()`)
- [ ] Client computes local state hash (currently accepts server hash without verifying)

## Implementation Tasks

**Completed (2-way test):**
- [x] Add `--tag-gamelogs` and `--verbosity` to server command (main.rs, server.rs)
- [x] Pass settings to GameLoop in `run_game_loop()`
- [x] Update `mtg_tui_networked.py` to pass `--tag-gamelogs` to server and clients
- [x] Fix RichInputController.choose_from_options() for network fixed mode
- [x] Create test script `tests/gamelog_equivalence_e2e.sh`
- [x] Add 2-way test to make validate (passes when run individually)
- [x] Add `--tag-gamelogs` to connect command for client GameLoop
- [x] Add `MTG_GAMELOG_DIR` support to mtg_tui_networked.py
- [x] Remove message-based mode (deleted `run_game_message_based()`)
- [x] Add protocol timestamps for debugging

**Completed (sync debugging infrastructure):**
- [x] Add DebugSyncInfo and SyncErrorDetails types to protocol
- [x] Add optional debug fields to SubmitChoice, ChoiceRequest, OpponentChoice
- [x] Add SyncError message variant
- [x] Create compute_view_hash function (network-safe hash from GameStateView)
- [x] Create build_debug_sync_info function (extract turn/phase/life/zones)
- [x] Add `--network-debug` flag to server command
- [x] Server sends network_debug in GameStarted message
- [x] Client reads network_debug from server and configures itself
- [x] Server populates debug_info in ChoiceRequest when debug enabled
- [x] Client populates client_state_hash + debug_info in SubmitChoice when debug enabled

**In Progress (sync debugging):**
- [ ] Server validates client_state_hash against its expected hash
- [ ] Server logs detailed DebugSyncInfo diff when hashes mismatch
- [ ] Add `--network-debug-halt` flag to pause on first mismatch
- [ ] Debug client/server desync around Turn 5-7
- [ ] Re-enable network tests once sync is stable

**Future (4-way test):**
- [ ] Have clients report their gamelog at game end
- [ ] Extend test script to compare all 4 sources

## Test Design

- Use `--seed` for determinism (same game in both modes)
- Use `--tag-gamelogs` flag on server and clients
- Use fixed controllers for deterministic choices (semicolon-separated: "1;0;0;...")
- Filter to only `[GAMELOG ...]` tagged lines for comparison
- Set `MTG_GAMELOG_DIR=/tmp/gamelogs` to capture per-process output

## Test Files

- `tests/gamelog_equivalence_e2e.sh` - Main test script (2-way working)
- Server output includes GAMELOG entries when game completes normally
- `scripts/mtg_tui_networked.py` - Network drop-in replacement with --merge-logs utility

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

**Phase 2 (Blocked - 4-way test):**
1. Runs identical game in local and network modes
2. Extracts gamelogs from all 4 sources (local, server, client1, client2)
3. Asserts all 4 match exactly
4. Fails loudly with diff output if any mismatch

**Blockers for Phase 2:**
- Client GameLoop sync issues - need to fix NetworkLocalController timing

## Root Cause Analysis (2025-12-29)

**The Problem:** Games hang around Turn 3-7 due to **game state drift** between client shadow state and server authoritative state.

**Architecture:**
```
Server                          Client (e.g., P1)
══════                          ═══════════════════
GameLoop (authoritative)        GameLoop (shadow state)
  ├─► NetworkController P1        ├─► NetworkLocalController (us)
  └─► NetworkController P2        └─► RemoteController (opponent)
```

**Sync Mechanism:**
1. For **our choices**: `NetworkLocalController.wait_for_choice_request()` blocks until server sends `ChoiceRequest`
2. For **opponent choices**: `RemoteController.wait_for_choice()` blocks until server sends `OpponentChoice`

**The Deadlock Scenario:**

When client and server game states diverge (even slightly), they may disagree about **whose turn it is** or **who has priority**:

```
Server state: Player 1 has priority → sends ChoiceRequest to P1
Client state: Player 2 has priority → P1's GameLoop asks RemoteController for P2's choice

Result: Server waits for P1's SubmitChoice
        Client waits for OpponentChoice for P2
        DEADLOCK!
```

**Why State Drifts:**
1. **Card reveals**: Libraries differ between server (full cards) and client (CardIds only until revealed)
2. **Draw timing**: Client must receive CardRevealed before draw, any timing issues cause drift
3. **Token creation**: New tokens need proper CardId synchronization
4. **Mana payment choices**: Multi-step mana payment may diverge

**Evidence:**
- Test `test_run_game_with_random_controllers` times out at Turn 3-4 with "Client 1 timed out"
- Last logged state shows both players at 20 life, game running normally
- Then suddenly hangs without errors - classic deadlock pattern

**Required Fix:**
The fundamental issue is that client's GameLoop is **racing independently** against server's GameLoop. Even with wait_for_choice_request(), if the GameLoop reaches a choice point for the **wrong player** first, we deadlock.

**Potential Solutions:**

1. **Explicit turn/phase sync**: Server broadcasts current phase/turn to clients before each choice, clients verify their state matches

2. **Choice type in ChoiceRequest**: Include which controller type the server expects (local vs remote), client can validate or recover

3. **Server-driven model**: Instead of running parallel GameLoops, have server tell client exactly what state changes to apply

4. **State hash validation**: Before each choice, compare state hashes; if mismatch, resync from server snapshot
   - **Progress**: Debug infrastructure now in place (--network-debug flag, client_state_hash, debug_info)
   - **Next**: Server-side validation of client hashes
