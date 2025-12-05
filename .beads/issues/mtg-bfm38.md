---
title: Networking E2E tests
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:58:29.730244324+00:00
updated_at: 2025-12-05T17:58:29.730244324+00:00
---

# Description

## Networking E2E Tests

End-to-end tests for networked gameplay.

## Tasks

- [x] Test: Protocol serialization round-trips for all message types
- [x] Test: State hash computation excludes hidden info (test_network_mode_strips_hidden_info)
- [x] Test: LibraryMode::Remote draw behavior (zones.rs tests)
- [ ] Test: Server accepts two clients, starts game
- [ ] Test: Full game with fixed-script controllers over network
- [ ] Test: Hash verification detects intentional desync
- [ ] Test: Deck visibility flag sends/hides deck lists
- [ ] Test: Graceful handling of client disconnect
- [ ] Test: Concurrent games on same server (if supported)
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

## Remaining Work

Integration tests requiring actual WebSocket connections:
1. Spawn server, connect two clients, verify game starts
2. Run complete game with FixedScriptController over network
3. Detect desync by corrupting client state

## Test Strategy

Use localhost connections with fixed-script or heuristic controllers.
Compare network game results with equivalent local games to verify determinism.

## Validation

```bash
# Local game (baseline)
mtg tui deck1.dck deck2.dck --p1=heuristic --p2=heuristic --seed=12345

# Network game (should produce identical result)
mtg server --port=17771 --password=test &
mtg connect deck1.dck --server=localhost:17771 --password=test --controller=heuristic &
mtg connect deck2.dck --server=localhost:17771 --password=test --controller=heuristic
```

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Part 5
