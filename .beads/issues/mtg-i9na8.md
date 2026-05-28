---
title: 'Web card loading: log set name + lazy-load only played decks'' sets (not all 26)'
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T19:48:29.601361983+00:00
updated_at: 2026-05-28T19:48:29.601361983+00:00
---

# Description

[human-reported, 2026-05-28] Web client card loading: improve logging + fix suspected eager over-loading.

1. LOGGING: dev-console messages like "load_set: +135 new cards (total: 135)" should name WHICH set is being loaded, e.g. "load_set <SET-CODE/name>: +135 new cards (total: 135)".

2. SUSPECTED OVER-LOAD: a single game (gabriel avatar draft deck vs chandra_tokens) loaded ~11,275 cards + ~800 tokens. That is far more than two decks need -- looks like an over-approximation. The dev console also reported 26 decks loaded. Hypothesis: the client is loading the card sets for ALL 26 decks in the lobby/deck list, not just the two decks selected for play.

3. INTENT: sets should load ON DEMAND, lazily, as the played decks require them -- only the two decks in the current game, not the whole deck catalog.

Context: relates to mtg-6fsjb (per-set WASM bin split + on-demand load via index.json). Investigate where the deck list / lobby preloads sets; ensure only the active game's two decks trigger set loads. Likely web/ loader + deck-list code.
