---
title: WASM network random test intermittent hangs
status: open
priority: 2
issue_type: bug
depends_on:
  mtg-byq4z: parent-child
created_at: 2026-01-20T10:20:18.995281594+00:00
updated_at: 2026-01-20T10:20:18.995281594+00:00
---

# Description

## Problem

The random E2E test (test_network_random_e2e.js) has intermittent failures with ~25-30% pass rate.
The basic E2E test (test_network_e2e.js) passes consistently (5/5 runs).

## Symptoms (2026-01-20_#1717)

1. Game progresses normally for 15-21 choices (Turn 2)
2. Browser hangs - no more JavaScript logs
3. Native game (server + native client) continues and eventually finishes
4. Test times out after 90 seconds

## Investigation Findings

### State Tracking Fix (Attempted)
Moved last_submitted_choice_seq from ephemeral WasmNetworkLocalController to
persistent WasmNetworkClient. This was architecturally correct but didn't
fully resolve the issue.

### Hang Pattern
Last browser log entries show:
- choice_request seq=N arrives
- Triggering game loop after choice_request
- (nothing - no submit_choice sent)

This suggests the game loop is called but the controller returns NeedInput
or hangs instead of making/submitting a choice.

### Timing Observations
- Messages arrive rapidly (1-2ms apart)
- Multiple game loop triggers can queue up
- JavaScript single-threaded - triggers are sequential
- WASM tui_run_turn() is synchronous (blocks JS event loop)

## Possible Root Causes

1. **Race condition**: choice_request arrives during game loop, not seen until next iteration
2. **State corruption**: Some game state causes infinite loop or unexpected NeedInput
3. **WebSocket timing**: Messages queued during WASM execution not processed correctly
4. **Game loop termination**: Game returns Complete but state not cleaned up properly

## Reproduction

```bash
cd web
for i in {1..10}; do
  timeout 90 node test_network_random_e2e.js 2>&1 | grep -E 'Result:|PASSED|FAILED'
done
```

## Files Involved

- mtg-engine/src/wasm/network/local_controller.rs - check_choice_request_ready()
- mtg-engine/src/wasm/network/client.rs - state tracking
- mtg-engine/src/wasm/fancy_tui.rs - run_network_mode_ai()
- web/fancy.html - onMessageProcessed callback, tui_run_turn() calls

## Next Steps

1. Add more detailed logging to trace exact control flow
2. Check if RandomController ever returns NeedInput (shouldn't)
3. Investigate if specific game states trigger the hang
4. Consider adding yield points in long-running game loops
