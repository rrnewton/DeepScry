# Shadow State and CardID Allocation in MTG Forge-rs

## Overview

The MTG Forge-rs networking system uses a **shadow state** architecture where clients maintain a partial view of the game state without seeing hidden information. This document explains how shadow state works and how CardIDs are allocated to maintain synchronization between client and server.

---

## Quick Answer: How Does `init_game` Work if Decks are Remote?

**The decks aren't remote!** This is the key insight:

The **server sends the opponent's full deck list** in the `GameStarted` message:

```rust
ServerMessage::GameStarted {
    opponent_decklist: Some(DeckListInfo {
        main_deck: vec![
            ("Lightning Bolt", 4),
            ("Mountain", 8),
            // ... all cards in opponent's deck
        ],
        // ...
    }),
    // ...
}
```

The client then uses this deck list + its own local deck to call `GameInitializer::init_game()`:

```rust
// Client knows both decks now:
let our_deck = self.our_deck.as_ref()?;              // Loaded locally before connecting
let opponent_deck = opponent_decklist.to_deck_list(); // Received from server

// Both use the SAME GameInitializer with SAME deck order
// This produces IDENTICAL CardID allocations on both sides
let initializer = GameInitializer::new(card_db);
let game = initializer.init_game(p1_deck, p2_deck, ...)?;
```

**Result**: Both server and client create identical game state with matching CardIDs, even though the client doesn't know the **shuffle order** (which gets revealed via `CardRevealed` messages).

---

## What is Shadow State?

**Shadow state** is the client's representation of the game state. It mirrors the server's game state but with critical differences:

1. **Deck composition is known, but shuffle order is not** - Client has all card names/counts but not shuffle order
2. **Libraries use `LibraryMode::Remote`** - Only the SIZE is tracked, not card order
3. **Reveal queue tracks drawn cards** - When server reveals a card to client, it gets queued in `pending_reveals`
4. **Opponent's hand is tracked by count + revealed cards** - Known cards tracked individually, unknown cards tracked via `hidden_card_count`
5. **Only public information is visible** - Client sees own hand + public zones (Battlefield, Graveyard, Stack, Exile)

### Key Insight: Composition vs. Order

```rust
// Client's knowledge:
// ✅ KNOWS: Opponent deck has 4x Lightning Bolt, 8x Mountain, etc. (from GameStarted)
// ❌ DOESN'T KNOW: Which order they're shuffled in

// Library state during game:
// Server side: Full card order known
LibraryMode::Local {
    cards: vec![CardId(42), CardId(17), CardId(93), ...]  // Shuffled order known
}

// Client side: Order unknown, but all CardIDs are allocated
LibraryMode::Remote {
    size: 60,                              // Know there are 60 cards
    pending_reveals: VecDeque::new()       // Will be filled when server reveals
}

// But the CardIds in that library (42, 17, 93) are KNOWN because they were
// allocated deterministically by GameInitializer using the deck composition
```

---

## Hidden Cards and Unknown Card IDs

### The Problem

When an opponent draws a card into their hand (hidden from us), we **don't immediately allocate a CardID for it**. We only know:
- The count increased by 1
- We don't know what card it is

So where's the CardID?

### The Solution: No CardID Until Reveal

**There is no CardID for unrevealed cards.** Instead:

1. **Hidden cards are tracked by count only** in `CardZone::hidden_card_count`
2. When the opponent's hand moves from 3 → 4 cards, we log `HiddenDraw` action
3. This logs an action with `owner: opponent_id` but **no card_id field**
4. The hand's `hidden_card_count` increments from 3 → 4

### Code Example

From `mtg-engine/src/game/state.rs`:

```rust
// Server draws a card (always known)
GameAction::MoveCard {
    card_id: CardId(42),
    from_zone: Zone::Library,
    to_zone: Zone::Hand,
    owner: opponent_id,
}

// Client draws a card into hidden opponent hand
GameAction::HiddenDraw {
    player_id: opponent_id,
}
// This action keeps action_count in sync without revealing the card
// The library size decrements and hand.hidden_card_count increments
```

