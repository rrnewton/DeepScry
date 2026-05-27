---
title: Landing-page lobby never sends create_game / join_game over WebSocket
status: closed
priority: 2
issue_type: bug
labels:
- web
- network
- landing-page
created_at: 2026-05-27T18:33:46.476733752+00:00
updated_at: 2026-05-27T19:03:32.062553491+00:00
---

# Description

FIXED in branch fix-lobby-create-join (2026-05-27).

Root cause was as reported: index.html createGame()/joinGame() built a query
string and window.location.href = 'native_game.html?...' instead of sending
ClientMessage::CreateGame / JoinGame over the open lobby WebSocket. Receiving
pages had no lobby-param handler.

Fix (Option-A-flavored, single-page lobby): web/index.html now
 - Sends ClientMessage::CreateGame { name, password, game_name, game_password,
   player_name, deck } over the existing lobby WS (instead of redirecting).
 - Sends ClientMessage::JoinGame similarly.
 - Uses a hardcoded 60-card placeholder DeckSubmission that mirrors
   decks/combat_test_4ed.dck (server requires main_deck_size >= 40).
 - Renders a new in-page #pane-waiting card showing "Waiting for opponent…",
   "Server is holding the slot…", or "Joined! Waiting for the game to start…"
   based on the GameCreated / WaitingForOpponent / AuthResult / JoinFailed /
   ServerFull / Error messages received.
 - On game_started, surfaces "Game is LIVE on server" and notes (UI text)
   that in-page game rendering is still a follow-up.

Verification:
 - web/test_landing_page_ux.js: self-managed (spawns its own http.server and
   `mtg server` on random ports). All scenarios PASS, 0 findings, exit 0.
   The previously BLOCKING checks now succeed:
     * "qa-test-game" IS visible to bob after alice clicks Create.
     * Alice does NOT navigate away from the lobby (slot stays alive).
     * Bob's open-game create also stays in-page.
 - Wired into make validate-network-e2e-step.

Known follow-ups (not blockers, tracked separately):
 - In-page rendering of the actual game UI after pairing — needs the WASM
   network client to attach to the lobby's already-open WS (currently each
   game page opens its own WS via launch_network_game). Tracked as part of
   the "single-page lobby" UX work in mtg-i1ye3 follow-ons; existing standalone
   tui_game.html/native_game.html remain functional for solo/spectate.
 - Deck-picker UI in the lobby (today: hardcoded combat-test deck).
