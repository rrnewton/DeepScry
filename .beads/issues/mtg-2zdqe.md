---
title: 'NetworkController: server-side remote player proxy'
status: closed
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:57:50.191575086+00:00
updated_at: 2025-12-05T18:28:07.395516198+00:00
---

# Description

## NetworkController

Implement a PlayerController that proxies decisions to a remote client over WebSocket.

## Tasks

- [x] Create `NetworkController` struct with channels for request/response
- [x] Implement all `PlayerController` trait methods
- [x] Each method: build options list, compute state hash, send ChoiceRequest, await response
- [x] Handle disconnection gracefully (returns ExitGame)
- [x] Map choice indices back to actual game objects (SpellAbility, CardId, etc.)
- [x] Unit tests with mock channels (4 tests)
- [~] Handle timeouts (TODO: need async/tokio integration)
- [N/A] Verify options list format matches `format_choice_menu()` - simplified for now

## Key Design

Controller is used SERVER-SIDE to represent a remote player. It:
1. Receives choice request from game loop
2. Serializes options to strings
3. Sends ChoiceRequest over channel (to WebSocket handler)
4. Awaits ChoiceResponse
5. Returns the selected option to game loop

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Section 2.3
