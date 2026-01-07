---
title: 'Network desync debugging: nondeterministic state divergence'
status: open
priority: 3
issue_type: task
created_at: 2026-01-07T15:45:56.797997299+00:00
updated_at: 2026-01-07T16:01:05.203173122+00:00
---

# Description

## Nondeterministic Network Desync

## Problem

When running network multiplayer games (server + 2 clients), the game **occasionally desyncs** with clients reporting different game states. The desync is **nondeterministic** - the same game (same seed, same decks, same AI controllers) can desync on different action numbers across runs.

## Observed Symptoms

Server logs show state hash mismatch errors like:
```
[ERROR mtg_forge_rs::network::server]       [ 765] Choice(P1 #361 = Some(SpellAbility(Some(CastSpell { card_id: 62 }))))
[ERROR mtg_forge_rs::network::server]       [ 765] Choice(P1 #361 = Some(SpellAbility(Some(PlayLand { card_id: 65 }))))
```

Key observations:
- Same action number (765), different card_id values (62 vs 65)
- Not reproducible with same seed - desync point varies
- Suggests race condition in event ordering

## Suspected Root Cause: Multiple Channel Select

The network code uses `tokio::select!` over multiple channels, which can introduce nondeterminism. The code already has comments acknowledging this issue:

### Server side (server.rs)
- Line 917: Main game loop `select!` over game_loop_handle, p1_handler, p2_handler
- Line 1019: Per-player handler `select!` over:
  - `fatal_error_rx.recv()` - error broadcasts
  - `game_end_rx` - game end signal
  - `request_rx.recv()` - choice requests
  - `ws_rx.next()` - WebSocket messages
  - `reveal_rx.recv()` - reveal broadcasts (documented as problematic)
  - `immediate_reveal_rx.recv()` - immediate reveals
  - `opponent_choice_rx.recv()` - opponent choices

### Client side (client.rs)
- Line 959, 1394, 1558: Multiple `select!` blocks in WebSocket handler

### Previous Fixes (documented in comments)
The code contains comments noting previous attempts to fix desync:
- Lines 1119-1127: "NOTE: We intentionally do NOT broadcast reveals to the opponent via async channels. The async broadcast can arrive out of order due to tokio::select! scheduling, causing desync."
- Lines 1543-1555: Similar note about reveal channel ordering issues
- Lines 1747-1750: "NOTE: We intentionally do NOT use .with_reveal_pusher() here..."
- SINGLE-CHANNEL FIX comments in client.rs (966, 978, 1003, etc.)

## Architecture Goal

**Single channel, single event loop, totally ordered events.**

The only nondeterministic aspects should be:
- GUI/redraw timing
- User input timing

Game state progression must be fully deterministic.

## Files to Audit

- `mtg-engine/src/network/client.rs` - Lines 959, 1394, 1558 (`select!` usage)
- `mtg-engine/src/network/server.rs` - Lines 917, 1019 (`select!` usage)
- `mtg-engine/src/network/controller.rs` - NetworkController channels

## Channels in Use (from audit)

Server creates many channels between sync/async boundaries:
- `request_tx/rx` - ChoiceRequest (sync -> async bridged)
- `response_tx/rx` - ChoiceResponse
- `game_end_tx/rx` - GameEndInfo (oneshot)
- `opponent_choice_tx/rx` - OpponentChoiceInfo
- `reveal_tx/rx` - RevealBroadcast batches
- `immediate_reveal_tx/rx` - Immediate reveals
- `ability_tx/rx` - ChosenAbilityInfo
- `fatal_error_tx/rx` - broadcast channel for errors

The reveal channels have been mostly mitigated but the core `select!` over multiple sources remains.

## Debugging

Build with network feature and use `--network-debug` flag:
```bash
cargo build --release --features network
./target/release/mtg server --network-debug --seed=42
./target/release/mtg connect --controller=heuristic -n P1 deck1.dck
./target/release/mtg connect --controller=random -n P2 deck2.dck
```

## Related Issues

- Winner signal race condition (FIXED in commit 05737b3d)
- WebSocket shutdown handshake (separate issue)

## Priority

Priority 3 - significant bug affecting network play correctness.
