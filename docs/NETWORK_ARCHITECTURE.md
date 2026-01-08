# Network Architecture

This document describes the fundamental principles of the MTG network multiplayer
architecture. These principles are **inviolable** - any code that violates them
is a bug.

## Core Principle: Deterministic Sequential Simulation

The MTG game is a **deterministic state machine**. Given the same initial state
(decks, RNG seed) and the same sequence of player choices, the game will always
produce identical results.

In network multiplayer, this state machine is **split across machines**:
- One server
- Two clients (one per player)

Despite being distributed, the simulation remains **sequential**:
- Control transfers linearly from server to client (for choices) and back
- There is never parallel or concurrent execution of game logic
- At any moment, exactly ONE entity has "control"

## Replicated Game State

All three parties (server + 2 clients) maintain a copy of the game state:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         GAME STATE                                  │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐             │
│  │   Server    │    │  Client 1   │    │  Client 2   │             │
│  │  (golden)   │    │  (shadow)   │    │  (shadow)   │             │
│  │             │    │             │    │             │             │
│  │ Full state  │    │ Sees own    │    │ Sees own    │             │
│  │ No hidden   │    │ cards only  │    │ cards only  │             │
│  │ info        │    │             │    │             │             │
│  └─────────────┘    └─────────────┘    └─────────────┘             │
└─────────────────────────────────────────────────────────────────────┘
```

**Server**: Has the "golden" copy with full visibility (no hidden information).
Used to validate client states and resolve disputes.

**Clients**: Have "shadow" copies where opponent's hidden information (hand cards,
library order) is not revealed until game rules require it.

The states must remain identical except for:
1. Hidden information not yet revealed
2. The PlayerController implementations (local vs remote)

## The Action Log (Undo Log)

The game maintains a sequential **action log** (implemented as `undo_log`) that
records every game action:

```rust
enum GameAction {
    MoveCard { card_id, from_zone, to_zone, owner },
    RevealCard { card_id, name, revealed_to },  // WHO sees this reveal
    ChoicePoint { player_id, choice_type, ... },
    TapCard { card_id },
    // ... etc
}
```

This log is **deterministic** - given identical inputs, all parties produce
identical logs. It functions like a blockchain: an agreed-upon sequential
history of all game events.

### RevealCard Actions

Card reveals are **first-class game actions** in the action log. When a card's
identity needs to be known (draw, play, mill to graveyard, etc.), the engine
logs a `RevealCard` action BEFORE any downstream actions that depend on knowing
the card.

Key properties of RevealCard:
- **Target audience**: Each reveal specifies WHO sees it (P1, P2, or both)
- **Deduplication**: If a card was already revealed to a player, no redundant
  reveal is logged for that player
- **Ordering**: Reveals appear in the log BEFORE actions that depend on them

Example sequence when P1 draws a card:
```
1. RevealCard { card_id: 42, name: "Lightning Bolt", revealed_to: P1 }
   // P2 doesn't see this - hand is hidden
2. MoveCard { card_id: 42, from: Library, to: Hand, owner: P1 }
```

Example when a card goes to battlefield (public zone with ETB triggers):
```
1. RevealCard { card_id: 42, name: "Serra Angel", revealed_to: BOTH }
   // Everyone sees cards entering public zones
2. MoveCard { card_id: 42, from: Hand, to: Battlefield, owner: P1 }
   // Move processing can now check ETB triggers, enters-tapped, etc.
```

The reveal MUST come before the move because:
- Move processing may need card identity (ETB triggers, enters-tapped checks)
- Any code touching the card can assume it's already revealed
- Simpler invariant: "reveal before first use"

## The Message History

Communication happens over a single WebSocket per client, creating a
**sequential message history**:

```
Server ←──────────────────────────────────→ Client
        │                                  │
        │  ServerMessage (ordered)         │
        │  ─────────────────────────────►  │
        │                                  │
        │  ClientMessage (ordered)         │
        │  ◄─────────────────────────────  │
        │                                  │