---

## How CardIDs Are Allocated

### Timing: When Cards Get IDs

CardIDs are allocated **at game initialization**, not on-demand. This is critical for deterministic synchronization.

### Deterministic Allocation Process

Both **server and client independently allocate identical CardIDs** by:

1. **Loading the same decks in the same order**
2. **Using identical shuffling seeds**
3. **Calling `next_card_id()` in the same sequence**

### In Detail: `GameInitializer::init_game`

From `mtg-engine/src/loader/game_init.rs`:

```rust
/// Initialize a two-player game from two decks
pub async fn init_game(
    &self,
    player1_name: String,
    player1_deck: &DeckList,
    player2_name: String,
    player2_deck: &DeckList,
    starting_life: i32,
) -> Result<GameState> {
    // Step 1: Pre-load all unique cards (deterministic sorting ensures identical order)
    let mut card_names: Vec<String> = unique_cards.into_iter().collect();
    card_names.sort();  // Deterministic ordering
    self.card_db.load_cards(&card_names).await?;

    // Step 2: Create game with capacity hint
    let mut game = GameState::new_two_player_with_capacity(
        player1_name, 
        player2_name,
        starting_life,
        total_cards  // Pre-size EntityStore
    );

    // Step 3: Load decks sequentially - cards allocated in order
    self.load_deck_into_game(&mut game, player1_id, player1_deck).await?;
    self.load_deck_into_game(&mut game, player2_id, player2_deck).await?;

    Ok(game)
}

/// Load a deck into a player's library
async fn load_deck_into_game(
    &self,
    game: &mut GameState,
    player_id: PlayerId,
    deck: &DeckList
) -> Result<()> {
    for entry in &deck.main_deck {
        let card_def = self.card_db.get_card(&entry.card_name).await?;

        // Create the requested number of copies
        for _ in 0..entry.count {
            let card_id = game.next_card_id();  // ← CardID allocated here
            let card = card_def.instantiate(card_id, player_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(player_id).library.add(card_id);
        }
    }
    Ok(())
}
```

### Key Property: Identical IDs on Both Sides

Since both server and client:
- Start with `next_id = 2` (after creating 2 players)
- Load cards in identical order (sorted)
- Instantiate decks in same sequence (P1 first, then P2)
- Call `next_card_id()` for each card copy

**They allocate identical CardIDs for identical cards.**

Example:
```
Server P1 deck: ["Lightning Bolt", "Mountain", "Lightning Bolt"]
Client P1 deck: ["Lightning Bolt", "Mountain", "Lightning Bolt"]

Server allocation:
  CardId(2): Lightning Bolt
  CardId(3): Mountain
  CardId(4): Lightning Bolt

Client allocation (after GameInitializer):
  CardId(2): Lightning Bolt  ← Same!
  CardId(3): Mountain        ← Same!
  CardId(4): Lightning Bolt  ← Same!
```

---

## Client Initialization: `wait_for_game_start`

When the client connects and receives `GameStarted`, it performs this critical sequence:

### Phase 1: Server Sends `GameStarted` with Opponent's Deck List

The server **always sends the opponent's full deck list** in the `GameStarted` message:

```rust
ServerMessage::GameStarted {
    your_player_id: PlayerId,
    opponent_name: String,
    opening_hand: Vec<CardReveal>,          // Our 7 cards
    opponent_hand_count: usize,              // 7
    library_size: usize,                     // 53 (60 - 7 drawn)
    opponent_library_size: usize,            // 53
    opponent_decklist: Some(DeckListInfo {   // ← FULL OPPONENT DECK
        main_deck: vec![
            ("Lightning Bolt".to_string(), 4),
            ("Mountain".to_string(), 8),
            // ... all 60 cards listed
        ],
        sideboard: vec![],
        main_deck_size: 60,
        sideboard_size: 0,
    }),
    starting_life: 20,
    initial_state_hash: 12345,
    network_debug: false,
}
```

