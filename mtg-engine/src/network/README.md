# Network Module

WebSocket-based client/server multiplayer implementation for MTG Forge.

## Architecture

The network module implements a **deterministic simulation with hidden information enforcement**:

- **Server** (native only): Runs authoritative game state, controls RNG, knows all cards
- **Clients** (native or WASM): Run shadow game state, only see revealed cards
- **Protocol**: Choice-based sync (not full state transfer)
- **Verification**: Hash-based checksums at each choice point detect desync

## Key Principles

1. **Deterministic simulation**: Clients run independent simulation synced via choices
2. **Hidden information by construction**: Clients never receive opponent card names, library order, or RNG state
3. **Action count synchronization**: ALL parties have identical action_count (via dummy reveals)
4. **State verification**: Hash-based checksums exclude undo_log (allowing asymmetric reveals)

## Hidden Information Model (mtg-qtqcr)

### Core Invariant: Identical Action Counts

All parties (server + all clients) must have **identical action_count** at every sync point.
This means every `CardRevealed` message must be sent to all clients - but with different content:

```
Own card:       CardRevealed { card_id: 5, name: "Lightning Bolt" }  // Real reveal
Opponent card:  CardRevealed { card_id: 5, name: "" }                // Dummy reveal
```

The dummy reveal (empty name) lets the client know "CardID 5 exists and was revealed" without
learning what card it is. This keeps action_count synchronized across all parties.

### What's Public vs Private

| Information | Visibility |
|-------------|------------|
| CardID ranges (P1: 0-39, P2: 40-79) | PUBLIC - all parties know |
| "CardID 5 is in P1's hand" | PUBLIC - zone membership |
| "CardID 5 is Lightning Bolt" | PRIVATE - until played/revealed |
| Library order after shuffle | PRIVATE (server knows, clients don't) |
| RNG state | PRIVATE (server only) |

### RevealCard Semantics

When a card needs to be revealed (draw, play, discard to graveyard):

1. **Server broadcasts** `CardRevealed` to ALL clients
2. **Recipients differ**:
   - Owner/viewers: `name = "Lightning Bolt"` (real card name)
   - Non-viewers: `name = ""` (dummy reveal, keeps count in sync)
3. **Client processes**:
   - Real reveal: instantiate Card in EntityStore
   - Dummy reveal: track that CardID exists, don't instantiate

### Opening Hand Example

When game starts with 7-card hands:

```
Server sends to P1:
  7x CardRevealed for P1's hand (with real names)
  7x CardRevealed for P2's hand (with empty names)  // P1 can't see P2's cards

Server sends to P2:
  7x CardRevealed for P1's hand (with empty names)  // P2 can't see P1's cards
  7x CardRevealed for P2's hand (with real names)
```

Both clients receive 14 reveals → same action_count → state hashes match.

### The Option<Box<Card>> Field

In `RevealCard` GameAction, the `card: Option<Box<Card>>` field stores the full card instance:
- `Some(card)`: Real reveal - card data for undo (EntityStore restoration)
- `None`: Dummy reveal - nothing to undo

This allows proper undo semantics: a real reveal can be undone by removing the card from
EntityStore; a dummy reveal is a no-op for undo.

### Implementation Status (Phase 3)

**Current bug**: Server sends full card names to ALL clients for opponent's cards.
**Fix needed**: Strip opponent card names before sending (set to empty string).

## Module Structure

| File | Description |
|------|-------------|
| `mod.rs` | Module exports and constants (default port 17771) |
| `protocol.rs` | Message types for client/server communication (`ClientMessage`, `ServerMessage`) |
| `controller.rs` | `NetworkController` - player controller that delegates to network |
| `local_controller.rs` | `LocalController` - wraps local player for network games |
| `remote_controller.rs` | `RemoteController` - simplified remote player interface |
| `server.rs` | WebSocket server implementation (requires `network` feature) |
| `client.rs` | WebSocket client implementation (requires `network` feature) |

## Protocol Messages

### Client → Server
- `Authenticate` - Initial auth with password and deck submission
- `SubmitChoice` - Response to choice requests (with action count for sync)
- `Disconnect` - Graceful disconnect
- `Ping` - Keepalive

### Server → Client
- `AuthResult` - Authentication success/failure
- `WaitingForOpponent` - Lobby state
- `GameStarted` - Initial game setup info
- `CardRevealed` - Hidden card becomes visible
- `ChoiceRequest` - Request player decision
- `OpponentChoice` - Opponent's decision for simulation sync
- `GameEnded` - Final game result
- `Error` / `Pong` - Utility messages

## CLI Usage

```bash
# Start server
mtg server --port=17771 --password=SECRET

# Connect as client
mtg connect deck.dck --server=HOST:PORT --password=SECRET
```

## Feature Flag

The server and client implementations require the `network` feature:

```toml
[features]
network = ["tokio-tungstenite", "futures-util"]
```

## Related Issues

- mtg-to96y: Networking epic tracking issue
- mtg-bfm38: E2E network testing
