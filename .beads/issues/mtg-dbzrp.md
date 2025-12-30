---
title: Web/WASM Network Integration
status: open
priority: 1
issue_type: epic
created_at: 2025-12-30T19:23:47.819632157+00:00
updated_at: 2025-12-30T19:57:41.610381971+00:00
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

### Phase 2: Game Loop Integration
- [x] Add WasmControllerType::Network variant (done in Phase 1)
- [ ] Extend fancy_tui.rs run_until_choice() for Network branch
- [ ] Wire up WasmNetworkLocalController and WasmRemoteController
- [ ] Handle reveal draining before game loop resume

### Phase 3: JavaScript Integration
- [ ] `web/network.js` - WebSocket wrapper
- [ ] Modify `web/fancy.html` - Add Network controller option
- [ ] Connection UI (server URL, password, player name)

### Phase 4: Testing (Web vs Native)
- [ ] `web/test_network_e2e.js` - Playwright E2E tests
- [ ] Web client + Native fixed client against native server
- [ ] Secondary: Web vs Web (both Playwright browsers)

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

- 2025-12-30: Phase 1 complete - created wasm/network module with client, controllers, exports
