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
updated_at: 2026-01-23T01:47:39.764992958+00:00
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

This structural difference is necessary due to browser constraints, but the GAME LOGIC must remain identical.

### 4. Proper State Synchronization

**WASM must maintain synchronized local game state with server:**

- Use the same action-count keyed reveal processing as native
- Process CardRevealed messages to instantiate cards in shadow state
- Maintain server_action_count tracking
- Use drain_reveals_up_to() for sync points

## Current Status

- [ ] WASM network client builds with wasm-network feature
- [ ] WASM connects to server and authenticates
- [ ] WASM receives CardRevealed and processes them correctly
- [ ] WASM maintains synchronized game state
- [ ] random/random games produce identical results (native vs WASM)
- [ ] State hashes match at each action count
- [ ] --network-debug works in WASM
- [ ] Heuristic controller works in WASM (after random is stable)

## Verification Criteria

For each milestone, verify with:

```bash
## Run same game with native client
./target/release/mtg server --port 17771 --password play --seed 42
./target/release/mtg connect deck1.dck --server localhost:17771 --controller random
./target/release/mtg connect deck2.dck --server localhost:17771 --controller random

## Run same game with WASM client
## Compare: game logs, state hashes, final result must be IDENTICAL
```

## Anti-Patterns to Avoid

These are WRONG approaches that have been tried before:

1. **"Direct response" to server** - Bypasses game loop, loses state sync (commit 1715f546, 0fa012e6)
2. **WASM-specific AI logic** - Violates behavioral identity principle
3. **Removing state sync to "fix" sync issues** - Makes problem worse
4. **Server-centric protocol changes** - WASM and native must use same protocol

## Related Files

- `mtg-engine/src/wasm/network/client.rs` - WASM network client
- `mtg-engine/src/wasm/network/local_controller.rs` - Local player controller wrapper
- `mtg-engine/src/wasm/network/remote_controller.rs` - Remote player controller
- `mtg-engine/src/wasm/fancy_tui.rs` - Main WASM TUI (network mode handling)
- `docs/NETWORK_ARCHITECTURE.md` - Network protocol documentation

## References

- Bad commit (archived): wasm-direct-response-bad.v1
- Native client sync: src/network/client.rs drain_reveals_up_to()
