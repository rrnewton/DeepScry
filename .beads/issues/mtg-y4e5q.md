---
title: 'WASM network DESYNC: CardRevealed for drawn card not processed before ability computation'
status: open
priority: 2
issue_type: bug
labels:
- bug
created_at: 2026-02-21T17:02:07.902149965+00:00
updated_at: 2026-02-21T17:09:19.048876653+00:00
---

# Description

## Bug

In WASM network human mode, the local game state doesn't have the drawn card's identity when abilities are computed for the ChoiceRequest, causing a FATAL DESYNC:

```
FATAL DESYNC: Local abilities (3) != server abilities (4)
Local: [PlayLand { card_id: 113 }, PlayLand { card_id: 114 }, PlayLand { card_id: 115 }]
Server: [PlayLand { card_id: 112 }, PlayLand { card_id: 113 }, PlayLand { card_id: 114 }, PlayLand { card_id: 115 }]
```

Card 112 (Bazaar of Baghdad) was drawn during Turn 2 draw step. The server sends CardRevealed for it, but the WASM client hasn't processed it by the time abilities are computed locally.

## Root Cause

The sync_callback that drains CardRevealed messages may not run at the right time in the WASM game loop. In native mode, the client processes reveals synchronously before computing abilities. In WASM mode, the CardRevealed messages arrive via WebSocket and are queued, but the local game state may compute abilities before the sync_callback processes them.

## Reproducer

```
node web/test_network_human_input.js
```

Requires: make build-network && make wasm-network

**IMPORTANT**: Always launch the server with `--network-debug` to enable full state hash validation. See `docs/NETWORK_ARCHITECTURE.md` "Testing Requirements" section.

## Expected Behavior

Local abilities should always match server abilities. CardRevealed messages must be fully processed before ability computation.

## Context

Per NETWORK_ARCHITECTURE.md, desync is ALWAYS fatal. The WASM local_controller now correctly returns ChoiceResult::Error on desync detection (previously it was a log-only warning that was papered over).

## Investigation Notes

Key files to examine:
- `mtg-engine/src/wasm/network/local_controller.rs` - WASM local controller, validates abilities
- `mtg-engine/src/wasm/network/client.rs` - Receives CardRevealed from server, queues them
- `mtg-engine/src/wasm/network/remote_controller.rs` - Opponent choice handling
- `mtg-engine/src/game/game_loop/mod.rs` - Core game loop, where reveals should be emitted

The fix must ensure CardRevealed messages are applied to shadow state BEFORE ability computation, while maintaining deterministic sequential simulation.
