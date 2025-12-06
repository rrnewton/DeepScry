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

## Completed (2025-12-06_#1189)

- [x] Fixed server initialization to match client pattern for synchronized GameLoops
- [x] Enable the `test_run_game_with_random_controllers` test (synchronized GameLoop mode)

### Root Cause Analysis

The previous fix (#1188) addressed the stale game state mutex issue, but the root cause
of the action_count divergence (client=17 vs server=14) remained. Investigation showed:

**Server flow** (before fix):
1. `draw_opening_hand()` called BEFORE GameLoop starts
2. Adds 14 `MoveCard` actions to undo_log
3. Game cloned for GameLoop (undo_log has 14 entries)
4. GameLoop sees non-empty undo_log → `is_resuming_from_snapshot = true`
5. GameLoop **skips** setup block entirely

**Client flow**:
1. Fresh GameState with empty undo_log
2. Libraries converted to Remote mode
3. GameLoop with `.skip_opening_hands()` flag
4. GameLoop sees empty undo_log → `is_resuming_from_snapshot = false`
5. GameLoop **enters** setup block, draws 14 cards from reveal queue
6. Also logs step advance actions (Untap→Upkeep, Upkeep→Draw, Draw→Main1)

**The difference of 3 (17 - 14)** = step advance actions logged only by client.

### Fix

1. Changed `draw_opening_hand()` to `peek_opening_hand()` in server.rs
   - Server now **peeks** at top 7 cards without drawing
   - Game state remains unchanged (empty undo_log)

2. Added `.skip_opening_hands()` to server's GameLoop builder
   - Both server and client GameLoops now use the same initialization path
   - Both start with empty undo_log
   - Both draw opening hands during `setup_game()`
   - Both log identical actions

This ensures both server and client GameLoops produce identical undo_logs.

## Completed (2025-12-06_#1190)

- [x] Remove the unconditional broadcast hack from server.rs

### Fix: Principled Choice Ordering

The "unconditional broadcast hack" was broadcasting opponent choices with a default
`ChoiceType::Priority { available_count: 0 }` when the client submitted a choice before
the server had processed its corresponding `ChoiceRequest`.

**Root Cause**: In synchronized GameLoop mode, the client's GameLoop might reach a choice
point slightly before the server's GameLoop due to timing differences. When this happens,
the client sends `SubmitChoice` before the server has even sent `ChoiceRequest`.

**Solution**: Added `pending_choice: Option<PendingChoice>` field to `PlayerConnection`:

1. **When SubmitChoice arrives before ChoiceRequest**: Queue it in `pending_choice`
2. **When ChoiceRequest arrives and there's a pending_choice**: Process immediately
   with proper `choice_type` from the request, then broadcast to opponent
3. **Normal case (SubmitChoice after ChoiceRequest)**: Process directly with the
   already-known `choice_type`

This ensures:
- No more fallback default `ChoiceType` - always use the correct type from `ChoiceRequest`
- No race conditions - choices are matched by order (ChoiceRequest sets context,
  SubmitChoice provides answer)
- Clean separation: server doesn't need to guess what type of choice it's processing

## Remaining Tasks

- [ ] Implement 3-way action log verification at end of game (server + 2 clients)

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