### Phase 2: Client Creates Matching Game State

The client extracts the opponent deck and creates a **matching shadow game**:

```rust
pub async fn wait_for_game_start(&mut self) -> Result<()> {
    let ServerMessage::GameStarted {
        your_player_id,
        opponent_name,
        opponent_decklist,
        library_size,
        opponent_library_size,
        starting_life,
        ..
    } = msg else { /* ... */ };

    // Step 1: Get opponent's deck list (sent by server)
    let opponent_deck = opponent_decklist.ok_or_else(||
        anyhow!("Server did not send opponent deck list - cannot synchronize card IDs")
    )?;
    let opponent_deck = opponent_deck.to_deck_list();

    // Step 2: Get our own deck (loaded locally before connecting)
    let our_deck = self.our_deck.as_ref()
        .ok_or_else(|| anyhow!("Our deck not loaded"))?;

    // Step 3: Determine player order and build arguments for GameInitializer
    let we_are_p1 = our_player_id.as_u32() == 0;
    let our_name = self.config.player_name.clone();
    let (p1_deck, p2_deck, p1_name, p2_name) = if we_are_p1 {
        (our_deck, &opponent_deck, our_name, opponent_name.clone())
    } else {
        (&opponent_deck, our_deck, opponent_name.clone(), our_name)
    };

    // Step 4: Create game with IDENTICAL CardID allocation as server
    // Both server and client use the same GameInitializer with same deck order
    let initializer = GameInitializer::new(card_db);
    let mut game = initializer
        .init_game(p1_name, p1_deck, p2_name, p2_deck, starting_life)
        .await?;

    // Step 5: Convert libraries to Remote mode (we don't know shuffle order)
    // We know the deck COMPOSITION but not the SHUFFLE ORDER
    if let Some(zones) = game.get_player_zones_mut(our_player_id) {
        zones.library = CardZone::new_remote_library(our_player_id, library_size);
    }
    if let Some(zones) = game.get_player_zones_mut(opponent_id) {
        zones.library = CardZone::new_remote_library(opponent_id, opponent_library_size);
    }

    // Step 6: Create ClientGameState with the initialized game
    self.state = Some(ClientGameState {
        game,
        our_player_id,
        opponent_id,
        known_cards: HashMap::new(),
        expected_hash: initial_state_hash,
        opponent_name: opponent_name.clone(),
        choice_seq: 0,
    });

    Ok(())
}
```

### Phase 3: Client Receives Opening Hand Reveals

After the shadow game is created, the server sends `CardRevealed` messages for all opening hand cards:

```rust
// Server sends opening hand reveals before GameLoop starts
// Client receives and queues them
while reveals_received < expected_reveals {
    let ServerMessage::CardRevealed { owner, card, reason } = self.receive_message().await?;
    
    // Queue the reveal in the library's pending_reveals buffer
    // The GameLoop with skip_opening_hands will drain these and draw the cards
    if let Some(zones) = state.game.get_player_zones_mut(owner) {
        zones.library.queue_reveal(card.card_id);  // ← Card IDs are known!
    }
    reveals_received += 1;
}
```

---

## How Reveals Work: The Queue System

### The `LibraryMode::Remote` Buffer

Remote libraries have a `pending_reveals` queue:

```rust
pub enum LibraryMode {
    Remote {
        size: usize,
        pending_reveals: VecDeque<CardId>,  // ← Queue of revealed cards
    }
}
```

### Server → Client Reveal Flow

1. **Server draws a card** (e.g., opponent draws from library)
   ```rust
   GameAction::MoveCard {
       card_id: CardId(42),
       from_zone: Zone::Library,
       to_zone: Zone::Hand,
       owner: opponent_id,
   }
   ```

