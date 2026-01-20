---
title: WASM Network Play tracking
status: open
priority: 1
issue_type: epic
created_at: 2026-01-20T10:20:00.468569518+00:00
updated_at: 2026-01-20T10:20:33.035382311+00:00
---

# Description

## WASM Network Play Tracking

Track progress on network multiplayer support in WASM/browser.

## Current Status (2026-01-20_#1717)

**Basic functionality works, but random E2E test has intermittent failures.**

### What Works:
- make wasm-dev builds with wasm-network feature
- WebSocket connection to native server
- Authentication and game setup
- Basic E2E test passes consistently (5/5 runs)
- State tracking moved to shared client (architectural fix)

### What Needs Work:
- mtg-kh2y7: Random E2E test has ~25-30% pass rate (intermittent hangs)

## Architecture

### Native vs WASM Controllers

| Component | Native | WASM |
|-----------|--------|------|
| Local Controller | NetworkLocalController | WasmNetworkLocalController |
| Remote Controller | RemoteController | WasmRemoteController |
| Client | WsReaderShared + MVar | WasmNetworkClient (polling) |
| Blocking | MVar.take() blocks | NeedInput pattern (yields) |

### Key Files:
- mtg-engine/src/wasm/network/client.rs - WebSocket client state
- mtg-engine/src/wasm/network/local_controller.rs - Wraps local controller
- mtg-engine/src/wasm/network/remote_controller.rs - Handles opponent choices
- mtg-engine/src/wasm/fancy_tui.rs - Game loop integration
- web/fancy.html - JavaScript WebSocket handling

## Test Infrastructure

- web/test_network_e2e.js - Basic connectivity test (deterministic)
- web/test_network_random_e2e.js - Random AI game test (exercises full flow)
- scripts/launch_network_game.sh - Starts server + native AI for testing

## Related Documentation

- Plan file: .claude/plans/snazzy-meandering-pond.md
- Native network: mtg-engine/src/network/ (reference implementation)
