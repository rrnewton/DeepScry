---
title: 'Networking: Client/Server multiplayer mode'
status: open
priority: 1
issue_type: epic
depends_on:
  mtg-akjrb: related
created_at: 2025-12-05T17:57:01.266857250+00:00
updated_at: 2026-02-21T17:09:35.670586874+00:00
---

# Description

## Networking: Client/Server Multiplayer Mode

Implement networked multiplayer using deterministic simulation with hidden information enforcement.

## Design Document

See `ai_docs/NETWORKING_DESIGN_PLAN.md` for full design.

## Architecture

- **Server** (native only): Authoritative game state, RNG, full deck contents
- **Clients** (native or WASM): Shadow game state, only sees revealed cards
- **Protocol**: WebSocket with JSON messages, choice-based sync (not full state)
- **Verification**: State hash at each choice point to detect desync

## Key Principles

1. **Deterministic simulation**: Clients run independent simulation synced via choices
2. **Hidden information by construction**: Clients never receive opponent hand contents, library order, or RNG state
3. **Remote library abstraction**: Client libraries are buffers that receive cards as revealed
4. **State verification**: Hash-based checksums exclude hidden info
5. **Desync is ALWAYS fatal**: See docs/NETWORK_ARCHITECTURE.md

## Testing Requirements

**ALWAYS** launch the server with \`--network-debug\` in ALL test scripts and launch helpers. This enables full state hash validation after every choice. See \`docs/NETWORK_ARCHITECTURE.md\` "Testing Requirements" section for details.

## CLI Commands

\`\`\`bash
mtg server --port=17771 --password=SECRET --network-debug [--deck-visibility]
mtg connect deck.dck --server=HOST:PORT --password=SECRET
\`\`\`

## Implementation Phases

- [x] mtg-d2p73: Protocol types and message serialization (CLOSED)
- [x] mtg-ely5l: Network state hashing (HashMode::Network) (CLOSED)
- [x] mtg-bl5pe: Engine refactoring (LibraryMode::Remote) (CLOSED)
- [x] mtg-2zdqe: NetworkController implementation (CLOSED)
- [x] mtg-3n53a: WebSocket server (CLOSED)
- [x] mtg-9644z: Client with shadow state (CLOSED)
- [ ] mtg-bfm38: E2E testing
- [x] mtg-akjrb: Action-count timestamped synchronization (protocol refactoring) (CLOSED)

## Active Bugs

- [ ] mtg-y4e5q: WASM network DESYNC: CardRevealed for drawn card not processed before ability computation (priority 2)

## Dependencies

- tokio-tungstenite (native WebSocket)
- futures-util
- futures-executor
