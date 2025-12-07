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
2. **Hidden information by construction**: Clients never receive opponent hand contents, library order, or RNG state
3. **Remote library abstraction**: Client libraries are buffers that receive cards as revealed
4. **State verification**: Hash-based checksums exclude hidden info

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
