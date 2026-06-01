---
title: Lobby UX redesign — write a storyboard + reconcile client/server state (flow is confusing)
status: open
priority: 1
issue_type: task
created_at: 2026-05-31T20:13:58.068340424+00:00
updated_at: 2026-06-01T03:49:50.931167695+00:00
---

# Description

USER: lobby flow is confusing/duplicated. Observed: deck-picker on the lobby page AND again on the create screen (unclear if the 2nd does anything); after Create you reach a screen that isn't joinable until a SECOND create-game click; browser-vs-server state not reconciled.

DELIVERABLE (design FIRST, before code): a screen-by-screen storyboard in ai_docs/ for the full flow — landing -> name -> create/join -> waiting room -> launch -> in-game — covering create+join x TUI+Native, explicitly mapping browser-side vs server-side state. Target: ONE deck-picker, ONE create action, immediately joinable. Get user sign-off, THEN implement.

Code sites: web/index.html (1095 lines): panes pane-name@284 / pane-lobby@303 / pane-waiting@409; create-deck <select>@330; btn-create@346; loadDeckNames@573; showWaitingRoom@863; redirectToGamePage@920; createGame@951; joinGame@974. web/lobby_launcher.js: buildRedirectUrl@68 / consumeLobbyParams@110 / applyLobbyParamsToForm@153 / buildLobbyAction@220. Server: mtg-engine/src/network/lobby.rs (LobbyState@132, waiting_games@134, list_waiting_paged@183) + protocol.rs (ListGames/CreateGame/JoinGame msg types).

BLOCKS mtg-1vwpd + mtg-dw9j3 (implement against this storyboard). Builds on mtg-465 (Phase 1), mtg-187 (Phase 2).

## Deck Editor (4th page) — COMPLETED (deck-editor-page branch)

Implemented web/deck_editor.html as a WASM-free deck builder:
- Card catalog: JSON export (catalog.<hash>.json, content-addressed) added to run_export_wasm pipeline; index.json gains card_catalog field
- Search/filter by name, type text, color, CMC; 200-item cap
- Add/remove cards with qty controls; 4-of limit; mana curve chart
- Save/load decks to localStorage (mtg_custom_decks)
- Import/export .dck format (proper tokenized parse mirroring DeckLoader::parse_with_problems)
- "Use in Lobby" button: saves deck + writes mtg_lobby_deck_preselect → lobby pre-selects it
- lobby deck picker shows custom decks (prefixed "[Custom]") merged with built-in decks
- Reachable from lobby's Deck Editor button (stub replaced with live link)
- Playwright E2E test (test_deck_editor.js) added to validate-wasm-e2e-step

Branch deck-editor-page, commits 75650b43 + 4db80806, awaiting make validate + coordinator merge.
