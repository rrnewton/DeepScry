---
title: 'Network architecture compliance tracking'
status: open
priority: 1
issue_type: tracking
created_at: 2026-01-08T01:50:48.341942482+00:00
updated_at: 2026-01-08T22:30:31.481711838+00:00
---

# Description

Tracking issue for compliance with `docs/NETWORK_ARCHITECTURE.md`.

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
  - draw_card, mill_cards, play_land, cast_spell, cast_spell_8_step now log RevealCard
- [x] Use `GameAction::RevealCard` with `revealed_to` field (P1, P2, or BOTH)
  - Added `RevealTarget` enum: `Player(PlayerId)` or `All`
- [x] Reveals logged BEFORE moves (reveal before first use)
  - RevealCard logged before MoveCard in all updated functions
- [x] Deduplication happens at log time (skip if already revealed to target)
  - Added `revealed_to_mask: u8` field to Card struct
  - `is_revealed_to()`, `mark_revealed_to()` helper methods
- [x] Update `collect_reveals_since_last_choice()` in NetworkController
  - Now reads RevealCard actions from log, not infers from MoveCard
- [ ] Remove reveal bundling from ChoiceRequest (still used, but populated from RevealCard)
- [x] `revealed_cards` HashSet was already removed from PlayerConnection
- [ ] Opening hand reveals still need updating to use RevealCard

### Code Violations Status

#### server.rs
- [x] `revealed_cards: HashSet<CardId>` - REMOVED (not present in code)
- [x] `send_reveal_if_new()` - REMOVED (not present in code)
- [ ] Opening hand reveals - still sent manually, needs RevealCard migration

#### controller.rs
- [x] `collect_reveals_since_last_choice()` - FIXED
  - Now reads RevealCard actions from log, not MoveCard
- [ ] `shared_reveal_index` - still in place for coordination, may simplify later

#### game_loop/
- [x] GameAction::RevealCard now logged on card moves
  - draw_card, mill_cards, play_land, cast_spell, cast_spell_8_step
- [x] Added `SetRevealedToMask` action for undo support when card already exists
- [x] Added helper functions: `maybe_reveal_to_player()` and `maybe_reveal_to_all()`
- [ ] `reveal_pusher` callback - still unused, review if needed
- [ ] `reveal_drainer` architecture - review if still needed

#### client.rs
- [ ] `drain_reveals` processing - may be simplified now
- [ ] Reveal validation timing - should be improved with proper ordering

## Architecture Principles (from docs/NETWORK_ARCHITECTURE.md)

### What's PROHIBITED
- Sleeps or retries (indicates protocol bug)
- `select!` over multiple channels (nondeterminism)
- Parallel message processing (violates sequential model)
- Reveal logic in server handlers (belongs in core engine)

### Correct Flow
```
GameLoop logs: RevealCard → MoveCard → ...
Server reads log, sends: CardRevealed → ...
Client receives in order, processes sequentially
```

## Related Issues
- mtg-hbt5i: Shadow state desync (blocked by this)
- mtg-to96y: Main networking tracking issue
- mtg-qtqcr: Hidden information architecture
- mtg-e66iz: Original desync bug (dormant)
