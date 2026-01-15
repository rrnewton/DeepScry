# Plan: Single-Channel Network Architecture for client.rs

## Problem Statement

The current `run_game` in `client.rs` violates the network architecture by using:
- Multiple channels (5+) for different message types
- `tokio::select!` in the WebSocket handler to multiplex
- `try_recv()` polling for reveals (race condition)

Per `docs/NETWORK_ARCHITECTURE.md`, this is prohibited:
- "No Select Over Multiple Channels" - introduces nondeterminism
- All messages should be processed "in exact order of arrival"

## Root Cause

The current design tries to separate concerns (reveals, local choices, remote choices, game end, errors) into different channels. But this creates races because messages that arrive sequentially over the WebSocket get dispersed to different channels and may be processed out of order.

## Solution: Split Reader/Writer Architecture

### New Architecture

```
                          WebSocket
                         /         \
                    Reader        Writer
                   (task)        (task)
                        \         /
                     recv_rx   send_tx
                         \     /
                        GameLoop
                   (via Controllers)
```

### Components

1. **Reader Task**: Reads from WebSocket, forwards to `recv_tx`
   - Trivial loop: `while let msg = ws.recv() { recv_tx.send(msg) }`
   - No select, no branching logic
   - All ServerMessage variants go to same channel

2. **Writer Task**: Reads from `send_rx`, forwards to WebSocket
   - Trivial loop: `while let msg = send_rx.recv() { ws.send(msg) }`
   - No select, no branching logic
   - Only ClientMessage (SubmitChoice) goes through here

3. **GameLoop Thread**: Runs synchronous GameLoop with network controllers
   - Controllers share access to `recv_rx` (via `Arc<Mutex<Receiver>>`)
   - Controllers send via `send_tx.clone()`
   - Process ALL messages through ONE channel

### Message Flow

**Incoming (Server → Client):**
```
WebSocket → Reader Task → recv_tx → recv_rx → Controller
```

**Outgoing (Client → Server):**
```
Controller → send_tx → send_rx → Writer Task → WebSocket
```

### Controller Behavior

Both `NetworkLocalController` and `RemoteController` share the same `recv_rx`:

```rust
fn wait_for_message(&mut self) -> NetworkMessage {
    loop {
        let msg = self.recv_rx.lock().recv();
        match msg {
            CardRevealed { owner, card, reason } => {
                // Process immediately - update game state
                self.process_reveal(owner, card, reason);
                // Continue waiting for actual choice message
            }
            ChoiceRequest { .. } if I_am_local_controller => return msg,
            OpponentChoice { .. } if I_am_remote_controller => return msg,
            GameEnded { .. } | Error { .. } => return msg,
            _ => {
                // Message for other controller - shouldn't happen
                // if protocol is correct, but log warning
            }
        }
    }
}
```

### Key Changes to `run_game`

**Remove:**
- `reveal_tx, reveal_rx`
- `local_msg_tx, local_msg_rx`
- `remote_choice_tx, remote_choice_rx`
- `game_end_tx, game_end_rx`
- `fatal_error_tx, fatal_error_rx`
- `select!` in WebSocket handler
- `drain_reveals` callback

**Add:**
- `recv_tx, recv_rx` - single channel for ALL server messages
- `send_tx, send_rx` - single channel for ALL client messages
- Shared channel receiver via `Arc<Mutex<Receiver<NetworkMessage>>>`

### Message Type

Use existing `ServerMessage` directly, or define a simplified enum:

```rust
enum NetworkMessage {
    CardRevealed { owner: PlayerId, card: CardReveal, reason: RevealReason },
    ChoiceRequest { action_count: u64, choice_seq: u32 },
    ChoiceAccepted { choice_seq: u32, action_count: u64 },
    OpponentChoice { choice_indices: Vec<usize>, description: String, spell_ability: Option<SpellAbility> },
    GameEnded { winner: Option<PlayerId>, action_count: u64 },
    Error { message: String, fatal: bool },
}
```

### Processing Reveals

Controllers need access to game state to process reveals. Two options:

**Option A: Pass game state reference to controllers**
- Controllers get `&mut GameState` (or `Arc<Mutex<GameState>>`)
- Process reveals directly in controller

**Option B: Controllers return reveals to be processed**
- Controller collects reveals in a buffer
- Before returning choice, expose reveals for GameLoop to process
- GameLoop processes reveals, then uses the choice

Option A is cleaner but requires careful lifetime management.
Option B is more explicit but adds complexity.

**Recommended: Option A** with `Arc<Mutex<GameState>>` for shared mutable access.

### Card Database Access

Controllers need `AsyncCardDatabase` to instantiate cards from reveals:
- Pass `Arc<AsyncCardDatabase>` to controllers
- Use `get_card_def_from_reveal()` helper (already exists)

### Implementation Order

1. Define `NetworkMessage` enum (or decide to use `ServerMessage`)
2. Create shared message receiver type: `Arc<Mutex<mpsc::Receiver<NetworkMessage>>>`
3. Modify `NetworkLocalController`:
   - Add `recv_rx: Arc<Mutex<Receiver>>`, `game: Arc<Mutex<GameState>>`, `card_db: Arc<AsyncCardDatabase>`
   - Implement `wait_and_process()` that loops reading from channel
   - Process CardRevealed messages inline
4. Modify `RemoteController` similarly
5. Rewrite `run_game`:
   - Create single recv channel and send channel
   - Spawn reader task (WebSocket → recv_tx)
   - Spawn writer task (send_rx → WebSocket)
   - Create controllers with shared recv_rx
   - Run GameLoop
6. Remove old infrastructure:
   - Remove `RevealMsg` type
   - Remove `WsHandlerChannels` struct
   - Remove `handle_server_message` (merged into controllers)
   - Remove `run_ws_handler` (replaced by simple reader/writer tasks)

### Testing

After implementation:
1. Run `make validate`
2. Run `tests/network_vs_local_equivalence_e2e.sh` multiple times
3. The ~40% flake rate should drop to 0% if races are eliminated

## Concerns and Mitigations

### Concern: Mutex contention
- Only one controller is active at a time (linear control transfer)
- Mutex should be uncontended in practice

### Concern: Blocking on recv()
- Controllers are synchronous and expected to block
- This matches the architecture: "each party waits for its turn"

### Concern: Opening hand timing
- Opening hand reveals are sent BEFORE GameLoop starts
- Reader task queues them; when controllers start, reveals are waiting
- No race because everything is sequential through one channel
