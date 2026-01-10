---
title: Network multiplayer infrastructure (dormant)
status: open
priority: 4
issue_type: task
created_at: 2026-01-07T15:45:56.797997299+00:00
updated_at: 2026-01-07T15:45:56.797997299+00:00
---

# Description

## Network Multiplayer Infrastructure (Dormant)

The network multiplayer code exists but is currently **dormant** - not exposed via CLI commands.

## Current State

- **Feature flag**: `network` cargo feature (NOT in default features)
- **CLI commands removed**: `server` and `connect` subcommands no longer exist in the CLI
- **Code location**: `mtg-engine/src/network/` module (client, server, controllers)
- **Default features**: `["native", "verbose-logging"]` - network NOT included

## Known Issues (to address when re-enabled)

### 1. Nondeterministic Desync (action 760)
- **Symptom**: Clients occasionally report different state hashes around action 760
- **Cause**: Unknown - may be related to hidden information enforcement or RNG sync
- **To investigate**: Enable `--network-debug` flag for detailed state comparison

### 2. WebSocket Shutdown Handshake
- **Issue**: Server doesn't gracefully shutdown WebSocket connections
- **Impact**: Clients may receive connection errors instead of clean game end signals
- **Solution needed**: Implement proper shutdown handshake protocol

### 3. Winner Signal Race Condition (FIXED)
- **Fixed in commit**: 05737b3d (fix(network): Increase winner signal timeout)
- **Was**: 100ms timeout in `run_game_sync()` caused false draws
- **Now**: 5 second timeout matching async version

## To Re-enable Network Play

1. Build with network feature: `cargo build --release --features network`
2. Add CLI subcommands back to main.rs for `server` and `connect`
3. Test with: `--network-debug` flag for detailed logging

## Files

- `mtg-engine/src/network/mod.rs` - Module root with feature gate
- `mtg-engine/src/network/client.rs` - Client implementation
- `mtg-engine/src/network/server.rs` - Server implementation
- `mtg-engine/src/network/controller.rs` - Network controller trait
- `mtg-engine/Cargo.toml` - Feature flag definitions

## Priority

This is a low priority (4) tracking issue for when network play becomes a focus again.
