---
title: 'Network protocol: Action-count timestamped synchronization'
status: open
priority: 1
issue_type: task
created_at: 2025-12-06T12:01:39.912045109+00:00
updated_at: 2025-12-06T13:06:38.391410303+00:00
---

# Description

## Protocol Refactoring for Synchronized Network Play

## Problem

The current network protocol has a race-prone design with the "unconditional broadcast" hack in `server.rs`. This needs to be refactored to a principled, fully synchronous model.

## Design Principles

1. **Fully Lazy, Fully Synchronous Communication**
   - No race conditions by design
   - Every message has a clear request/response pattern
   - No speculative or "just in case" broadcasts

2. **Action Log as Source of Truth**
   - The undo log (action log) is the true notion of time
   - All parties (server + clients) must have identical action logs
   - Like a blockchain ledger - append-only, deterministic

3. **Action Count Timestamps**
   - Every protocol message includes the current action count
   - ChoiceRequest includes: "At action count N, I need you to choose from..."
   - SubmitChoice includes: "At action count N, I chose..."
   - Server validates that action counts match before processing

4. **Debug Mode Validation**
   - `mtg connect --debug` flag enables full action log transmission
   - Server and clients exchange action sequences periodically
   - Any mismatch triggers immediate error with diagnostic info

## Completed Tasks (2025-12-06)

- [x] Add `action_count: u64` field to protocol messages (ChoiceRequest, SubmitChoice, OpponentChoice, ChoiceAccepted)
- [x] Track action count in NetworkController (server-side) - uses GameState::action_count() via GameStateView
- [x] Track action count in NetworkLocalController (client-side) - passes action_count with each choice
- [x] Wire action_count through entire message flow (client → server → opponent)

## Completed (2025-12-06_#1186)

- [x] Add `--debug` flag to `mtg connect` command
- [x] Validate action count in server (logs warning on mismatch)
- [x] Validate action count in client debug mode (fails fast on mismatch)
- [x] Server returns its own action_count in ChoiceAccepted (not echoed client value)

### Debug Mode Test Results

The `--debug` flag successfully detected a real synchronization issue:
```
SYNC WARNING: Player 0 action_count mismatch! client=17 server=14
SYNC ERROR: action_count mismatch! client=17 server=14
```

This confirms the clients running their own GameLoops can diverge from
the server's authoritative state. The debug mode is working correctly
to surface these issues early.

## Completed (2025-12-06_#1188)

- [x] Fixed server validation to use action_count from ChoiceRequest, not stale game state

### Root Cause Analysis

The server's WebSocket handler was reading `action_count` from a stale shared game
state mutex, but the GameLoop runs on a **cloned** copy of the game state. The
mutex-protected original game only had 14 entries (opening hand draws) while
the cloned game state being used by the GameLoop had advanced to 17 entries.

### Fix

Added `expected_action_count: Option<u64>` field to `PlayerConnection` struct.
When sending a `ChoiceRequest`, the server now stores the action_count in this
field. When validating `SubmitChoice`, the server uses this stored value instead
of reading from the stale game state mutex.

This ensures the server validates against the actual action_count that was
communicated to the client, not the stale value from before the GameLoop started.

## Remaining Tasks

- [ ] Remove the unconditional broadcast hack from server.rs
- [ ] Implement proper synchronization points based on action counts
- [ ] Add tests for action count synchronization
- [ ] Enable the `test_run_game_with_random_controllers` test (synchronized GameLoop mode)

## Related Files

- `mtg-engine/src/network/server.rs` - Server message handling
- `mtg-engine/src/network/client.rs` - Client WebSocket handler  
- `mtg-engine/src/network/local_controller.rs` - NetworkLocalController
- `mtg-engine/src/network/remote_controller.rs` - RemoteController
- `mtg-engine/src/network/protocol.rs` - Protocol message definitions

## Implementation Notes

The action_count is sourced from `GameState::action_count()` which returns the undo log position. This value is:
- Set by NetworkController when creating ChoiceRequest (via `view.action_count()`)
- Passed through server to client in ChoiceRequest
- Echoed back by passive client in SubmitChoice
- Tracked by NetworkLocalController in run_game mode (gets from GameStateView)
- Broadcast to opponent in OpponentChoice message
