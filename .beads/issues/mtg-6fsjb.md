---
title: 'feat(wasm): split cards.bin into per-set YYYY-SETCODE.bin files for on-demand load'
status: open
priority: 2
issue_type: feature
labels:
- wasm
- optimization
created_at: 2026-05-27T18:31:45.749126052+00:00
updated_at: 2026-05-27T22:58:55.578974699+00:00
---

# Description

[implementation complete; pushed to origin/impl-mtg-6fsjb 2026-05-27]

## Summary

Today the WASM client downloads a single monolithic `cards.bin` (~24 MB) containing all 32,434 card definitions. This issue proposes splitting that artifact into one file per Magic set, named `YYYY-<SETCODE>.bin`, and changing the WASM loader to fetch only the per-set files the current game/decks actually need. Strictly replaces (does not coexist with) `cards.bin`. The earlier per-deck `deck_cards/*.bin` pack optimization is made obsolete and is removed by this change.

## Implementation status (2026-05-27, commits 848da788..2d46d15d)

Pushed to `origin/impl-mtg-6fsjb`. Validation `make validate-impl-no-network`
PASSED at SHA 2d46d15d (validate_logs/validate_2d46d15d*.log). Network e2e
also green via `make validate-network-e2e-step`.

Files touched:
- mtg-engine/src/main.rs: rewrote run_export_wasm to emit per-set bins
  instead of cards.bin + deck_cards/*. Deleted deck_index.json output.
- mtg-engine/src/loader/edition.rs: added PrimarySetAssignment::scan that
  assigns each card to its earliest printing (year, set_code), keyed by
  the original-case card name.
- mtg-engine/src/loader/deck.rs: deleted DeckPack struct (no callers).
- mtg-engine/src/wasm/mod.rs: removed load_cards / load_deck_pack;
  added load_set(&[u8]) (idempotent merge, mirrors load_tokens).
- mtg-engine/tests/per_set_roundtrip.rs (new): structural roundtrip test.
- web/tui_game.html, native_game.html, demo.html, wasm_ai_harness.html,
  test_decouple_step3_launch_game_session.js, agentplay/lib/wasm_process.py:
  rewrote card loading to fetch sets/index.json + Promise.all the set bins.
- Makefile, agentplay/README.md: doc-updates.

Concrete numbers (today, cardsfolder=32434 cards, editions/=665 files):
  Per-set bins: 315 files, 32,604,046 bytes total
  Largest: 0000-MISC.bin (1,064,943 bytes, 878 orphan cards)
  sets/index.json: 1,122,790 bytes
  tokens.bin (unchanged): 271,075 bytes
  decks.bin (unchanged): 20,269 bytes

Per-game cold load expectation: typical curated 60-card deck spans ~5-20
sets -> ~5-20 small parallel fetches (<<1 MB combined) instead of one
24 MB blocking fetch. Custom decks no longer need the full-database
fallback either.
