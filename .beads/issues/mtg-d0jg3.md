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
updated_at: 2026-02-18T17:03:12.965469450+00:00
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

## Current Status (2026-02-18)

### Completed this session (2026-02-18):
- Removed server-authoritative fallbacks from WasmNetworkLocalController (choose_spell_ability_to_play, choose_cards_to_discard). WASM now computes locally like native.
- Added debug-assertion validation that logs DESYNC when local != server (but doesn't crash)
- Fixed WASM module exports: added `pub use network::*` in wasm/mod.rs so wasm-bindgen sees network functions
- Extracted init_game_reserve_only_wasm + process_card_reveal_wasm from fancy_tui.rs → wasm/network/game_init.rs (shared between FancyTUI and headless AI harness)
- Created wasm/network/ai_harness.rs with run_network_ai_step + network_ai_reset WASM exports for headless testing
- Extended bug_finding/network_test_lib.py: parse_deck_file, start_wasm_http_server, run_wasm_client (Playwright), ClientMode support in TestConfig, updated run_network_game
- Extended bug_finding/network_fuzz_test.py with --client flag (native/wasm/mixed)
- Extended tests/network_vs_local_equivalence.py with client mode args
- Fixed wasm_ai_harness.html: correct load_deck_pack API, proper deck_index.json parsing

### Remaining Issues (known divergence):
- WASM game state diverges from server at seq~9 when running random controller
- Specific case: WASM computes PlayLand(card_id=36) locally but server sends 0 abilities in ChoiceRequest
- Root cause: sync_callback timing - reveals may not be processed at exact right moment
- Symptom: After ~20 choices, game diverges; server sees connection drop
- Infrastructure correctly detects this as FAIL in equivalence tests
- Next: investigate sync_callback vs prepare_for_priority_choice() equivalence

### Prior Status (2026-02-13)

### Native Network (Working)
- [x] Native network random/random games work
- [x] network_vs_local_equivalence_e2e passes (100% determinism)
- [x] `prepare_for_priority_choice()` fix solves race condition
- [x] LibrarySearchByName supports random instance selection

### WASM Network (Partially Working)
- [x] WASM network client builds with wasm-network feature
- [x] WASM connects to server and authenticates
- [x] WASM captures deck_card_ids from GameStarted
- [x] WASM captures rng_state from GameStarted
- [x] WASM uses init_game_reserve_only_wasm() with server CardID ranges
- [x] **WASM uses server's authoritative abilities for Priority choices**
- [x] **WASM uses server's discard count from ChoiceType::Discard**
- [x] WASM random games progress 23+ choices without DESYNC
- [x] test_network_e2e.js passes (connection and game UI)
- [ ] WASM random/random games run to completion
- [ ] State hashes match at each action count
- [ ] --network-debug works in WASM
- [ ] **Local-equivalence verified** (WASM network == local same seed)

## Architecture Gap Analysis (2026-02-13)

### Native vs WASM Sync Approaches

| Aspect | Native | WASM |
|--------|--------|------|
| Race Protection | `prepare_for_priority_choice()` blocks on MVar | Uses server-authoritative data |
| Ability Computation | Local (then verified against server) | Server-provided via ChoiceRequest |
| Discard Count | Local (verified) | Server-provided from ChoiceType |
| Equivalence | Verified: local == network gamelogs | NOT verified |

### The Problem

Native client now has PERFECT determinism:
1. `prepare_for_priority_choice()` blocks until ChoiceRequest arrives
2. All CardRevealed messages buffered at that point
3. `sync_to_action()` processes reveals
4. Abilities computed locally = match server exactly

WASM takes a shortcut:
1. Uses `get_server_abilities()` to bypass local computation
2. Uses `get_server_discard_count()` to bypass local count
3. This WORKS but doesn't VERIFY behavioral identity

### Next Steps for True Parity

1. **Verify sync_callback timing** - ensure reveals processed before controller
2. **Remove server-authoritative fallbacks** - compute locally, verify against server
3. **Add WASM local-equivalence test** - same seed locally vs WASM network
4. **Enable state hash verification** - catch any remaining divergence

---

## Key Files

- `mtg-engine/src/network/controller.rs` - NetworkController
- `mtg-engine/src/network/reveal_processor.rs` - Shared reveal processing logic
- `mtg-engine/src/wasm/network/client.rs` - WASM network client
- `mtg-engine/src/wasm/network/local_controller.rs` - Local player controller wrapper
- `mtg-engine/src/wasm/fancy_tui.rs` - Main WASM TUI with sync_callback
- `docs/NETWORK_ARCHITECTURE.md` - Network protocol documentation

## References

- Native race condition fix: e30c0433d `prepare_for_priority_choice()`
- Server-authoritative WASM fix: bd0cfe41
