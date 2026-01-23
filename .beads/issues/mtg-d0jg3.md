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
updated_at: 2026-01-23T18:13:32.072082213+00:00
---

# Description

## WASM Network Client Architecture Tracking

## CRITICAL DESIGN PRINCIPLES

These principles are **non-negotiable** and must be followed for all WASM networking work:

### 1. WASM == Native (Behavioral Identity)

**The WASM web client MUST behave IDENTICALLY to the native network client.**

- Same RNG sequences for the same seeds
- Same game state at every point
- Same controller decisions given same inputs
- Same state hashes at every action count
- If you run native/native and wasm/native with the same seed, they produce the same game log

### 2. No WASM-Specific Controllers

**NEVER create any unique-to-WASM controller logic.**

- No "direct response" patterns that bypass game loop
- No WASM-specific AI decision making
- Controllers (random, heuristic, zero) must use the SAME code paths as native
- WASM wraps common controller code, it doesn't replace it

### 3. Only Blocking/Non-Blocking Differs

**The ONLY acceptable difference is HOW blocking is handled:**

- Native: Blocks thread, waits for server response
- WASM: Uses rewind/replay pattern (yields NeedInput, resumes when input arrives)

### 4. Proper State Synchronization Required

**WASM must maintain synchronized local game state with server:**

- Use action-count keyed reveal processing (same as native)
- Process CardRevealed messages to instantiate cards in shadow state
- Track server_action_count
- Use drain_reveals_up_to() for sync points

## Current Status (2026-01-23)

- [x] WASM network client builds with wasm-network feature
- [x] WASM connects to server and authenticates
- [x] WASM captures deck_card_ids from GameStarted (commit 13e6cae1)
- [x] WASM captures rng_state from GameStarted (commit 13e6cae1)
- [x] WASM uses init_game_reserve_only_wasm() with server CardID ranges (commit 13e6cae1)
- [x] Disabled heuristic/zero controllers in WASM until sync fixed (commit 36a39a2e)
- [x] WASM uses empty library mode - reveals add cards directly to hand (prevents shuffle divergence)
- [x] WASM queues opening_hand from GameStarted immediately (prevents timing issues)
- [x] WasmNetworkLocalController clamps choice indices to server option count (prevents desync)
- [x] network_random_e2e test passes without DESYNC errors
- [ ] random/random games produce identical results (native vs WASM) (partial - no desync, but full parity testing needed)
- [ ] State hashes match at each action count (needs verification)
- [ ] --network-debug works in WASM

## Implementation Progress

### Phase 1: Late-Binding Architecture (COMPLETED)

**commit 13e6cae1** - Added late-binding CardID architecture to WASM:
- WasmNetworkClient now stores deck_card_ids, rng_state, token_definitions
- New init_game_reserve_only_wasm() creates games with reserved CardID slots
- launch_network_game() uses DeckCardIdRanges from server

### Phase 2: Controller Restrictions (COMPLETED)

**commit 36a39a2e** - Restricted to safe controllers:
- Only human and random controllers allowed in WASM network mode
- Heuristic and zero disabled until state sync is verified
- Added controller_seed parameter for deterministic random

### Phase 3: State Synchronization Fixes (COMPLETED)

**Session 2026-01-23** - Fixed critical state sync issues:

1. **Empty library mode**: `init_game_reserve_only_wasm()` no longer adds CardIDs to libraries.
   - Server shuffles libraries in specific order using its RNG
   - WASM doesn't have access to that shuffle order
   - If we added CardIDs to library, draws would be in wrong order
   - Fix: Reveals add cards directly to hand (reveal_processor's "empty library mode")

2. **Opening hand queueing**: GameStarted handler now queues `opening_hand` cards immediately.
   - Server sends CardRevealed messages AFTER GameStarted
   - WASM game loop may start before those messages arrive
   - Without opening hand, sync_to_action() draws from empty library → empty hand
   - Fix: Queue opening_hand cards from GameStarted as pending reveals

3. **Choice index clamping**: WasmNetworkLocalController clamps indices to server option count.
   - Local game state can diverge from server
   - RandomController picks index from LOCAL available options
   - If local has more options than server, desync occurs
   - Fix: Clamp choice index to server's option count, log warning if clamped

### Phase 4: Full Parity Testing (TODO)

- Compare state hashes between WASM and native at each choice point
- Run extended games with --network-debug enabled
- Verify no clamping warnings occur (indicates state divergence)
- Test with various decks and scenarios

## Anti-Patterns to Avoid

These are WRONG approaches that have been tried before:

1. **"Direct response" to server** - Bypasses game loop, loses state sync (commit 1715f546, wasm-direct-response-bad.v1)
2. **WASM-specific AI logic** - Violates behavioral identity principle
3. **Removing state sync to "fix" sync issues** - Makes problem worse
4. **Server-centric protocol changes** - WASM and native must use same protocol

## Related Files

- `mtg-engine/src/wasm/network/client.rs` - WASM network client (updated)
- `mtg-engine/src/wasm/network/local_controller.rs` - Local player controller wrapper
- `mtg-engine/src/wasm/network/remote_controller.rs` - Remote player controller
- `mtg-engine/src/wasm/fancy_tui.rs` - Main WASM TUI (updated with init_game_reserve_only_wasm)
- `docs/NETWORK_ARCHITECTURE.md` - Network protocol documentation

## References

- Bad commit (archived): wasm-direct-response-bad.v1
- Native client sync: `src/network/client.rs` drain_reveals_up_to()
- Late-binding architecture: mtg-qtqcr
