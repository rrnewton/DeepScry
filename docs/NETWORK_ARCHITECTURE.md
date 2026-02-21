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

## CRITICAL: Desync is ALWAYS a Fatal Error

**Any desynchronization between server and client is an immediate, fatal error.**

This principle is absolute and admits NO exceptions:

1. **Never paper over desync** - If client and server have different views of the
   game state, the correct response is to crash with a clear error message, NOT
   to silently "fix" the discrepancy with recovery heuristics.

2. **No half-working hacks** - We have NO interest in code that "stumbles along
   for a few more turns" in a desynced state. Such code masks bugs and makes
   debugging nearly impossible.

3. **Desync means a bug exists** - When desync is detected, the correct action is:
   - Log detailed diagnostic information (state hashes, action counts, choice indices)
   - Terminate the game immediately with `FATAL ERROR: DESYNC DETECTED`
   - File a bug report with reproduction steps

4. **Validation, not recovery** - Extra data sent in messages (like `spell_ability`
   in `ChoiceResponse`) is for **validation and early detection only**. If validation
   fails, we crash immediately - we do NOT use the extra data to "recover" from
   inconsistent state.

### Why This Matters

The deterministic simulation model is the foundation of network correctness. If we
allow recovery hacks:

- Bugs become invisible (game continues despite corruption)
- State corruption compounds (one wrong choice leads to cascading errors)
- Debugging becomes impossible (the "fix" obscures the original cause)
- Trust in the system erodes (players experience random failures)

By crashing immediately on desync, we:

- Catch bugs early with clear error messages
- Get reproducible failure cases
- Maintain trust in the correctness of successful games
- Keep the codebase simple (no complex recovery logic)

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

## CRITICAL: Controllers Must Be Information-Independent

**All controllers (heuristic, random, zero, etc.) MUST produce identical decisions
whether running on the server (full state) or on a client (shadow state).**

This is a direct consequence of the deterministic simulation model:

1. **Controllers must NEVER depend on hidden information** - opponent hand contents,
   library order, or RNG state are not legally visible. A controller that reads this
   information and uses it to make decisions is cheating AND will cause network desync.

2. **Any divergence is a bug** - If a controller produces different gamelogs when run
   locally (where GameStateView exposes everything) vs in network mode (where shadow
   state hides opponent cards), the controller has an information-leakage bug.

3. **GameStateView exposes more than it should in local mode** - Methods like
   `player_hand(opponent_id)` return real data locally but empty data on clients.
   Controllers must not call these methods for opponents, or must handle the empty
   case identically to having data.

4. **Testing requirement** - The network vs local equivalence E2E test
   (`tests/network_vs_local_equivalence_e2e.sh`) validates gamelog identity across
   ALL controller types. This is not optional - it catches info-leakage bugs.

## Testing Requirements: Always Use `--network-debug`

When testing any network functionality, **always** launch the server with `--network-debug`:

```bash
mtg server --port 17771 --password test --network-debug
```

The `--network-debug` flag enables **full state hash validation** after every choice:
- Server computes a hash of the game state after applying each choice
- Client computes the same hash independently and sends it with each `ChoiceResponse`
- Server validates the hashes match — any mismatch is an immediate fatal error

Without `--network-debug`, the server still runs the game correctly, but state hash
validation is disabled. The cheap integer-comparison checks (ability count, hand size,
discard count) always run in all builds, but these only catch a subset of desyncs.
The full hash check catches **all** state divergences.

**Every test script and launch helper MUST pass `--network-debug` to the server.**
This includes:
- E2E tests (`web/test_network_*.js`, `tests/network_vs_local_equivalence_e2e.sh`)
- Bug-finding infrastructure (`bug_finding/network_test_lib.py`)
- Launch helpers (`scripts/launch_network_game.sh`, `scripts/play-web.sh`)
- Manual testing scripts (`scripts/network_desync_reproducer.sh`)

If you create a new test or script that launches a network server, add `--network-debug`.
If you find an existing script without it, that's a bug — fix it.

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
