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
updated_at: 2026-02-19T07:37:11.081789879+00:00
---

# Description

### CRITICAL DESIGN PRINCIPLES

These principles are non-negotiable and must be followed for all WASM networking work:

**1. WASM == Native (Behavioral Identity)**
The WASM web client MUST behave IDENTICALLY to the native network client.

**2. No WASM-Specific Controllers**
NEVER create any unique-to-WASM controller logic.

**3. Only Blocking/Non-Blocking Differs**
The ONLY acceptable difference is HOW blocking is handled:
- Native: Blocks thread, waits for server response
- WASM: Uses rewind/replay pattern (yields NeedInput, resumes when input arrives)

**4. Proper State Synchronization**
WASM must maintain synchronized local game state with server, using the same action-count keyed reveal processing as native.

---

### Current Status (2026-02-18_#1825)

**COMPLETE: WASM network equivalence 10/10 seeds pass**

All known desync patterns have been fixed. Seeds 1-10 pass 100% with identical local vs network gamelogs.

**Fixes applied this session (commit 8cb994050):**

1. **pending_activation guard** (fixed seeds 5, 6): When WASM game loop was interrupted (NeedInput) during target selection of an activated ability (e.g. Barrels of Blasting Jelly), re-entry would misroute the pending targets ChoiceRequest as a spell ability choice → invalid index sent to server → DESYNC. Fixed by adding `pending_activation: Option<(PlayerId, CardId, usize)>` to GameState (#[serde(skip)]) with labeled block `'ability_choice:` for early exit. Pattern mirrors `pending_cast`.

2. **Token definitions preloading** (fixed seeds 2, 4, 7): `init_game_reserve_only_wasm()` created bare GameState with empty `token_definitions`. When server created Clue Tokens (via Cunning Maneuver), WASM couldn't process them: "Token definition not found: 'c_a_clue_draw'" → infinite retry loop → timeout. Fixed by cloning token_definitions from WasmNetworkClient (received in GameStarted) and inserting into game.token_definitions in `init_harness()`.

3. **Removed [CAST-DEBUG] temporary logging** from priority.rs and controller.rs.

4. **Removed unused SmallVec import** from game_loop/mod.rs.

**Prior fixes (earlier commits in this session):**
- `pending_cast` guard for spell casting resumption (seeds 1, 3)
- `pending_cycling_search` guard for library cycling search
- `blockers_declared_turn` guard for DeclareBlockers re-execution
- Combat damage step guards (3 `#[serde(skip)]` fields)
- `spell_targets` persistence moved from GameLoop to GameState
- `choose_from_library` 0-based → 1-based index encoding fix

---

### Key Files

- `mtg-engine/src/network/controller.rs` - NetworkController
- `mtg-engine/src/network/reveal_processor.rs` - Shared reveal processing logic
- `mtg-engine/src/wasm/network/client.rs` - WASM network client
- `mtg-engine/src/wasm/network/local_controller.rs` - WasmNetworkLocalController
- `mtg-engine/src/wasm/network/ai_harness.rs` - Headless AI harness for equivalence tests
- `mtg-engine/src/wasm/network/game_init.rs` - WASM game initialization
- `docs/NETWORK_ARCHITECTURE.md` - Network protocol documentation

### References

- Native race condition fix: e30c0433d (prepare_for_priority_choice())
- WASM desync fixes: 8cb994050 (pending_activation + token defs)
- Equivalence test: ./tests/network_vs_local_equivalence_e2e.sh
