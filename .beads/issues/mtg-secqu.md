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
- [x] Enable skip_reveals=false for network games (server.rs)
  - In-game reveals now properly logged and collected
- [ ] Remove reveal bundling from ChoiceRequest (LOW PRIORITY - working correctly)
  - Current flow: NetworkController collects from log → bundles in ChoiceRequest →
    server handler sends CardRevealed → client processes via drain_reveals
  - This indirection is necessary because GameLoop runs in spawn_blocking
- [x] `revealed_cards` HashSet was already removed from PlayerConnection
- [~] Opening hand reveals use `peek_opening_hand()` (TIMING CONSTRAINT)
  - See "Opening Hand Reveal Timing" section below

### Code Violations Status

#### server.rs
- [x] `revealed_cards: HashSet<CardId>` - REMOVED (not present in code)
- [x] `send_reveal_if_new()` - REMOVED (not present in code)
- [x] `skip_reveals=false` for network games - ENABLED
- [~] Opening hand reveals - use `peek_opening_hand()` (timing constraint, see above)

#### controller.rs
- [x] `collect_reveals_since_last_choice()` - FIXED
  - Now reads RevealCard actions from log, not MoveCard
- [x] `shared_reveal_index` - still in place for coordination (needed for deduplication)

#### game_loop/
- [x] GameAction::RevealCard now logged on card moves
  - draw_card, mill_cards, play_land, cast_spell, cast_spell_8_step
- [x] RevealCard includes `old_mask` field for undo support
  - Always use RevealCard (SetRevealedToMask is DEPRECATED)
  - Undo restores the previous revealed_to_mask value
- [x] Added helper functions: `maybe_reveal_to_player()` and `maybe_reveal_to_all()`
- [ ] `reveal_pusher` callback - unused, can be removed (LOW PRIORITY)
- [x] `reveal_drainer` architecture - NEEDED for client to process CardRevealed

#### client.rs
- [x] `skip_reveals=false` enabled for network games - matches server logs
- [x] `drain_reveals` processing - working correctly for late-binding architecture
- [x] Reveal validation timing - works with current ChoiceRequest flow

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

## Opening Hand Reveal Timing

Opening hand reveals use `peek_opening_hand()` instead of RevealCard for a specific timing reason:

**The constraint**: Client's `drain_reveals()` must process opening hand CardRevealed messages
BEFORE the client's GameLoop starts drawing. Otherwise, `draw_card()` would fail because
Card instances don't exist yet in `game.cards`.

**Current flow**:
1. Server: `peek_opening_hand()` looks at top 7 cards (no mutation)
2. Server: Sends `CardRevealed` for all 14 opening hand cards
3. Client: Receives `CardRevealed`, queues in `reveal_tx`
4. Client: Starts GameLoop with `skip_opening_hands()`
5. Client: GameLoop calls `drain_reveals()` → processes queued reveals → creates Card instances
6. Client: GameLoop calls `draw_card()` × 14 → works because Cards exist

**Why RevealCard is difficult**:
- If server draws inside GameLoop first, CardRevealed messages arrive AFTER client's GameLoop starts
- Client's `drain_reveals()` might run before reveals arrive → empty queue → `draw_card()` fails
- Both GameLoops run in parallel (ish), making synchronization complex

**Possible future improvements**:
1. Add post-setup callback to GameLoop that sends opening hand reveals before main loop
2. Have client's `drain_reveals()` block-wait for opening hand reveals (complex)
3. Add explicit "opening hands ready" synchronization signal

For now, `peek_opening_hand()` is a pragmatic solution that works reliably. The key insight:
opening hand reveals are sent BEFORE either GameLoop starts, ensuring they're queued in time.

In-game reveals (draws, plays, etc.) properly use RevealCard architecture because they happen
DURING GameLoop execution where the timing is controlled by ChoiceRequest/ChoiceAccepted flow.

## Related Issues
- mtg-hbt5i: Shadow state desync (blocked by this)
- mtg-to96y: Main networking tracking issue
- mtg-qtqcr: Hidden information architecture
- mtg-e66iz: Original desync bug (dormant)
