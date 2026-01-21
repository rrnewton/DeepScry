---
title: WASM network random test intermittent hangs
status: open
priority: 2
issue_type: bug
depends_on:
  mtg-byq4z: parent-child
created_at: 2026-01-20T10:20:18.995281594+00:00
updated_at: 2026-01-21T10:01:54.202722011+00:00
---

# Description

## WASM Network Sync Fix

## Root Cause

The WASM network client was receiving CardRevealed messages but NOT processing them into the local game state. The reveals were queued in `pending_reveals` but never instantiated as actual card objects in the GameState.

This caused desync because:
1. Server sends CardRevealed with card definitions
2. WASM client queues the reveal
3. Game loop makes choices based on local state
4. Local state has no cards (empty game state)
5. Server has cards (proper state)
6. Desync: choice indices don't match

## Fix

Added `sync_callback` to the WASM network game loop (matching native client architecture):

1. Created `process_card_reveal_wasm()` function in `fancy_tui.rs`:
   - Handles Draw, OpeningHand, Played, TokenCreated reveal types
   - Instantiates cards using embedded `card_def` from server
   - Panics if server doesn't provide card_def (desync detection)

2. Added sync_callback to all network game loops:
   - `run_network_mode_ai()` - for Random/Heuristic/Zero controllers
   - `run_network_mode_human()` - both replay and normal paths

The sync_callback drains pending reveals and calls `process_card_reveal_wasm()` before each choice point, keeping local state synchronized with server.

## Verification

- `make validate`: 822 tests passed
- `test_network_random_e2e.js`: Game progressed through 21+ choices without desync errors

## Related Architecture

Per NETWORK_ARCHITECTURE.md:
- Clients maintain "shadow" game states synchronized with server
- CardRevealed messages instantiate cards in local state
- Desync is ALWAYS a fatal error - never paper over it
