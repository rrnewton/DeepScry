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

## Root Cause (2026-01-20_#1719)

**CONFIRMED: Client-Server Game State Desync**

The server logs reveal the issue:
```
[WARN] NetworkController 1: Invalid choice index 2 (max 1), clamping to 0
[WARN] NetworkController 1: Corrected choice from [2] to [0]
[WARN] RemoteController: invalid ability index 2 (available=0)
```

The WASM client's local "shadow" game state is **drifting** from the server's
authoritative game state:
1. Server says: "you have 1 option (pass priority)"
2. Client's local game says: "I have 3 options"
3. Client's RandomController chooses option index 2
4. Server receives invalid index, clamps to 0

This desync accumulates over time. Eventually the states diverge so much that:
- The local game gets stuck in a state the server isn't in
- OR the game loop enters an infinite internal loop
- OR a controller call hangs waiting for something that won't arrive

**Why this happens:**
The WASM network client lacks proper synchronization mechanisms that the native
client has:
- Native: Uses action-count keyed reveals with `drain_reveals_up_to()`
- Native: Has `sync_callback` that processes reveals before choices
- Native: Tracks `server_action_count` for sync targeting
- WASM: Has none of these, uses simple FIFO reveal queue

**The basic E2E test passes because:**
It uses deterministic behavior (ZeroController always passes), so the local game
state stays roughly in sync with the server. The random test introduces divergence
because the client's RandomController makes choices based on its (incorrect)
local game state.

## Fix Required

Implement the architecture updates outlined in the plan file:
`.claude/plans/snazzy-meandering-pond.md`

Key changes needed:
1. Add action-count keyed reveal handling to WasmNetworkClient
2. Add sync mechanism to WasmNetworkLocalController
3. Track server_action_count for proper synchronization
4. Ensure reveals are processed before each choice

This is non-trivial - the native client evolved this architecture over time,
and WASM needs similar mechanisms adapted for non-blocking operation.

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

1. ✅ Add more detailed logging - DONE (root cause found)
2. ✅ Check RandomController - VERIFIED it never returns NeedInput
3. ✅ Identify cause - Client-server desync confirmed via server warnings
4. **TODO**: Implement sync architecture (see Fix Required section above)
5. **TODO**: Add integration tests for sync behavior
