---
title: Unify native_game.html and tui_game.html on ONE common query-parameter dispatch interface
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T20:13:58.070866123+00:00
updated_at: 2026-05-31T20:24:52.454825016+00:00
---

# Description

Define + document ONE canonical query-param dispatch contract consumed identically by web/native_game.html + web/tui_game.html via web/lobby_launcher.js (consumeLobbyParams@110, applyLobbyParamsToForm@153, buildLobbyAction@220, buildRedirectUrl@68, GAME_PAGE@44, DEFAULT_UI@41). Param set: lobby/game id, pass, name, ws, deck, ui, mode(local|network), seed. index.html redirectToGamePage@920 already emits ?ui=tui|native — formalize the full set + add a test that identical params yield equivalent setup on both pages. Only the renderer differs. PARAM/DISPATCH half; controller/network half is mtg-tnsk7. Implement against the mtg-khy7x storyboard.
