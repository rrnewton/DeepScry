---
title: Web/WASM Network Integration
status: open
priority: 1
issue_type: epic
created_at: 2025-12-30T19:23:47.819632157+00:00
updated_at: 2025-12-30T21:14:31.072497649+00:00
---

# Description

## Epic: Network Mode for Web/WASM Play

Enable web clients to connect to native Rust server via WebSocket for multiplayer games.

## Key Insight

The existing abort/replay pattern for human input in WASM is orthogonal to networking. Instead of blocking on channel.recv(), we return `NeedInput` and let JavaScript callbacks queue messages.

## Critical Principles

1. **Shared GameLoop**: Use ONE common GameLoop for all modes (local, network, WASM, native)
2. **Avoid Duplication**: Share logic between native and WASM network controllers
3. **Single HTML Entry**: Use existing fancy.html - add "Network" as controller option

## Implementation Phases

### Phase 1: Core WASM Network Infrastructure ✅ COMPLETE (5a47626)
- [x] `wasm/network/mod.rs` - Module structure
- [x] `wasm/network/client.rs` - WasmNetworkClient state machine
- [x] `wasm/network/local_controller.rs` - WasmNetworkLocalController
- [x] `wasm/network/remote_controller.rs` - WasmRemoteController
- [x] `wasm/network/exports.rs` - wasm_bindgen exports
- [x] Updated network/mod.rs to expose protocol types unconditionally
- [x] Added wasm-network Cargo feature

### Phase 2: Game Loop Integration ✅ COMPLETE (29afb99)
- [x] Add WasmControllerType::Network variant
- [x] Extend fancy_tui.rs run_until_choice() for Network branch
- [x] Add run_network_mode() method with rewind/replay pattern
- [x] Clone derive for WasmHumanController for network mode

### Phase 3: JavaScript Integration ✅ COMPLETE (8983a9f)
- [x] `web/network.js` - WebSocket wrapper class (MTGNetworkClient)
- [x] Modify `web/fancy.html` - Add Network controller option
- [x] Connection UI (server URL, password, player name)
- [x] Settings persistence for network fields
- [x] Conditional network imports (graceful fallback when not built)
- [x] launch_network_game() WASM export (placeholder)
- [x] network_init(), network_is_game_ready() exports
- [x] WasmCardDatabase.get_deck_json() for deck submission

### Phase 4: E2E Testing (Web vs Native) ✅ COMPLETE
- [x] `web/test_network_e2e.js` - Playwright E2E tests
- [x] Web client + Native fixed client against native server
- [x] Test verifies: server start, native client connect, browser launch, network mode selection, connection, game UI
- [ ] Secondary: Web vs Web (both Playwright browsers) - future work

### Phase 5: Complete Game Initialization - TODO
- [ ] Receive GameStarted message with seed, player order, starting life
- [ ] Create game state from server-provided parameters (not placeholder)
- [ ] Wire up WasmRemoteController for P2 opponent
- [ ] Process CardRevealed messages to instantiate opponent cards

### Phase 6: Full Network Game Loop - TODO
- [ ] Integrate reveal draining before game loop resume
- [ ] Handle ChoiceRequest/ChoiceAccepted synchronization properly
- [ ] Process OpponentChoice messages through WasmRemoteController
- [ ] Update game state hash verification

### Phase 7: Error Handling & Robustness - TODO
- [ ] Network reconnection after disconnect
- [ ] Graceful handling of server errors
- [ ] Timeout handling for unresponsive server
- [ ] UI feedback for connection state changes

### Phase 8: Polish & UX - TODO  
- [ ] Show opponent name in game UI
- [ ] Display "Waiting for opponent..." status properly
- [ ] Add disconnect/reconnect button during game
- [ ] Show network latency/status indicator

## Architecture

```
Browser (WASM)                          Native Server
┌─────────────────────────┐            ┌─────────────────┐
│ JavaScript              │            │                 │
│ ├─ WebSocket handler    │◄──────────►│ mtg server      │
│ └─ message queue        │  WebSocket │ (existing)      │
│           │             │            │                 │
│           ▼             │            └─────────────────┘
│ WASM Module             │
│ ├─ WasmNetworkClient    │  (state machine + message queues)
│ ├─ WasmNetworkLocal...  │  (wraps WasmHumanController)
│ └─ WasmRemoteController │  (returns NeedInput when waiting)
│           │             │
│           ▼             │
│ GameLoop + ReplayCtrl   │  (existing infrastructure)
└─────────────────────────┘
```

## Progress Log

- 2025-12-30_#1376: Phase 4 complete - E2E test with Playwright (web/test_network_e2e.js)
- 2025-12-30_#1364: Phase 3 complete - JavaScript/HTML integration (8983a9f)
- 2025-12-30_#1363: Phase 2 complete - fancy_tui.rs network integration (29afb99)
- 2025-12-30_#1362: Phase 1 complete - wasm/network module infrastructure (5a47626)
