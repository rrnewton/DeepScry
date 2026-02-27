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

- [x] mtg-y4e5q: WASM network DESYNC: CardRevealed for drawn card not processed before ability computation (CLOSED)
- [ ] mtg-61a70: WASM network hang: available_count mismatch after intermediate server actions
- [ ] mtg-lyh66: WASM network: card images not loading in network mode
- [ ] mtg-ouk0p: WASM network: click focus between panes not working
- [ ] mtg-yetqe: WASM network: clicking card doesn't show detail
- [ ] mtg-vgmjz: WASM network: interface unresponsive, flashes during opponent turn
- [ ] mtg-hbjkp: WASM network: game log only shows turn markers, not action messages
- [ ] mtg-7umvv: WASM network: reduce console spam, put debug logs behind network-debug flag

## Bug Fix: Transient Guard Reset in Rewind (2026-02-24_#1855)

Fixed critical DESYNC in network human mode (`60a77990b`). During `rewind_to_turn_start()`, transient guard fields (`draw_step_executed_turn`, `turn_state_reset_turn`, etc.) were NOT reset because they are `#[serde(skip)]` and not tracked by the undo log. After rewind, the draw step guard still had `Some(current_turn)`, causing the mandatory draw to be skipped during replay → missing card → ability count DESYNC.

Fix: Call `game.turn.reset_transient_guards()` and clear `pending_cast`/`pending_activation`/`spell_targets` after rewinding.

Previous fix `12308e80c` addressed a related but different issue: RevealCard undo destroying card instances in EntityStore.

## Bug Fix: Multiple Rewind/Replay DESYNCs (2026-02-24_#1857(387a24da6))

Fixed 5 DESYNC bugs in WASM network human mode (`387a24da6`):

1. **card.damage accumulation**: Damage/bonus fields not undo-logged → doubled during replay. Fix: Clear damage, power_bonus, toughness_bonus, temp_base_stats in `rewind_to_turn_start()`.
2. **City of Brass targeting**: `Defined$ You` for DealDamage not parsed → targeted creatures instead of controller. Fix: Parse into `TargetRef::Player(placeholder)`, resolved via `resolve_effect_placeholder()`.
3. **Non-deterministic trigger targets**: `.find()` on battlefield after rewind → different order than original. Fix: Collect candidates, sort by CardId, take `.first()`.
4. **Wheel of Fortune**: `Defined$ Player` (each player) and `Mode$ Hand` (entire hand) not supported. Fix: Added ALL_PLAYERS_ID sentinel, `expand_all_players_effect()`, Mode$ Hand → count=u8::MAX sentinel.
5. **Unrevealed card discard crash**: Network shadow state has no Card entity for hidden cards. Fix: Fallback card name in `discard_card()`, direct hand iteration for Mode$ Hand.

Test evidence: 100 choices across 11 turns with zero DESYNCs.

## Dependencies

- tokio-tungstenite (native WebSocket)
- futures-util
- futures-executor
