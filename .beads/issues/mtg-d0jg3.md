---
title: WASM Network Client - Architecture and Sync Tracking
status: open
priority: 1
issue_type: task
labels:
- wasm
- network
- tracking
created_at: 2026-01-23T01:47:39.764992958+00:00
updated_at: 2026-01-26T16:35:00.000000000+00:00
---

# Description

## WASM Network Client Architecture Tracking

## CRITICAL DESIGN PRINCIPLES

These principles are **non-negotiable** and must be followed for all WASM networking work:

### 1. WASM == Native (Behavioral Identity)

**The WASM web client MUST behave IDENTICALLY to the native network client.**

### 2. No WASM-Specific Controllers

**NEVER create any unique-to-WASM controller logic.**

### 3. Only Blocking/Non-Blocking Differs

**The ONLY acceptable difference is HOW blocking is handled:**

- Native: Blocks thread, waits for server response
- WASM: Uses rewind/replay pattern (yields NeedInput, resumes when input arrives)

This structural difference is necessary due to browser constraints, but the GAME LOGIC must remain identical.

### 4. Proper State Synchronization

**WASM must maintain synchronized local game state with server.**

- Use the same action-count keyed reveal processing as native
- Process CardRevealed messages to instantiate cards in shadow state
- Maintain server_action_count tracking
- Use drain_reveals_up_to() for sync points
- **Use server's authoritative data for choices** (abilities, counts, option lists)

## Current Status (2026-01-26)

### Native Network (Prerequisites for WASM)
- [x] Native network random/random games work (fixed in affdfc22, 1682ac37)
- [x] network_vs_local_equivalence_e2e passes with random controller
- [x] LibrarySearchByName supports random instance selection

### WASM Network
- [x] WASM network client builds with wasm-network feature
- [x] WASM connects to server and authenticates
- [x] WASM captures deck_card_ids from GameStarted (commit 13e6cae1)
- [x] WASM captures rng_state from GameStarted (commit 13e6cae1)
- [x] WASM uses init_game_reserve_only_wasm() with server CardID ranges (commit 13e6cae1)
- [x] Disabled heuristic/zero controllers in WASM until sync fixed (commit 36a39a2e)
- [x] **WASM uses server's authoritative abilities for Priority choices (commit bd0cfe41)**
- [x] **WASM uses server's discard count from ChoiceType::Discard (commit bd0cfe41)**
- [x] **WASM random games progress 23+ choices without DESYNC (commit bd0cfe41)**
- [ ] WASM random/random games run to completion
- [ ] State hashes match at each action count
- [ ] --network-debug works in WASM

---

## Fix: Server-Authoritative Choice Data (2026-01-26)

### Problem
WASM network games were DESYNCing because `WasmNetworkLocalController` was using
local game state values instead of server's authoritative data:

1. **Priority choices**: Local game state computed different available abilities
   than server due to CardRevealed race conditions.
   - DESYNC: Client sent index 4, server only had 2 options

2. **Discard choices**: Local game state calculated different discard count
   (e.g., 8 vs 2) because local hand size diverged from server's.
   - DESYNC: Client sent 8 indices, server expected 2

### Solution
Use server's authoritative data from ChoiceRequest (same pattern as native):

```rust
// 1. Use server's abilities for Priority choices
let effective_available = if let Some(ref abilities) = self.get_server_abilities() {
    abilities.clone()
} else {
    available.to_vec()
};

// 2. Use server's option count to limit hand/list size
let server_option_count = self.get_server_option_count();
let effective_hand = hand[..server_option_count].to_vec();

// 3. Use server's discard count
let effective_count = self.get_server_discard_count().unwrap_or(count);
```

### Result
- Games progress 23+ choices without DESYNC
- No more "invalid choice index" errors
- Server-authoritative pattern matches native NetworkLocalController

---

## Investigation: CardReveal Owner Bug (2026-01-25)

Found a bug in `collect_reveals_since_last_choice()` where reveal owner was set to
`self.player_id` (placeholder) instead of actual card owner.

### Fix Applied
Changed controller.rs to look up actual card owner:
```rust
let card_owner = view.get_card(*card_id)
    .map(|c| c.owner)
    .unwrap_or(self.player_id);
```

### Status
The native network equivalence tests now pass (fixed in integration branch via
affdfc22 and 1682ac37). This owner fix is a belt-and-suspenders improvement.

---

## Anti-Patterns (NEVER DO THESE)

1. **"Direct response" to server** - Bypasses game loop, loses state sync
2. **WASM-specific AI logic** - Violates behavioral identity principle
3. **Removing state sync to "fix" sync issues** - Makes problem worse
4. **Server-centric protocol changes** - WASM and native must use same protocol
5. **Empty library mode for WASM** - WASM must populate libraries same as native
6. **Using local counts/abilities** - Use server's authoritative data from ChoiceRequest

## Key Files

- `mtg-engine/src/network/controller.rs` - NetworkController with collect_reveals_since_last_choice
- `mtg-engine/src/network/reveal_processor.rs` - Shared reveal processing logic
- `mtg-engine/src/wasm/network/client.rs` - WASM network client (ChoiceRequestData with abilities)
- `mtg-engine/src/wasm/network/local_controller.rs` - Local player controller wrapper (server-authoritative choices)
- `mtg-engine/src/wasm/fancy_tui.rs` - Main WASM TUI
- `docs/NETWORK_ARCHITECTURE.md` - Network protocol documentation

## References

- Bad commit (archived): wasm-direct-response-bad.v1
- Native client sync: src/network/client.rs drain_reveals_up_to()
- Network equivalence fixes: affdfc22, 1682ac37
- **Server-authoritative WASM fix: bd0cfe41**
