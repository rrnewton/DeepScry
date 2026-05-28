# Network Debug Mode Design

## Overview

This document designs a network debug mode for early detection of client/server sync drift,
with meaningful error messages that pinpoint where divergence first occurred.

## Goals

1. **Early detection**: Catch sync drift at the first choice where state hash mismatches
2. **Meaningful errors**: "State first desynced at action_count=47, choice_seq=12..."
3. **History context**: Show last K actions/choices leading up to divergence
4. **Production-safe**: Can disable extra network traffic in production mode

## Modes

### Production Mode (default)
- `state_hash` sent in ChoiceRequest (already exists)
- Client validates hash but only logs warning on mismatch
- Minimal network overhead (just the u64 hash)

### Debug Mode (`--network-debug`)
- Full state hash validation with enforcement
- Extra debug fields in messages:
  - `debug_last_actions: Option<Vec<String>>` - Last N actions from undo log
  - `debug_phase_info: Option<PhaseInfo>` - Current turn/phase/step
- On mismatch: immediate halt with detailed diagnostic dump

## Protocol Changes

### Add to SubmitChoice (client → server)

```rust
SubmitChoice {
    choice_seq: u32,
    choice_index: usize,
    action_count: u64,
    timestamp_ms: u64,
    // NEW: Client's state hash for server validation
    #[serde(skip_serializing_if = "Option::is_none")]
    client_state_hash: Option<u64>,
    // NEW: Debug info (only in debug mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    debug_info: Option<DebugSyncInfo>,
}
```

### Add to ChoiceRequest (server → client)

Already has `state_hash`, but add:

```rust
ChoiceRequest {
    // ... existing fields ...
    // NEW: Debug info (only in debug mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    debug_info: Option<DebugSyncInfo>,
}
```

### Add to OpponentChoice (server → client)

```rust
OpponentChoice {
    // ... existing fields ...
    // NEW: Post-choice state hash for client validation
    #[serde(skip_serializing_if = "Option::is_none")]
    state_hash_after: Option<u64>,
    // NEW: Debug info (only in debug mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    debug_info: Option<DebugSyncInfo>,
}
```

### New DebugSyncInfo struct

```rust
/// Debug synchronization information
/// Only populated when network debug mode is enabled
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSyncInfo {
    /// Turn number
    pub turn: u32,
    /// Current phase
    pub phase: String,
    /// Active player
    pub active_player: PlayerId,
    /// Priority holder (who's making this choice)
    pub priority_player: Option<PlayerId>,
    /// Life totals: [P1_life, P2_life]
    pub life_totals: [i32; 2],
    /// Hand sizes: [P1_hand_size, P2_hand_size]
    pub hand_sizes: [usize; 2],
    /// Library sizes: [P1_lib_size, P2_lib_size]
    pub library_sizes: [usize; 2],
    /// Battlefield card count
    pub battlefield_count: usize,
    /// Stack size
    pub stack_size: usize,
    /// Last N actions from undo log (strings)
    pub last_actions: Vec<String>,
}
```

### New SyncError message (server → client)

```rust
/// Sent when server detects sync mismatch
SyncError {
    /// The choice sequence where mismatch was detected
    choice_seq: u32,
    /// Expected action count
    expected_action_count: u64,
    /// Client's reported action count
    client_action_count: u64,
    /// Expected state hash
    expected_hash: u64,
    /// Client's reported hash
    client_hash: u64,
    /// Server's debug info at this point
    server_debug_info: DebugSyncInfo,
    /// Detailed description of the mismatch
    description: String,
}
```

## Implementation

### 1. Replace `compute_simple_hash` with `compute_network_state_hash`

In `network/controller.rs`, replace the placeholder:

```rust
fn compute_simple_hash(&self, view: &GameStateView) -> u64 {
    // Replace with proper network hash
    // Problem: GameStateView doesn't give us full GameState
    // Solution: Store a reference to game state or compute from view
    compute_view_hash(view)
}
```

Since `GameStateView` doesn't have the full state, we need a hash function that works with the view.

### 2. Create `compute_view_hash` function

