---
title: WebSocket server implementation
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:58:04.317148983+00:00
updated_at: 2025-12-05T17:58:04.317148983+00:00
---

# Description

## WebSocket Server

Implement the game server that accepts client connections and runs authoritative games.

## Tasks

- [ ] Create `GameServer` struct with config, waiting player, active games
- [ ] Implement WebSocket listener with tokio-tungstenite
- [ ] Handle authentication (password check, deck submission)
- [ ] Implement waiting room (first player waits for second)
- [ ] Create game when two players connected
- [ ] Load decks deterministically (sorted card names)
- [ ] Shuffle libraries with server RNG
- [ ] Draw opening hands, send GameStarted to both clients
- [ ] Run game loop with NetworkControllers for both players
- [ ] Broadcast CardRevealed events (draws, plays, etc.)
- [ ] Broadcast OpponentChoice notifications
- [ ] Send GameEnded when game completes
- [ ] Add `mtg server` CLI subcommand

## CLI

\`\`\`bash
mtg server --port=17771 --password=SECRET [--deck-visibility] [--life=20]
\`\`\`

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Part 3
