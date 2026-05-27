---
title: Landing-page lobby never sends create_game / join_game over WebSocket
status: open
priority: 2
issue_type: bug
labels:
- web
- network
- landing-page
created_at: 2026-05-27T18:33:46.476733752+00:00
updated_at: 2026-05-27T18:33:46.476733752+00:00
---

# Description

BLOCKING bug found by Playwright QA on commit d8b2448f (branch landing-page-lobby).

The new web/index.html lobby renders a 'Create & Wait' button that, on submit, redirects to native_game.html?lobby=create&game=...&pass=...&name=...&ws=.... It NEVER sends a client_message::CreateGame to the WebSocket server. Symmetrically, joinGame() redirects to native_game.html?lobby=join&... without sending JoinGame.

native_game.html (and tui_game.html) have ZERO references to 'lobby', 'searchParams', or 'URLSearchParams' — the entire query-string contract is silently dropped. Result: a second browser hitting the lobby never sees the just-created game (verified end-to-end with two Playwright contexts; bob's Open Games stayed empty after alice clicked Create).

The commit message acknowledges this is a stub but the UI does not. Recommended fix: have the lobby itself issue CreateGame/JoinGame over its already-open WebSocket and render an in-place 'Waiting for opponent…' state, redirecting only once the slot is filled.

Repro: see ai_docs/landing_page_ux_qa_20260527.md (BLOCKING-1, BLOCKING-2) and web/test_landing_page_ux.js.
