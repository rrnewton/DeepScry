# Analysis: "Catch Up to Action X" Hook Architecture

## Current Problem

The current pre-choice hook architecture has a fundamental timing issue:

```
Server GameLoop:                           Messages sent:
1. draw_card(player)                       CardMoved
   -> triggers CardRevealed broadcast      CardRevealed
2. get_available_spell_abilities()         (validation happens here)
3. controller.choose_spell_ability()       ChoiceRequest
```

```
Client GameLoop:
1. draw_card(player)                       // Card added to hand
2. get_available_spell_abilities()         // validate_cards_revealed() PANICS!
   // CardRevealed hasn't been processed yet!
3. choose_spell_ability_with_hook()        // Hook would process reveals HERE
   // But it's too late - validation already failed
```

**Root cause**: `validate_cards_revealed()` runs in `get_available_spell_abilities()` (line 598 of actions.rs), but the pre-choice hook is called later in `choose_spell_ability_with_hook()`.

## User's Proposed Solution

Replace the pre-choice hook with two separate mechanisms:

### 1. "Catch Up to Action X" Hook

- **Purpose**: State synchronization only (process CardRevealed messages)
- **Called**: BEFORE any operation that requires revealed cards (before validation)
- **Signature**: `FnMut(&mut GameState, target_action: u64)`
- **Behavior**: Processes all NetworkMessages until it reaches the target action count
- **Returns**: Nothing (orthogonal to choices)

### 2. IVar/MVar Pattern for Choices

- **Purpose**: Choice synchronization only
- **IVar**: Empty → Populated → Consumed (single-use pattern)
- **Populated by**: Network event loop (stores ChoiceRequest/OpponentChoice as they arrive)
- **Consumed by**: Controllers (NetworkLocalController reads when choose_X is called)

## Sequence with New Architecture

```
Client GameLoop:
1. draw_card(player)
   -> action_count = N
2. CATCH_UP_HOOK(N)                        // Process CardRevealed for the draw
3. get_available_spell_abilities()         // Validation passes - card is revealed!
4. controller.choose_spell_ability()       // Controller reads from IVar
```

```
Network Event Loop (runs independently):
- Reads messages from WebSocket
- CardRevealed: Routes to catch-up processing
- ChoiceRequest: Stores in IVar for controller
- OpponentChoice: Stores in IVar for RemoteController
```

## The Lookahead Question

When at action N, about to make choice (action N+1):
- Catch-up to action N processes all reveals up to that point
- But ChoiceRequest for action N+1 hasn't been processed yet

**Three options for handling this:**

### Option A: Eager Reading by Network Loop
- Network loop reads ahead continuously
- Populates IVar with ChoiceRequest as soon as it arrives
- Catch-up hook only processes CardRevealed, doesn't peek
- **Pro**: Clean separation of concerns
- **Con**: Need to ensure IVar is populated before controller reads

### Option B: Peek-Ahead in Catch-Up Hook
- Catch-up hook optionally peeks one message ahead
- If next message is ChoiceRequest, stores it in IVar
- **Pro**: Single point of message processing
- **Con**: Catch-up hook does two things (state sync + partial choice sync)

### Option C: Controller Blocking Read
- If IVar is empty when controller needs it, do blocking read
- Network loop still routes CardRevealed to catch-up
- **Pro**: Simple, on-demand
- **Con**: Blocking read in controller may complicate things

## Recommended Approach: Option A (Eager Reading)

The network event loop should be the single point of message routing:

```rust
// Network event loop (simplified)
loop {
    let msg = ws_stream.next().await?;
    match msg {
        CardRevealed { .. } => {
            // Store in pending_reveals queue for catch-up hook
            pending_reveals.push(msg);
        }
        ChoiceRequest { .. } | OpponentChoice { .. } => {
            // Store in IVar for controllers
            choice_ivar.set(msg);
        }
        GameEnded { .. } | Error { fatal: true, .. } => {
            // Signal exit
            break;
        }
        _ => {}
    }
}
```

The catch-up hook reads from `pending_reveals`:

