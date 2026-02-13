---
title: Network architecture compliance tracking
status: open
priority: 1
issue_type: epic
created_at: 2026-01-08T01:50:48.341942482+00:00
updated_at: 2026-02-13T11:30:40.649636165+00:00
---

# Description

Tracking issue for compliance with `docs/NETWORK_ARCHITECTURE.md`.

## CRITICAL PRINCIPLE: Desync is ALWAYS a Fatal Error

**Any desynchronization between server and client is an immediate, fatal error.**

- **Never paper over desync** - If client and server have different views of the game state, crash with a clear error message, NOT silently "fix" the discrepancy.
- **No half-working hacks** - We have NO interest in code that "stumbles along for a few more turns" in a desynced state.
- **Desync means a bug exists** - The correct action is to log diagnostics and terminate immediately.
- **Validation, not recovery** - Extra data in messages (like `spell_ability`) is for early detection only, NOT for recovering from inconsistent state.

## Core Architecture Principles

The network architecture is based on these inviolable principles:
1. **Deterministic sequential simulation** - state machine split across machines
2. **Replicated game state** - server (golden) + clients (shadow)
3. **Sequential action log** - agreed upon by all parties like a blockchain
4. **Sequential message history** - single WebSocket, no reordering
5. **Linear control transfer** - one active party at a time
6. **No sleeps/retries/selects** - each party waits for its turn
7. **Reveals are core game actions** - not server handler logic

## Compliance Checklist

### Single-Channel Architecture (DONE)
- [x] Define `GameToHandler` and `HandlerToGame` enums in server.rs
- [x] Create single channel pair per player (game_tx, game_rx)
- [x] Remove old multi-channel infrastructure
- [x] Handler loop is sequential (no select!)
- [x] Opponent choices flow through coordinator

### Reveal Architecture (IN PROGRESS - mtg-hbt5i)
- [x] Move reveal generation to core GameLoop (not server handlers)
- [x] Use `GameAction::RevealCard` with `revealed_to` field
- [x] Reveals logged BEFORE moves (reveal before first use)
- [x] Deduplication happens at log time
- [x] Update `collect_reveals_since_last_choice()` in NetworkController
- [x] Enable skip_reveals=false for network games

### NetworkController Choice Methods (DONE - commit 3d66c52bd)
- [x] choose_blocker_for_lethal_damage - SMART damage assignment sync
- [x] choose_blocker_for_remaining_damage - SMART damage assignment sync
- [x] LethalDamageAssignment and RemainingDamageAssignment ChoiceTypes added

### Code Violations Status

#### server.rs
- [x] `revealed_cards: HashSet<CardId>` - REMOVED
- [x] `send_reveal_if_new()` - REMOVED
- [x] `skip_reveals=false` for network games - ENABLED

#### controller.rs
- [x] `collect_reveals_since_last_choice()` - FIXED
- [x] `shared_reveal_index` - in place for coordination

#### game_loop/
- [x] GameAction::RevealCard now logged on card moves
- [x] RevealCard includes `old_mask` field for undo support

## Related Issues
- mtg-hbt5i: Shadow state desync (blocked by this)
- mtg-to96y: Main networking tracking issue
- mtg-qtqcr: Hidden information architecture
- mtg-wsl8g: Network fuzz test bugs
