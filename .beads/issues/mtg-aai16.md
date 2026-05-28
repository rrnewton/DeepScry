---
title: 'tui_game.html: cardDb.load_set is not a function (per-set WASM API drift)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T01:34:06.848503437+00:00
updated_at: 2026-05-28T01:34:06.848503437+00:00
---

# Description

## Symptom

Live smoke test 2026-05-28 vs https://deepscry.net. After redirect to `tui_game.html?lobby_create=...`, WASM load fails:

```
Failed to launch TUI: TypeError: cardDb.load_set is not a function
  at tui_game.html:1349 (loadSetFiles)
  at loadCardsForDecks (tui_game.html:1458)
  at launchTui (tui_game.html:2821)
```

## Context

The 2026-05-28 deploy (b03e22cc) introduced per-set WASM bins + per-set load API. `tui_game.html` calls `cardDb.load_set(...)` but the deployed wasm-bindgen module does not export that method. Either:

- the method was renamed (e.g. to `load_set_bin`, `add_set`, `register_set`) and the HTML wasn't updated, or
- the per-set deploy is using an older wasm bundle that predates the JS-facing API.

5 per-set bins did successfully download (`web/screenshots/live_smoke_findings.json` shows alice fetched 5 `/data/sets/*.bin` responses with 2xx), so the index fetch + per-set HTTP layer works — the failure is purely the JS->WASM call.

## Reproduction

```
DEEPSCRY_BASE_URL=https://deepscry.net node web/smoke_test_live.js
```

Then inspect `web/screenshots/live_smoke/03_alice_game_page.png` and the findings JSON.

## Impact

Even if mtg-vevb7 is fixed and the lobby reaches Connected, the game launch will still error out because the WASM card DB never gets loaded. End-to-end lobby->game flow is broken on production.