```

Messages are processed in **exact order of arrival**. No reordering is possible
because:
- Single WebSocket = single TCP connection = ordered delivery
- Single channel internally = no race conditions

The server→client message stream is essentially an **ordered, abbreviated subset**
of the action log. When catching up a client before a choice request, the server
reads RevealCard actions from the log and sends corresponding CardRevealed messages.

## Linear Control Transfer

```
┌────────┐         ┌────────┐         ┌────────┐
│ Server │ ──────► │Client 1│ ──────► │ Server │ ──────► ...
│(execute│         │(choose)│         │(execute│
│  game) │         │        │         │  game) │
└────────┘         └────────┘         └────────┘
     │                  │                  │
     ▼                  ▼                  ▼
  Actions            Choice             Actions
  logged             made               logged
```

1. **Server executes game logic** until a player choice is needed
2. **Server sends reveals + ChoiceRequest** to the deciding player's client
3. **Client processes reveals**, then presents choice to player
4. **Client sends ChoiceResponse** back to server
5. **Server applies choice**, sends confirmation, notifies opponent
6. Repeat

At each step, only ONE party is "active" - others are waiting.

## What This Architecture PROHIBITS

### No Sleeps or Retries

```rust
// WRONG - violates linear model
loop {
    if data_available() { break; }
    sleep(10ms);  // NO! This indicates a protocol bug
}
```

If you find yourself wanting to sleep/retry, it means messages are arriving
in the wrong order. Fix the protocol, don't paper over it.

### No Select Over Multiple Channels

```rust
// WRONG - introduces nondeterminism
tokio::select! {
    msg = channel_a.recv() => { ... }
    msg = channel_b.recv() => { ... }  // Race condition!
}
```

Each party waits on exactly ONE source at a time. The single-channel
architecture ensures this.

### No Parallel Message Processing

```rust
// WRONG - violates sequential processing
spawn(process_reveals());
spawn(process_choices());  // These might race!
```

All messages are processed sequentially in arrival order.

### No Reveal Logic in Server Handlers

```rust
// WRONG - reveals belong in the core engine
fn handle_choice_request() {
    // Scanning undo_log for reveals here is wrong
    let reveals = collect_reveals_from_log();
    send_reveals(reveals);
}
```

Reveals are GameActions logged by the core GameLoop. The server just reads
them from the log and forwards to clients.

## Correct Reveal Architecture

Reveals are generated in the **core GameLoop** as deterministic game actions:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         GAME LOOP (Core Engine)                     │
│                                                                     │
│  1. Card moves from hidden zone (Library, Hand)                     │
│  2. Engine checks: who needs to see this card?                      │
│  3. Engine logs RevealCard action with target audience              │
│  4. Deduplication: skip if already revealed to that audience        │
│  5. Continue with downstream actions                                │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         SERVER                                      │
│                                                                     │
│  1. At choice point, read action log since last choice              │
│  2. Extract RevealCard actions                                      │
│  3. Send CardRevealed messages to appropriate clients               │
│  4. Send ChoiceRequest                                              │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         CLIENT                                      │
│                                                                     │
│  1. Receive CardRevealed messages in order                          │
│  2. Instantiate cards in shadow game state                          │
│  3. Receive ChoiceRequest                                           │
│  4. Present choice to player                                        │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

## Related Issues

- `mtg-secqu`: Single-channel architecture to eliminate select! nondeterminism
- `mtg-hbt5i`: Shadow state desync debugging
- `mtg-qtqcr`: Hidden information architecture
- `mtg-to96y`: Main networking tracking issue

## Summary

1. **Deterministic simulation** split across machines
2. **Sequential action log** agreed upon by all parties
3. **Sequential message history** via single WebSocket/channel
4. **Linear control transfer** - one active party at a time
5. **No sleeps, retries, or selects** - each party waits for its turn
6. **Reveals are core game actions** - not server handler logic
