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

### Reveal Architecture (TODO - mtg-hbt5i)
- [ ] Move reveal generation to core GameLoop (not server handlers)
- [ ] Use `GameAction::RevealCard` with `revealed_to` field (P1, P2, or BOTH)
- [ ] Reveals logged BEFORE moves (reveal before first use)
- [ ] Deduplication happens at log time (skip if already revealed to target)
- [ ] Remove `collect_reveals_since_last_choice()` from NetworkController
- [ ] Remove reveal bundling from ChoiceRequest
- [ ] Remove `revealed_cards` HashSet from PlayerConnection
- [ ] Server reads RevealCard from action log, forwards to clients

### Code Violations to Fix

#### server.rs
- [ ] `collect_reveals_since_last_choice()` scans undo_log in handler - WRONG
  - Should read RevealCard actions from log, not infer from MoveCard
- [ ] `revealed_cards: HashSet<CardId>` in PlayerConnection - WRONG
  - Deduplication belongs in core engine, not handler
- [ ] `send_reveal_if_new()` - WRONG
  - Reveals should be deterministic game actions, not handler decisions
- [ ] Opening hand reveals sent manually - WRONG
  - Should be RevealCard actions in the log from GameLoop

#### controller.rs
- [ ] `collect_reveals_since_last_choice()` - WRONG
  - NetworkController shouldn't compute reveals from MoveCard
  - Should forward RevealCard actions from log
- [ ] `shared_reveal_index` coordination - WRONG
  - No need for coordination if reveals are in the log

#### game_loop/
- [ ] No `GameAction::RevealCard` being logged on card moves - WRONG
  - Core engine should log reveals before moves
- [ ] `reveal_pusher` callback never used - should be removed or repurposed
- [ ] `reveal_drainer` architecture - review if still needed

#### client.rs
- [ ] `drain_reveals` processing - review timing
  - With proper reveal ordering, should be simpler
- [ ] Reveal validation happening before reveals arrive - timing issue
  - Proper reveal-before-move ordering should fix this

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
