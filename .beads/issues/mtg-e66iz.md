---
title: 'Network desync debugging: nondeterministic state divergence'
status: open
priority: 3
issue_type: task
created_at: 2026-01-07T15:45:56.797997299+00:00
updated_at: 2026-01-08T01:39:11.862629756+00:00
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

The network code uses `tokio::select!` over multiple channels, which can introduce nondeterminism.

## Architecture Goal

**Single channel, single event loop, totally ordered events.**

The only nondeterministic aspects should be:
- GUI/redraw timing
- User input timing

Game state progression must be fully deterministic.

## Progress

### Single-Channel Migration - COMPLETE (2026-01-08)

The channel architecture has been consolidated:

1. **PlayerChannelMessage enum** - Multiplexes different message types through one channel
   - `OpponentChoice(OpponentChoiceInfo)` - opponent made a choice

2. **player_tx/player_rx channels** - Cross-wired channel pairs
   - P1's player_tx → P2's player_rx (P1 sends opponent choices to P2)
   - P2's player_tx → P1's player_rx (P2 sends opponent choices to P1)

3. **Eliminated channels**:
   - `reveal_rx` - reveals now sent synchronously via ChoiceRequest.reveals
   - `immediate_reveal_rx` - same as above  
   - `ability_rx` - abilities now included directly in ChoiceRequest.abilities
   - `opponent_choice_tx/rx` - replaced by player_tx/rx single channel

4. **Remaining channels in select! block**:
   - `fatal_error_rx` - errors (doesn't affect game state order)
   - `game_end_rx` - one-shot (only fires once)
   - `request_rx` - ChoiceRequests TO client (triggers client decisions)
   - `ws_rx` - WebSocket messages FROM client (single stream, ordered)
   - `player_rx` - single-channel messages (opponent choices, totally ordered)

### Key Commits

- `e525d89f4` - fix(wasm): Remove skip_serializing_if from CardDefinition (fixed bincode deserialization)
- `61dc1bbed` - refactor(network): Eliminate ability_rx channel via ChoiceRequest.abilities
- `48682a1ae` - refactor(network): Begin single-channel migration for desync fix
- `3dc65432b` - refactor(network): Remove dead code from server.rs
- `ca99a7fe4` - refactor(network): Remove dead reveal channel infrastructure

### Current Architecture

The channel architecture is now simplified:
- **Game state messages** flow through `player_rx` (single channel, totally ordered)
- **Control messages** (errors, game end) use dedicated channels that don't affect ordering
- **WebSocket I/O** is single-stream per player
- **Ability info** is bundled with ChoiceRequest (no separate channel)

### Next Steps

1. ✅ Migrate ability_rx to single-channel → DONE (bundled into ChoiceRequest.abilities)
2. ✅ Remove dead reveal channel infrastructure → DONE
3. Extensive stress testing to verify desync is eliminated
4. Consider closing issue if stress tests pass

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
