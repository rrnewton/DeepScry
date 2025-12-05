---
title: 'NetworkController: server-side remote player proxy'
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:57:50.191575086+00:00
updated_at: 2025-12-05T17:57:50.191575086+00:00
---

# Description

## NetworkController

Implement a PlayerController that proxies decisions to a remote client over WebSocket.

## Tasks

- [ ] Create `NetworkController` struct with channels for request/response
- [ ] Implement all `PlayerController` trait methods
- [ ] Each method: build options list, compute state hash, send ChoiceRequest, await response
- [ ] Handle timeouts and disconnection gracefully
- [ ] Map choice indices back to actual game objects (SpellAbility, CardId, etc.)
- [ ] Verify options list format matches `format_choice_menu()` for consistency
- [ ] Unit tests with mock channels

## Key Design

Controller is used SERVER-SIDE to represent a remote player. It:
1. Receives choice request from game loop
2. Serializes options to strings
3. Sends ChoiceRequest over channel (to WebSocket handler)
4. Awaits ChoiceResponse
5. Returns the selected option to game loop

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Section 2.3
