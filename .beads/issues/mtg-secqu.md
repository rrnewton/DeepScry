---
title: 'Single-channel network architecture: eliminate select! nondeterminism'
status: closed
priority: 2
issue_type: task
created_at: 2026-01-08T01:50:48.341942482+00:00
updated_at: 2026-01-08T02:22:40.887719603+00:00
---

# Description

## Goal

Implement a truly single-channel architecture where each PlayerConnection has exactly ONE `game_rx` and ONE `game_tx` channel, eliminating all `tokio::select!` over multiple game-state channels.

## Design Principles

1. **Linear control transfer**: At any moment, exactly ONE entity has "control"
2. **Sequential handler loop**: No select!, handler knows exactly what to wait for
3. **Opponent choices flow through game coordinator**: Not directly between handlers
4. **WebSocket I/O is naturally sequential**: One message at a time

## Message Types

```rust
/// Messages from game coordinator to handler
enum GameToHandler {
    /// Server needs this player to make a choice
    ChoiceRequest(ChoiceRequest),
    /// Opponent made a choice (routed through coordinator, not direct)
    OpponentMadeChoice(OpponentChoiceInfo),
    /// Acknowledge player's choice was applied
    ChoiceAccepted { choice_seq: u32, action_count: u64 },
    /// Game has ended
    GameEnded(GameEndInfo),
    /// Fatal error occurred
    FatalError(String),
}

/// Messages from handler to game coordinator
enum HandlerToGame {
    /// Player submitted their choice
    ChoiceResponse(ChoiceResponse),
    /// Client disconnected
    ClientDisconnected,
    /// Client sent invalid data
    ClientError(String),
}
```

## New PlayerConnection Structure

```rust
struct PlayerConnection {
    player_id: PlayerId,
    ws_tx: WsSender,
    game_rx: Receiver<GameToHandler>,  // SINGLE rx from game coordinator
    game_tx: Sender<HandlerToGame>,    // SINGLE tx to game coordinator
}
```

## Handler Loop (No Select!)

```rust
async fn handle_player(conn: PlayerConnection, mut ws_rx: WsReceiver) {
    loop {
        match conn.game_rx.recv().await {
            Some(GameToHandler::ChoiceRequest(req)) => {
                conn.ws_tx.send(ChoiceRequest).await;
                let response = ws_rx.recv_client_choice().await;
                conn.game_tx.send(HandlerToGame::ChoiceResponse(response)).await;
            }
            Some(GameToHandler::OpponentMadeChoice(info)) => {
                conn.ws_tx.send(OpponentChoice).await;
            }
            Some(GameToHandler::ChoiceAccepted { .. }) => {
                conn.ws_tx.send(ChoiceAccepted).await;
            }
            Some(GameToHandler::GameEnded(info)) => {
                conn.ws_tx.send(GameEnded).await;
                break;
            }
            Some(GameToHandler::FatalError(msg)) => {
                conn.ws_tx.send(Error { fatal: true }).await;
                break;
            }
            None => break,
        }
    }
}
```

## Game Coordinator Flow

When P1 makes a choice:
1. Coordinator receives `HandlerToGame::ChoiceResponse` from P1
2. Applies choice to game state
3. Sends `GameToHandler::ChoiceAccepted` to P1
4. Sends `GameToHandler::OpponentMadeChoice` to P2
5. Continues game loop

Key: Opponent notifications flow THROUGH coordinator, not P1 handler -> P2 handler directly.

## Implementation Tasks

- [ ] Define `GameToHandler` and `HandlerToGame` enums in server.rs
- [ ] Create single channel pair per player (game_tx, game_rx)
- [ ] Remove old channels: request_rx, response_tx, player_rx, player_tx, game_end_rx, fatal_error_rx/tx
- [ ] Rewrite handler loop to be sequential (no select!)
- [ ] Modify game coordinator to route opponent choices through channels
- [ ] Remove pending_choice handling (no longer needed - messages buffer in WebSocket)
- [ ] Update NetworkController to work with new channel structure
- [ ] Handle client disconnect detection (WebSocket read returns error/None)
- [ ] Test extensively for determinism

## Why This Eliminates Nondeterminism

- No `select!` over multiple channels = no race conditions
- Handler receives from exactly one source at a time
- All game state messages have total ordering through coordinator
- Control transfer is explicit and sequential

## Related

- mtg-e66iz: Original desync bug report
- Replaces the partial channel consolidation done previously
