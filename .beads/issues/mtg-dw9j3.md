---
title: Lobby game liveness/heartbeat — drop stale waiting games when host/client disconnects
status: open
priority: 2
issue_type: bug
created_at: 2026-05-31T20:13:58.074378441+00:00
updated_at: 2026-06-01T12:33:30.787994177+00:00
---

# Description

Lobby game liveness/heartbeat — drop stale waiting games when host/client disconnects.

USER (live test 2026-05-31): waiting games stay in the list after the host closes the browser. Add server-side liveness so ListGames only returns live, joinable rooms.

STATUS CORRECTION (2026-06-01): an agent CLOSED this claiming heartbeat/eviction done in lobby-server-protocol (merged @18b2941d), BUT the lobby flow was NEVER play-tested end-to-end and the deployed flow is broken (see mtg-35z3s REDO). The eviction CODE exists (run_create_flow select! loop evicts on creator WS-drop; 30-min WAIT_FOR_JOINER timeout) but is UNVERIFIED against a real two-client journey. REOPENED. Done = proven by the mtg-35z3s end-to-end played-game acceptance test (a closed/exited client's game must disappear from the other client's list). Code refs: mtg-engine/src/network/lobby.rs, server.rs run_create_flow. Part of the mtg-35z3s redo.
