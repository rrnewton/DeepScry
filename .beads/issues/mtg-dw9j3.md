---
title: Lobby game liveness/heartbeat — drop stale waiting games when host/client disconnects
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-31T20:13:58.074378441+00:00
updated_at: 2026-06-01T01:03:37.646035115+00:00
---

# Description

## Status: COMPLETED (2026-05-31)

Implemented in lobby-server-protocol branch.

### What was done:
- Waiting games now tied to the creator's live WS connection via the `run_create_flow` select! loop
- Creator disconnect detected via `read_one_lobby_message` returning Err in the waiting loop
- On disconnect: immediately removes the game from `waiting_games` with a log message
- WAIT_FOR_JOINER timeout also evicts stale games (30 min)
- ListGames now only returns games with live connections (no stale entries)
- WaitingPlayerState tracked server-side per player (deck + ready flag)
- Watch channel (tokio::sync::watch) used to propagate state changes to creator

### New protocol:
- SetDeck / SetReady ClientMessages handled in waiting room loop
- WaitingRoomUpdate ServerMessage sent to both players on state change
- WaitingRoomSnapshot as internal data structure
