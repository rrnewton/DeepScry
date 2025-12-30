---
title: 4-way gamelog equivalence test for NETWORK_MODE
status: open
priority: 2
issue_type: task
created_at: 2025-12-08T11:49:48.576522867+00:00
updated_at: 2025-12-30T16:59:04.166495803+00:00
---

# Description

## Update (2025-12-30) - spell_ability and CardRevealed Integration Complete

**All Remaining Work from 2025-12-29 is now DONE:**

1. ✅ Server populates `spell_ability` in OpponentChoice
   - Added ChosenAbilityInfo channel from NetworkController to WebSocket handler
   - NetworkController sends ability info after each Priority choice
   - WebSocket handler waits for ability before broadcasting OpponentChoice

2. ✅ Client handles executing abilities for cards not in game state
   - drain_reveals now handles RevealReason::Played
   - Cards played by opponent are instantiated in game.cards
   - card_db is cloned and passed to spawn_blocking for instantiation

3. ✅ Cards played from hand are revealed before execution
   - Server sends CardRevealed before OpponentChoice when opponent plays
   - Client receives and processes the reveal to instantiate the card
   - Then receives OpponentChoice with spell_ability to execute

**Implementation Summary:**

The flow now works as follows:
1. Server's NetworkController makes a Priority choice
2. After receiving client response, sends ChosenAbilityInfo on ability_tx
3. WebSocket handler receives ability info via ability_rx
4. If a card was played, server sends CardRevealed to opponent
5. Then sends OpponentChoice with spell_ability populated
6. Client receives CardRevealed, instantiates card in shadow state
7. Client receives OpponentChoice with spell_ability
8. RemoteController uses the spell_ability to execute correctly

**Files Changed (commit bb588ba):**
- mtg-engine/src/network/controller.rs - ChosenAbilityInfo, ability_tx channel
- mtg-engine/src/network/server.rs - CardRevealed sending, ability channel wiring
- mtg-engine/src/network/client.rs - CardReveal handling, card instantiation

**Testing Status:**
- Unit tests: 414 passed (after fixes)
- Network E2E: WORKING - Full 69-turn game with 2393 actions completed successfully!

## Update (2025-12-30) - Network Sync Bug Fixed

**Bug**: Network games were hanging during priority checks when `available_count=0`.

**Root Cause**: Three separate issues:

1. **Server NetworkController returned `ControllerType::Zero`**
   - This caused the server to auto-pass without sending ChoiceRequest
   - Clients never received notification of the pass

2. **Client NetworkLocalController delegated to inner controller type**
   - When the inner controller (Fixed) had type Zero, the client auto-passed
   - NetworkLocalController never sent SubmitChoice back to server

3. **RichInputController failed on empty available list**
   - When asked to choose with 0 options, it tried to match command "1"
   - This caused an error since no options existed

**Fixes Applied:**

1. Added `ControllerType::Network` variant to `game/snapshot.rs`
2. NetworkController now returns `ControllerType::Network`
3. NetworkLocalController now returns `ControllerType::Network`
4. Priority loop checks for both `Remote` and `Network` controller types
5. RichInputController returns `Ok(None)` when available list is empty

**Files Changed:**
- mtg-engine/src/game/snapshot.rs - Added Network variant
- mtg-engine/src/network/controller.rs - Returns Network type
- mtg-engine/src/network/local_controller.rs - Returns Network type
- mtg-engine/src/game/game_loop/priority.rs - Check both Remote and Network
- mtg-engine/src/game/rich_input_controller.rs - Handle empty available list

**Next Steps:**
- Re-enable gamelog_equivalence_e2e.sh to verify 4-way sync
- Close this issue after full validation

---

## Update (2025-12-29) - Auto-pass Bug Fix

**Root Cause 2 Identified: Client auto-passes for opponent**

The client's priority loop auto-passes when no available abilities are computed:

```rust
if available_count == 0 {
    break None;  // Auto-pass
}
```

But for opponent's hand, the client doesn't know the contents, so `get_available_spell_abilities()` returns empty. This means the client **never asks the RemoteController** for opponent's choices and auto-passes instead!

This causes massive divergence: opponent plays spells on server, but client skips them entirely.

**Fix Applied (Partial)**:

1. Added `ControllerType::Remote` variant
2. Priority loop now checks: `if available_count == 0 && !is_remote { auto-pass }`
3. RemoteController now always gets asked, even when `available` is empty
4. Added `spell_ability: Option<SpellAbility>` to OpponentChoice protocol
5. RemoteController can use `spell_ability` when provided by server

Validation passes with partial fix.
