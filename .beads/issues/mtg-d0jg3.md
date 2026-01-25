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
updated_at: 2026-01-25T18:06:54.083509404+00:00
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

### 4. Proper State Synchronization Required

**WASM must maintain synchronized local game state with server.**

- Use action-count keyed reveal processing (same as native)
- Process CardRevealed messages to instantiate cards in shadow state
- Track server_action_count
- Use drain_reveals_up_to() for sync points

## Current Status (2026-01-25)

- [x] WASM network client builds with wasm-network feature
- [x] WASM connects to server and authenticates
- [x] WASM captures deck_card_ids from GameStarted (commit 13e6cae1)
- [x] WASM captures rng_state from GameStarted (commit 13e6cae1)
- [x] WASM uses init_game_reserve_only_wasm() with server CardID ranges (commit 13e6cae1)
- [x] Disabled heuristic/zero controllers in WASM until sync fixed (commit 36a39a2e)
- [ ] Native network random/random games work without desync (BLOCKED - see bug below)
- [ ] WASM random/random games produce identical results as native

---

## NATIVE DESYNC BUG (2026-01-25)

**IMPORTANT**: Before fixing WASM-specific issues, we need to fix NATIVE network desync first!

Discovered a DESYNC bug that affects NATIVE random-vs-random network games (not WASM-specific!).

### Symptoms
- Client has 3 extra cards (46, 47, 52) in Gabriel's hand that server doesn't have
- First mismatch at Turn 20, choice_seq=222, action_count=1008
- Both have same action_count but different state hashes

### Investigation Findings

1. **Wrong `owner` in `collect_reveals_since_last_choice`** (controller.rs:439)
   - Was using `self.player_id` (the player making choice) as placeholder
   - Should use actual card owner from `view.get_card(card_id).owner`
   - FIX APPLIED but issue persists - may be partial fix

2. **Reveal Processing Logic** (reveal_processor.rs)
   - For Draw/OpeningHand reveals, checks if card is in `owner`'s library
   - If wrong owner passed, checks wrong player's library
   - Could cause cards to be added to hand directly when they shouldn't

3. **GameLoop draws opening hands even with `skip_opening_hands=true`** (game_loop/mod.rs:1073-1079)
   - Named misleadingly - only skips shuffle, still draws 7 cards per player
   - This is probably intentional but confusing

### Root Cause (Suspected)

The `collect_reveals_since_last_choice` function was setting `owner: self.player_id` for all reveals, regardless of which player actually owns the card. This caused:
1. Reveal for Gabriel's card collected by Ryan's controller
2. Reveal sent to client with owner=Ryan
3. Client's reveal_processor checks Ryan's zones, card not found
4. Card added to wrong player's hand directly

However, the fix didn't fully resolve the issue, suggesting there may be MULTIPLE sources of incorrect reveal ownership or another related bug.

### Next Steps
1. Verify the fix is actually being used (rebuild and test)
2. Check if there are other code paths that send CardRevealed with wrong owner
3. Add debug logging to trace exact reveal flow
4. Consider stricter validation that reveals match expected CardID ranges

---

## Anti-Patterns (NEVER DO THESE)

1. **"Direct response" to server** - Bypasses game loop, loses state sync (commit 1715f546, wasm-direct-response-bad.v1)
2. **WASM-specific AI logic** - Violates behavioral identity principle
3. **Removing state sync to "fix" sync issues** - Makes problem worse
4. **Server-centric protocol changes** - WASM and native must use same protocol

## Key Files

- `mtg-engine/src/network/controller.rs` - NetworkController with collect_reveals_since_last_choice
- `mtg-engine/src/network/reveal_processor.rs` - Shared reveal processing logic
- `mtg-engine/src/wasm/network/client.rs` - WASM network client
- `mtg-engine/src/wasm/network/local_controller.rs` - Local player controller wrapper
- `mtg-engine/src/wasm/fancy_tui.rs` - Main WASM TUI (updated with init_game_reserve_only_wasm)
- `docs/NETWORK_ARCHITECTURE.md` - Network protocol documentation

## References

- Bad commit (archived): wasm-direct-response-bad.v1
- Native client sync: `src/network/client.rs` drain_reveals_up_to()
- Late-binding architecture: mtg-qtqcr
