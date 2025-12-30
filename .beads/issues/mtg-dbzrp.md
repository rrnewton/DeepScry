---
title: Web/WASM Network Integration
status: open
priority: 1
issue_type: epic
created_at: 2025-12-30T19:23:47.819632157+00:00
updated_at: 2025-12-30T19:23:47.819632157+00:00
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

### Phase 1: Core WASM Network Infrastructure
- [ ] `wasm/network/mod.rs` - Module structure
- [ ] `wasm/network/client.rs` - WasmNetworkClient state machine
- [ ] `wasm/network/local_controller.rs` - WasmNetworkLocalController
- [ ] `wasm/network/remote_controller.rs` - WasmRemoteController
- [ ] `wasm/network/exports.rs` - wasm_bindgen exports

### Phase 2: Game Loop Integration
- [ ] Add WasmControllerType::Network variant
- [ ] Extend fancy_tui.rs run_until_choice() for Network
- [ ] Add ChoiceContext::WaitingForServer/WaitingForOpponent variants
- [ ] Add wasm-network Cargo feature

### Phase 3: JavaScript Integration
- [ ] `web/network.js` - WebSocket wrapper
- [ ] Modify `web/fancy.html` - Add Network controller option

### Phase 4: Testing (Web vs Native)
- [ ] `web/test_network_e2e.js` - Playwright E2E tests
- [ ] Web client + Native fixed client against native server
- [ ] Secondary: Web vs Web (both Playwright browsers)

## Code Sharing Strategy

Extract common logic:
1. Message processing (protocol.rs already shared)
2. Choice handling - `process_opponent_choice()` helper
3. Reveal processing - `process_card_reveal()` helper
4. State machine transitions

## Architecture

```
Browser (WASM)                          Native Server
┌─────────────────────────┐            ┌─────────────────┐
│ JavaScript              │            │                 │
│ ├─ WebSocket handler    │◄──────────►│ mtg server      │
│ └─ message queue        │  WebSocket │ (existing)      │
│           ▼             │            └─────────────────┘
│ WASM Module             │
│ ├─ WasmNetworkClient    │
│ ├─ WasmNetworkLocal...  │  (wraps WasmHumanController)
│ └─ WasmRemoteController │  (returns NeedInput when waiting)
│           ▼             │
│ GameLoop + ReplayCtrl   │  (existing infrastructure)
└─────────────────────────┘
```

## Risk Mitigations

- State desync: Use existing action_count echoing + hash comparison
- CardRevealed timing: Drain reveals BEFORE resuming game loop
- Message ordering: TCP guarantees order; queue all before processing

## Related Issues

See mtg-037fw for native network mode implementation details.