```rust
/// Compute network-safe hash from GameStateView
/// This must produce the same result as compute_network_state_hash(game)
/// for clients and servers to agree
pub fn compute_view_hash(view: &GameStateView) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Turn/phase info
    view.turn_number().hash(&mut hasher);
    view.phase().hash(&mut hasher);
    view.active_player().hash(&mut hasher);

    // Life totals
    for player_id in &[PlayerId::new(0), PlayerId::new(1)] {
        view.player_life(*player_id).hash(&mut hasher);
    }

    // Zone sizes (public info)
    view.hand_size().hash(&mut hasher);
    view.library_size().hash(&mut hasher);
    // ... opponent zone sizes too

    // Battlefield state (sorted by card ID for determinism)
    let mut battlefield_cards: Vec<_> = view.battlefield_cards().collect();
    battlefield_cards.sort_by_key(|c| c.id);
    for card in battlefield_cards {
        card.id.hash(&mut hasher);
        card.tapped.hash(&mut hasher);
        // ... other visible attributes
    }

    // Stack (public)
    for item in view.stack() {
        item.id.hash(&mut hasher);
    }

    // Graveyards (public)
    // ... etc

    hasher.finish()
}
```

### 3. Add `NetworkDebugMode` configuration

```rust
/// Network debug mode settings
#[derive(Debug, Clone, Default)]
pub struct NetworkDebugConfig {
    /// Enable debug mode (validate all hashes, include debug info)
    pub enabled: bool,
    /// Number of actions to include in debug_last_actions
    pub action_history_size: usize, // default: 20
    /// Halt on first mismatch (vs warn and continue)
    pub halt_on_mismatch: bool,
}
```

### 4. Validation flow

**Server side (on receiving SubmitChoice):**

```rust
if let Some(client_hash) = submit_choice.client_state_hash {
    if client_hash != expected_hash {
        if debug_config.halt_on_mismatch {
            // Send SyncError and close connection
            send_sync_error(...)
        } else {
            log::warn!("SYNC WARNING at choice_seq={}: client_hash={:08x} expected={:08x}",
                choice_seq, client_hash, expected_hash);
        }
    }
}
```

**Client side (on receiving ChoiceRequest):**

```rust
let local_hash = compute_view_hash(view);
if local_hash != choice_request.state_hash {
    if debug_config.halt_on_mismatch {
        return Err(SyncError { ... })
    } else {
        log::warn!("SYNC WARNING: local_hash={:08x} server_hash={:08x}",
            local_hash, choice_request.state_hash);
    }
}
```

### 5. CLI flags

```
mtg server --network-debug          # Enable debug mode
mtg server --network-debug-halt     # Also halt on mismatch
mtg connect --network-debug         # Client debug mode
```

## Error Message Format

When desync is detected:

```
=== NETWORK SYNC ERROR ===
First detected at: choice_seq=12, action_count=47
Mismatch type: state_hash

Server state:
  hash: 0xABCD1234
  turn: 4, phase: Main1, active: P1
  life: [20, 17]
  hands: [5, 6], libs: [48, 47]
  battlefield: 8 cards, stack: 0

Client state:
  hash: 0xDEAD5678
  turn: 4, phase: Main1, active: P1
  life: [20, 17]
  hands: [5, 6], libs: [48, 47]
  battlefield: 7 cards, stack: 0  ← DIFFERENCE: 1 fewer card

Server last 5 actions:
  [45] P1 plays Tundra
  [46] P1 taps Tundra for {U}
  [47] P1 casts Ancestral Recall targeting P1

Client last 5 actions:
  [45] P1 plays Tundra
  [46] P1 taps Tundra for {U}
  [47] P1 casts Ancestral Recall targeting P1

Analysis: States match through action 47, but battlefield count differs.
Likely cause: Token creation or card draw handling mismatch.
```

## Implementation Order

1. Add `DebugSyncInfo` and `SyncError` to protocol.rs
2. Create `compute_view_hash` function
3. Replace `compute_simple_hash` in controller.rs
4. Add `client_state_hash` to SubmitChoice
5. Add validation in server's handle_submit_choice
6. Add validation in client's wait_for_choice_request
7. Add CLI flags for debug mode
8. Add diagnostic output formatting

## Network Overhead

**Production mode:** ~0 extra bytes (hash already sent)

**Debug mode (typical):**
- DebugSyncInfo: ~200-500 bytes per message
- Extra hash in SubmitChoice: 8 bytes
- Total per choice: ~500 bytes extra

This is acceptable for debugging. In a 100-choice game, that's ~50KB extra traffic.