```rust
// Catch-up hook
fn catch_up_to_action(game: &mut GameState, target_action: u64) {
    while let Some(reveal) = pending_reveals.try_pop() {
        // Process CardRevealed immediately
        process_card_reveal(game, reveal);
    }
    // Returns when all pending reveals are processed
}
```

Controllers read from IVar:

```rust
// NetworkLocalController
fn choose_spell_ability(...) {
    // Read ChoiceRequest from IVar (may block briefly)
    let choice_request = choice_ivar.take()?;

    // Ask inner controller for decision
    let choice = self.inner.choose_spell_ability(...)?;

    // Send choice to server
    self.send_choice(choice, choice_request.choice_seq)?;
}
```

## Where to Call Catch-Up Hook

The hook should be called at synchronization points:

1. **Before `get_available_spell_abilities()`** - ensures hand cards are revealed
2. **Before any validation that checks revealed cards**
3. **After draws** - server sends CardRevealed right after CardMoved

In priority.rs around line 275:
```rust
// Catch up to current action before validating
self.catch_up_to_action(self.game.action_count());

// Now validation in get_available_spell_abilities() will pass
let available_count = self.get_available_spell_abilities(current_priority).len();
```

## Shared State Design

The network event loop and controllers need shared state:

```rust
struct SharedNetworkState {
    /// Pending reveals for catch-up hook
    pending_reveals: Mutex<VecDeque<CardReveal>>,

    /// IVar for next choice (ChoiceRequest or OpponentChoice)
    /// Empty → Populated → Consumed pattern
    choice_ivar: Mutex<Option<ChoiceInfo>>,

    /// Condvar to signal when choice is available
    choice_ready: Condvar,
}

enum ChoiceInfo {
    Request { action_count: u64, choice_seq: u32 },
    Opponent { indices: Vec<usize>, spell_ability: Option<SpellAbility> },
    Exit,
}
```

## Key Benefits of This Architecture

1. **Decouples state sync from choice sync** - Cleaner separation of concerns
2. **Catch-up called at right time** - Before validation, not after
3. **UI updates possible** - Can call catch-up hook anytime to sync state for display
4. **Single channel preserved** - All messages still come through one WebSocket
5. **Controllers stay simple** - Just read from IVar, don't manage message flow

## Simpler Alternative: Add Catch-Up Before Validation

Instead of the full architectural change, we could:

1. Keep the existing pre-choice hook (handles choice synchronization)
2. Add a NEW catch-up call BEFORE `get_available_spell_abilities()`

```rust
// In priority.rs, before line 275:
self.drain_pending_reveals();  // New call - process any pending CardRevealed

let available_count = self.get_available_spell_abilities(current_priority).len();
```

This would require:
- A second channel or shared queue for CardRevealed messages
- The pre-choice hook continues to handle ChoiceRequest/OpponentChoice

**Pros**:
- Smaller change to existing architecture
- Pre-choice hook still works as-is

**Cons**:
- Two mechanisms for reveal processing (drain + hook)
- Doesn't enable UI updates before choices (user's stated goal)
- Feels like a workaround rather than a clean design

## Recommendation

The user's proposed architecture (catch-up hook + IVar) is the cleaner long-term solution because:

1. **Single responsibility**: Catch-up hook only does state sync, IVar only does choice sync
2. **UI updates**: Can call catch-up anytime to sync state for display without waiting for choices
3. **Future-proof**: As more features need state sync, catch-up hook scales better
4. **Matches mental model**: "Catch up to action X" is intuitive; "pre-choice hook that also processes reveals" is muddled

The complexity cost is the IVar shared state, but that's well-understood concurrency pattern.

## Remaining Questions

1. **Multiple catch-up call sites**: Should catch-up be called at every potential sync point, or just before specific operations?

2. **IVar blocking behavior**: What if controller asks for choice before network loop has populated IVar? Options:
   - Block with timeout (and panic on timeout)
   - Spin-wait with condition variable
   - Return error and let caller retry

3. **Reveal ordering**: Do CardRevealed messages always come before the ChoiceRequest they enable? (I believe yes, based on server implementation)

4. **action_count synchronization**: The catch-up hook needs to know what action count to process up to. Is the client's action_count always accurate?
