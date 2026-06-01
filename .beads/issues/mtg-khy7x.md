---
title: Lobby UX redesign — write a storyboard + reconcile client/server state (flow is confusing)
status: open
priority: 1
issue_type: task
created_at: 2026-05-31T20:13:58.068340424+00:00
updated_at: 2026-05-31T20:24:52.453270871+00:00
---

# Description

USER: lobby flow is confusing/duplicated. Observed: deck-picker on the lobby page AND again on the create screen (unclear if the 2nd does anything); after Create you reach a screen that isn't joinable until a SECOND create-game click; browser-vs-server state not reconciled.

DELIVERABLE (design FIRST, before code): a screen-by-screen storyboard in ai_docs/ for the full flow — landing -> name -> create/join -> waiting room -> launch -> in-game — covering create+join x TUI+Native, explicitly mapping browser-side vs server-side state. Target: ONE deck-picker, ONE create action, immediately joinable. Get user sign-off, THEN implement.

Code sites: web/index.html (1095 lines): panes pane-name@284 / pane-lobby@303 / pane-waiting@409; create-deck <select>@330; btn-create@346; loadDeckNames@573; showWaitingRoom@863; redirectToGamePage@920; createGame@951; joinGame@974. web/lobby_launcher.js: buildRedirectUrl@68 / consumeLobbyParams@110 / applyLobbyParamsToForm@153 / buildLobbyAction@220. Server: mtg-engine/src/network/lobby.rs (LobbyState@132, waiting_games@134, list_waiting_paged@183) + protocol.rs (ListGames/CreateGame/JoinGame msg types).

BLOCKS mtg-1vwpd + mtg-dw9j3 (implement against this storyboard). Builds on mtg-465 (Phase 1), mtg-187 (Phase 2).
