---
title: 'feat(wasm): split cards.bin into per-set YYYY-SETCODE.bin files for on-demand load'
status: in_progress
priority: 2
issue_type: feature
labels:
- wasm
- optimization
created_at: 2026-05-27T18:31:45.749126052+00:00
updated_at: 2026-05-27T22:38:00.106191147+00:00
---

# Description

[implementation in progress on branch impl-mtg-6fsjb]

## Summary

Today the WASM client downloads a single monolithic `cards.bin` (~24 MB) containing all 32,434 card definitions. This issue proposes splitting that artifact into one file per Magic set, named `YYYY-<SETCODE>.bin`, and changing the WASM loader to fetch only the per-set files the current game/decks actually need. Strictly replaces (does not coexist with) `cards.bin`. The earlier per-deck `deck_cards/*.bin` pack optimization is made obsolete and is removed by this change.

## Implementation status (2026-05-27)

- Exporter rewritten: `mtg-engine/src/main.rs` `run_export_wasm` now writes
  `web/data/sets/<YYYY>-<CODE>.bin` (315 files, ~32.6 MB) + `sets/index.json`
  (~1.1 MB, ~32k entries). Deletes the old `cards.bin`, `deck_cards/*`,
  `deck_index.json` write paths.
- `loader::edition` extended with `PrimarySetAssignment` that walks the
  editions/ tree and assigns each card to its earliest printing (year, code).
  878 cards in cardsfolder are not in any edition file -> bucketed in
  `0000-MISC.bin`. (Sanity-checked: every edition file in the tree today has
  a valid 4-digit Date= header, so the year==0 fallback path is dormant.)
- WASM loader: `WasmCardDatabase::load_cards` and `load_deck_pack` removed;
  new `load_set(&[u8])` idempotently merges per-set bins (mirrors the
  existing `load_tokens` pattern).
- `loader::DeckPack` struct deleted (it had no callers outside the
  WASM/export path; removed from `loader::mod.rs` re-exports too).
- JS callers rewritten to fetch only the union of needed set bins via
  `Promise.all`: `web/tui_game.html`, `web/native_game.html`,
  `web/demo.html`, `web/wasm_ai_harness.html`, `agentplay/lib/wasm_process.py`.
  Per-game cold-load expected to drop from a single 24 MB blocking
  fetch to ~3-8 small parallel set fetches (typical curated 60-card deck
  spans 5-20 sets, each set bin 50-500 KB).
- Round-trip test added at `mtg-engine/tests/per_set_roundtrip.rs`:
  every cardsfolder entry resolves via index.json to its assigned set bin
  and deserialises cleanly. Manifest invariants (byte-size match,
  card_count sum) verified.