2. **Server sends `CardRevealed` message**
   ```rust
   ServerMessage::CardRevealed {
       owner: opponent_id,
       card: CardReveal { card_id: CardId(42), ... },
       reason: RevealReason::Draw,
   }
   ```

3. **Client receives and queues the reveal**
   ```rust
   match server_msg {
       ServerMessage::CardRevealed { owner, card, .. } => {
           if let Some(zones) = game.get_player_zones_mut(owner) {
               zones.library.queue_reveal(card.card_id);
           }
       }
   }
   ```

4. **When client's GameLoop draws from library**
   ```rust
   pub fn draw_card(&mut self, player_id: PlayerId) -> Result<Option<CardId>> {
       // For remote libraries, pop from pending_reveals queue
       if let Some(zones) = self.get_player_zones_mut(player_id) {
           if let Some(LibraryMode::Remote { pending_reveals, .. }) = &zones.library_mode {
               if let Some(card_id) = pending_reveals.pop_front() {
                   return Ok(Some(card_id));
               }
           }
       }
   }
   ```

### Critical: Opening Hand Reveals

The server sends `CardRevealed` messages for opening hands **before** the game loop runs:

```rust
// In server.rs run_game():

// Send opening hand to both players
p1_conn.send(&ServerMessage::GameStarted { opening_hand: p1_hand, .. }).await?;
p2_conn.send(&ServerMessage::GameStarted { opening_hand: p2_hand, .. }).await?;

// Send CardRevealed for opening hands so clients can queue them
for card in &p1_hand {
    p1_conn.send(&ServerMessage::CardRevealed {
        owner: p1_id,
        card,
        reason: RevealReason::Draw,
    }).await?;
}
for card in &p2_hand {
    p1_conn.send(&ServerMessage::CardRevealed {
        owner: p2_id,
        card,
        reason: RevealReason::Draw,
    }).await?;
}

// Set baseline reveal index to skip these already-sent reveals
p1_conn.last_reveal_index = opening_hand_count;
p2_conn.last_reveal_index = opening_hand_count;

// Now GameLoop runs and draws from the queued reveals
```

---

## Unknown Cards in Practice

### Scenario 1: Opponent Draws (Hidden from Client)

```
Server state:
  Opponent has: [CardId(42), CardId(17), CardId(93), ...]  in hand

Client state:
  Opponent hand: {
      cards: [CardId(42), CardId(17), CardId(93)],  // Revealed cards
      hidden_card_count: 3                           // Plus 3 more cards we don't know
  }
  Total opponent hand size = 3 + 3 = 6
```

When opponent draws a card into hand (not revealed):
```rust
// Server logs
GameAction::MoveCard {
    card_id: CardId(105),           // Real card ID, unknown to client
    from_zone: Zone::Library,
    to_zone: Zone::Hand,
    owner: opponent_id,
}

// Client cannot see CardId(105), so it logs
GameAction::HiddenDraw { player_id: opponent_id }

// Client state updates:
// - opponent_library.size: 60 → 59
// - opponent_hand.hidden_card_count: 3 → 4
```

### Scenario 2: Opponent Reveals a Card

If opponent plays a card from hand, the server sends `CardRevealed`:

```rust
ServerMessage::CardRevealed {
    owner: opponent_id,
    card: CardReveal {
        card_id: CardId(105),
        name: "Counterspell",
        ...
    },
    reason: RevealReason::Cast,
}
```

Now the client knows CardId(105) = Counterspell.

---

## Synchronization Guarantees

### What Stays in Sync

- **action_count**: Number of game actions performed (same on both sides)
- **state_hash**: Checksum of public information (size of zones, life totals, public permanents)
- **Card allocations**: Deterministic assignment ensures matching CardIDs for same cards
- **Turn count and phase**
- **Public zone contents** (Battlefield, Graveyard, Stack, Exile)

### What Doesn't Sync

