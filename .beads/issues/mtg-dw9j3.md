---
title: Lobby game liveness/heartbeat — drop stale waiting games when host/client disconnects
status: open
priority: 2
issue_type: bug
created_at: 2026-05-31T20:13:58.074378441+00:00
updated_at: 2026-05-31T20:24:52.456658870+00:00
---

# Description

USER (live testing): waiting games stay in the join list after the host closes the browser — no liveness; list shows stale unjoinable rooms. Add liveness so ListGames returns only live rooms: tie a waiting room's presence to its host WS connection (evict on disconnect) and/or a periodic heartbeat ping with eviction on miss.

Server: mtg-engine/src/network/lobby.rs (LobbyState@132; waiting_games HashMap@134; list_waiting_paged@183; len@236) + server.rs (per-connection lifecycle) + protocol.rs (ListGames). Implement the waiting-room lifecycle defined by the mtg-khy7x storyboard.
