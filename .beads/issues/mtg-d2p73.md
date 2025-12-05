---
title: Network protocol types and message serialization
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:57:12.749279939+00:00
updated_at: 2025-12-05T17:57:12.749279939+00:00
---

# Description

## Network Protocol Types

Define the message types for client/server communication.

## Tasks

- [ ] Create `src/network/mod.rs` module structure
- [ ] Define `ClientMessage` enum (Authenticate, SubmitChoice, Disconnect, Ping)
- [ ] Define `ServerMessage` enum (AuthResult, GameStarted, CardRevealed, ChoiceRequest, OpponentChoice, GameEnded, Error, Pong, DebugStateDump)
- [ ] Define supporting types (CardReveal, RevealReason, ChoiceType, ChoiceContext, DeckListInfo)
- [ ] Add serde Serialize/Deserialize derives
- [ ] Unit tests for serialization round-trips

## Dependencies to Add

\`\`\`toml
tokio-tungstenite = { version = "0.26", optional = true }
futures-util = { version = "0.3", optional = true }
\`\`\`

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Part 1.