- **Deck order** (shuffled with server seed, order unknown to client)
- **Opponent's hand contents** (hidden until revealed)
- **Opponent's library contents** (hidden until revealed or milled)
- **Face-down cards** (identity hidden)

---

## Why CardIDs Matter

### The Design Choice: Pre-allocate All CardIDs

Why allocate all CardIDs at initialization rather than on-demand when revealed?

**Pros:**
- ✅ Enables deterministic CardID allocation
- ✅ No synchronization race conditions
- ✅ Clients can create shadow state independently
- ✅ Simple, predictable behavior

**Cons:**
- ❌ Uses more memory (CardIds exist for all 120 cards upfront)
- ❌ Requires sending opponent's decklist to client (needed for GameInitializer)

### The Alternative (Not Used)

Could we allocate CardIDs on-demand when revealed?
- Would require server to assign CardIDs and send them with reveals
- More complex state machine
- Risk of timing issues where server assigns same ID twice

---

## Network Messages and Reveals

### `GameStarted` Message

Sent when a game is ready to begin:

```rust
ServerMessage::GameStarted {
    your_player_id: PlayerId,
    opponent_name: String,
    opening_hand: Vec<CardReveal>,          // 7 cards we drew
    opponent_hand_count: usize,              // 7, but unknown
    library_size: usize,                    // 53 (60 - 7 drawn)
    opponent_library_size: usize,           // 53 (60 - 7 drawn)
    opponent_decklist: Option<DeckListInfo>, // Full deck list
    starting_life: i32,                     // 20
    initial_state_hash: u64,                // For validation
    network_debug: bool,
}
```

### `CardRevealed` Message

Sent when a card becomes visible:

```rust
ServerMessage::CardRevealed {
    owner: PlayerId,
    card: CardReveal {
        card_id: CardId,
        name: String,
        mana_cost: Option<ManaCost>,
        types: Vec<CardType>,
        subtypes: Vec<String>,
        power: Option<i32>,
        toughness: Option<i32>,
        loyalty: Option<i32>,
    },
    reason: RevealReason,  // Draw, Cast, Discard, etc.
}
```

### `ChoiceRequest` Message (Includes Reveals)

When the server needs a choice from the player:

```rust
ServerMessage::ChoiceRequest {
    choice_seq: u32,
    for_player: PlayerId,
    choice_type: ChoiceType,
    options: Vec<ChoiceOption>,
    reveals: Vec<CardRevealInfo>,  // ← Bundled reveals that occurred
    state_hash: u64,
    action_count: u64,
    timestamp_ms: u64,
}
```

Reveals are bundled **with the choice request** (not sent async) to ensure strict ordering.

---

## Summary

| Aspect | Details |
|--------|---------|
| **CardID Allocation** | At game init, deterministic via GameInitializer |
| **Server CardIDs** | All 120 cards allocated upfront |
| **Client CardIDs** | All 120 cards allocated identically via GameInitializer |
| **Unknown Cards** | No CardID until revealed; tracked by count only |
| **Hidden Hand** | Tracked via `hidden_card_count` field |
| **Hidden Library** | Tracked via `LibraryMode::Remote` with size |
| **Reveal Mechanism** | Server sends `CardRevealed` messages; client queues in `pending_reveals` |
| **Opening Hands** | Pre-sent before GameLoop runs; guaranteed to be queued |
| **Sync Method** | Both sides independently create identical game state from decks |
| **Validation** | action_count and state_hash verify sync at each choice |

---

## See Also

- `mtg-engine/src/zones.rs` - `LibraryMode` and `CardZone` definitions
- `mtg-engine/src/loader/game_init.rs` - CardID allocation logic
- `mtg-engine/src/network/client.rs` - Client-side shadow state setup
- `mtg-engine/src/network/server.rs` - Server reveal broadcast logic
- `mtg-engine/src/undo.rs` - `HiddenDraw` and `HiddenDiscard` action types
