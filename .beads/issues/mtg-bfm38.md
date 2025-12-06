---
title: Networking E2E tests
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:58:29.730244324+00:00
updated_at: 2025-12-05T23:16:26.742820585+00:00
---

# Description

## Networking E2E Tests

End-to-end tests for networked gameplay.

## Tasks

- [x] Test: Protocol serialization round-trips for all message types
- [x] Test: State hash computation excludes hidden info (test_network_mode_strips_hidden_info)
- [x] Test: LibraryMode::Remote draw behavior (zones.rs tests)
- [x] Test: Server accepts two clients, starts game (test_two_clients_game_start)
- [x] Test: Server authentication flow (test_server_auth_flow)
- [x] Test: Wrong password rejected (test_wrong_password_rejected)
- [x] Test: Full game with automated controllers over network (test_full_game_always_pass)
- [ ] Test: Hash verification detects intentional desync
- [x] Test: Deck visibility flag sends/hides deck lists (test_deck_visibility_enabled/disabled)
- [x] Test: Graceful handling of client disconnect (test_client_disconnect_handling)
- [x] Test: Concurrent games on same server - NOT SUPPORTED (server handles connections sequentially)
- [ ] Integration with existing agentplay scripts

## Completed Tests

### Protocol Serialization (protocol.rs)
- `test_client_message_serialization` - Basic ClientMessage round-trip
- `test_server_message_serialization` - Basic ServerMessage round-trip
- `test_card_reveal_serialization` - CardReveal round-trip
- `test_choice_type_serialization` - ChoiceType Targets variant
- `test_reveal_reason_serialization` - All RevealReason variants
- `test_all_server_message_variants` - Comprehensive: all ServerMessage variants
- `test_all_client_message_variants` - Comprehensive: all ClientMessage variants
- `test_choice_type_all_variants` - All ChoiceType variants

### State Hash (state_hash.rs)
- `test_network_mode_strips_hidden_info` - Network mode strips RNG, hand/library contents

### Remote Library (zones.rs)
- `test_remote_library_creation` - Remote library setup
- `test_remote_library_draw_with_reveals` - Queue/draw FIFO
- `test_remote_library_peek` - Peek behavior
- `test_remote_library_add_to_top_and_bottom` - Size tracking
- `test_remote_library_clear` - Clear behavior
- `test_local_library_queue_reveal_is_noop` - Mode detection

### Client (client.rs)
- `test_client_config` - Config creation
- `test_deck_to_submission` - Deck conversion

### Server (server.rs)
- `test_server_config_default` - Default config values
- `test_deck_submission_sizes` - Size calculations
- `test_submission_to_decklist` - Conversion

### Controller (controller.rs)
- `test_network_controller_creation` - Controller setup
- `test_choice_request_response` - Choice flow
- `test_sequence_mismatch_error` - Sequence validation
- `test_invalid_choice_error` - Choice bounds
- `test_network_error_display` - Error formatting

### WebSocket Integration (network_e2e.rs)
- `test_server_auth_flow` - Server accepts connections, authenticates clients
- `test_two_clients_game_start` - Two clients connect and receive GameStarted
- `test_wrong_password_rejected` - Invalid password rejected
- `test_protocol_encoding_decoding` - GameStarted message round-trip
- `test_deck_submission_encoding` - ClientMessage Authenticate round-trip
- `test_choice_flow_encoding` - ChoiceRequest/SubmitChoice flow
- `test_card_reveal_flow` - CardRevealed message round-trip
- `test_full_game_always_pass` - Complete game played over network (107 turns to deck-out)
- `test_client_disconnect_handling` - Server handles client disconnect gracefully
- `test_deck_visibility_enabled` - Deck list shared when enabled
- `test_deck_visibility_disabled` - Deck list hidden when disabled

## Remaining Work

Integration tests requiring actual WebSocket connections:
1. ~~Spawn server, connect two clients, verify game starts~~ (DONE: test_two_clients_game_start)
2. ~~Run complete game with automated controllers~~ (DONE: test_full_game_always_pass)
3. Detect desync by corrupting client state

## Client-Side UI Progress (2025-12-05)

### Completed
- **RemoteController**: Created controller for receiving opponent choices from server
  - Implements full PlayerController trait
  - Receives choices via mpsc channel
  - Returns ChoiceResult::ExitGame on disconnect
- **Shared display function**: Extracted `print_battlefield_state` to game::display module
  - Used by both GameLoop and NetworkClient
  - Eliminates code duplication
  - Shows viewer's hand (not just active player)
- **Connect CLI enhanced**: Added --controller, --fixed-inputs, --seed-player, --visual-stacks, --verbosity
- **--real-gameloop flag**: Added CLI flag to use run_game_real (commits c0c6500e, d5f84f80)
- **SharedRevealQueue**: Infrastructure for passing card reveals from WebSocket to game thread (commit ba0fba13)
- **Makefile/test_helpers**: Added --features network to validation (commit c0c6500e)

### In Progress
- **run_game_real timing**: Reveals only drained at startup, not during gameplay
  - Need to either hook GameLoop or have controllers drain queue
  - Core infrastructure is in place

### FIXME-UNFINISHED Items
See markers in code for stubbed functionality:
- Client doesn't replay opponent choices on shadow state
- Hash verification accepts server hash without computing local
- Multi-select not supported for targets, mana, attackers, blockers, discard
- GameEndReason not from actual GameLoop

## Fixed Issues

- **Server now sends GameEnded**: Added oneshot channels to signal game end to WebSocket handlers.
  Handlers now properly send GameEnded message with winner, reason, and final state hash before closing.

## Known Issues

### Flaky test_run_game_with_random_controllers (2025-12-06)

The `test_run_game_with_random_controllers` test is flaky (~40% pass rate) due to timing issues.

**Implemented fix (2025-12-06_#1251)**: Added `RemoteMessage::GameEnded` signal
- `RemoteMessage` enum replaces struct with `Choice` and `GameEnded` variants
- WebSocket handler sends `RemoteMessage::GameEnded` through `remote_choice_tx` before exiting
- `RemoteController` handles `GameEnded` gracefully (no disconnect warning)
- This fixes the game-end race condition when server announces winner

**Remaining issue**: The test still fails sometimes during gameplay (before game ends).
The root cause appears to be timing between concurrent client GameLoops and the server's
message routing. When one client's GameLoop aborts its WebSocket handler (e.g., when
the game_result future wins the select! race), it can drop the channel before the
other client receives all expected `OpponentChoice` messages.

The test remains marked with `#[ignore]` while we investigate further.

## Test Strategy

Use localhost connections with fixed-script or heuristic controllers.
Compare network game results with equivalent local games to verify determinism.
