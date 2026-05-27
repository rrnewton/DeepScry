---
title: 'feat(wasm): split cards.bin into per-set YYYY-SETCODE.bin files for on-demand load'
status: open
priority: 2
issue_type: feature
labels:
- wasm
- optimization
created_at: 2026-05-27T18:31:45.749126052+00:00
updated_at: 2026-05-27T18:31:45.749126052+00:00
---

# Description

## Summary

Today the WASM client downloads a single monolithic `cards.bin` (~24 MB) containing all 32,434 card definitions. This issue proposes splitting that artifact into one file per Magic set, named `YYYY-<SETCODE>.bin`, and changing the WASM loader to fetch only the per-set files the current game/decks actually need. Strictly replaces (does not coexist with) `cards.bin`. The earlier per-deck `deck_cards/*.bin` pack optimization (see commits `01be1e64`, `2c30f4af`, `50bcf9a2`) is made obsolete and is removed by this change.

## Motivation

- Cold load of `cards.bin` is the dominant pre-game latency on the web.
- Per-deck packs partially solved this but only for the small set of curated, pre-shipped decks; custom decks fall through to the 24 MB fallback (see `web/tui_game.html:1434-1438`).
- Per-set partitioning is general-purpose: every deck (curated, custom, future format) loads exactly the sets it needs and no more, and per-set files are heavily shared across decks, so the browser's HTTP cache amortizes across sessions.

## Design Decisions (the 10 questions)

### Q1. File naming
- Pattern: `YYYY-<SETCODE>.bin`, uppercase set code, four-digit year. Examples: `2017-AER.bin`, `1993-LEA.bin`, `2021-STA.bin`, `2022-UNF.bin`.
- Source of truth: `[metadata]` block of each file in `editions/` (symlink to `forge-java/forge-gui/res/editions/`). `Code=` and `Date=YYYY-MM-DD` are already parsed by `CardEditionIndex::parse_edition_file()` in `mtg-engine/src/loader/edition.rs:88-114`.
- Sets with `Date=` missing/unparseable (year == 0 today, see edition.rs:111): fall back to `0000-<SETCODE>.bin`. The implementer must scan all 665 edition files (`forge-java/forge-gui/res/editions/`) and confirm whether any actually parse to year==0; if any do, decide whether to source a year from a sibling field or accept the `0000-` prefix.
- Collisions: `(year, set_code)` is uniquely keyed by `Code=`; set codes are globally unique in Forge data. No collision risk.
- Lowercase vs upperscale: uppercase to match Forge's canonical set codes and the existing `CardPrinting.set_code` field.

### Q2. What is a "set"; partitioning rule
- A "set" is the set of card *names* listed in an edition file's `[cards]` section (parsed by `extract_cards_from_file()` in `edition.rs:117-145`).
- **Reprints are NOT duplicated** across per-set files. Each card definition is assigned to exactly one "primary set" = the earliest printing year (ties broken alphabetically on set code) using `CardEditionIndex::get_card_printings()`, which already returns printings sorted by year.
- The bootstrap index (Q4) maps every printed-card alias (`card_name -> primary_set_file`) so the loader knows which file to fetch for any name a deck references.
- Cards that exist in `cardsfolder/` but appear in no edition file (test cards, unreleased) are assigned to a synthetic `0000-MISC.bin`.

### Q3. Export-side change (`mtg-engine/src/main.rs:3520` `run_export_wasm`)
- Replace the `cards.bin` write at `main.rs:3576-3585` with a loop that:
  1. Calls `CardEditionIndex::load_from_directory(Path::new("editions"))` (already used at `main.rs:3108-3112`).
  2. Inverts the index to a `BTreeMap<SetKey, Vec<&CardDefinition>>` where `SetKey { year: u16, code: String }`. Use references and `BTreeMap::entry` (no clone, no collect into intermediate Vec — per CLAUDE.md "Avoid clone / Avoid collect").
  3. For each set: `bincode::serialize(&Vec<(&str, &CardDefinition)>)` into `<output>/sets/<year>-<CODE>.bin`. (Iterator-of-references serialization keeps allocation bounded; if `bincode` requires owned tuples, build the `Vec` from an iterator without first collecting names.)
  4. Writes a single bootstrap index `<output>/sets/index.json` (Q4) and a `<output>/sets/manifest.json` listing every per-set file + byte size for cache warming and debugging.
- Delete the `cards.bin` write entirely, the entire `deck_cards/*.bin` block (`main.rs:3707-3784`), the `deck_index.json` write (`main.rs:3786-3812`), and the `tokens.bin` write (`main.rs:3641-3650`). Tokens are exported as a single `tokens.bin` retained AS-IS for now (token scripts are not partitioned by set in the source data; out of scope).
- Output directory: `web/data/sets/`. Reuse existing `web/data/` deploy path (`scripts/deploy-cloud.sh`).

### Q4. Load-side change & bootstrap index
- **Bootstrap `sets/index.json`** is necessary. It is tiny (~32k entries × ~30 B JSON each ≈ 1 MB raw, ~200 KB gzipped over the wire). Without it the loader cannot know which set file owns "Lightning Bolt" without already having an index.
- Schema:
  ```json
  {
    "version": 1,
    "sets": [{"file": "1993-LEA.bin", "bytes": 12345, "card_count": 302}, ...],
    "cards": {"Lightning Bolt": "1993-LEA.bin", "Counterspell": "1993-LEA.bin", ...}
  }
  ```
  Keyed on canonical card name (the same key used in `card_definitions: HashMap<String, CardDefinition>` at `main.rs:3541`).
- **Strategy: lazy / on-demand at game launch** — simplest path that satisfies the user's "on-demand load the sets needed for a game". The deck-builder path (which today calls `loadAllCards()` at `web/tui_game.html:3914-3920`) keeps a "Load All" affordance that downloads every set file in parallel via `Promise.all`.
- Pseudocode (replaces `loadCardsForDecks()` at `web/tui_game.html:1382-1454`):
  ```js
  async function loadCardsForDecks(p1Deck, p2Deck) {
    const decks = [p1Deck, p2Deck];
    const cardNames = new Set();
    for (const d of decks) for (const n of cardDb.get_deck_card_names(d)) cardNames.add(n);
    const setFiles = new Set();
    for (const n of cardNames) {
      const f = setIndex.cards[n];
      if (!f) throw new Error(`Card ${n} not found in set index`);
      setFiles.add(f);
    }
    await Promise.all([...setFiles].map(async f => {
      const r = await fetch(`./data/sets/${f}`);
      if (!r.ok) throw new Error(`Failed ${f}: ${r.status}`);
      cardDb.load_set(new Uint8Array(await r.arrayBuffer()));
    }));
  }
  ```
- Custom deck uploads: same path — extract card names, look them up in `setIndex.cards`, fetch the union of set files. No more "custom deck = load 24 MB fallback".

### Q5. Browser fetch coordination & caching
- Parallel fetch with `Promise.all`; one HTTP request per needed set. The browser's HTTP cache (with hashed-content filenames or appropriate `Cache-Control`) makes subsequent games on overlapping sets instant.
- No `localStorage` / `IndexedDB`. Per-set files are immutable artifacts; the HTTP cache layer is sufficient. Out of scope: service-worker offline caching (Q10).
- WASM side adds `WasmCardDatabase::load_set(&[u8])` in `mtg-engine/src/wasm/mod.rs` (replacing/joining `load_cards` at line 176). Each `load_set` call merges into the existing `cards: HashMap<String, Arc<CardDefinition>>` — already an idempotent merge pattern (see `load_tokens` at line 195).

### Q6. Backwards compatibility
- **No compat layer.** Per the user requirement "We should strictly replace loading one card binary file with loading N per-set binary files":
  - Remove `WasmCardDatabase::load_cards()` entirely (mod.rs:176-185).
  - Remove all `cards.bin` fetches in `web/index.html`, `web/tui_game.html`, `web/native_game.html`.
  - Remove the entire `deck_cards/*.bin` and `deck_index.json` code paths (export + JS loader). Delete `loader/DeckPack` struct if it has no other users.
  - The Makefile `wasm-export` target description (`Makefile:619-628`) updates accordingly; warn about stale `web/data/sets/` and clean it on each export.

### Q7. Test plan
- **Round-trip unit test (`mtg-engine/tests/per_set_roundtrip.rs`):** run `export-wasm` to a temp dir, then for every card name in the original 32,434-card map, (a) look up its primary set in `sets/index.json`, (b) deserialize that set's bincode file, (c) assert byte-equal `CardDefinition`. Must run in `make validate`.
- **WASM e2e (`web/test_per_set_load.js`):** boot the WASM loader, launch a game with a curated deck, assert `cardDb.card_count()` < 1000 and that all deck cards resolve. Add to `make wasm-e2e` and `make validate`.
- **Manifest invariants test:** every file in `sets/manifest.json` exists on disk, byte size matches, and the union of `cards.bin` keys equals `index.json.cards` keys.
- **Custom-deck regression:** upload a deck whose cards span 5 sets, assert exactly 5 set files are fetched (instrument via `performance.getEntriesByType('resource')`).
- **CI smoke:** `cargo test --features wasm-tui -p mtg-engine per_set_` runs in the existing CI matrix.

### Q8. Interaction with prior per-deck-pack optimization
- This **replaces** the per-deck packs. The pack optimization (commit `01be1e64`) was a special-case of what per-set partitioning generalizes:
  - Per-deck packs: O(decks) curated bundles, no help for custom decks.
  - Per-set packs: O(sets) bundles, automatically right-sized for any deck (curated, custom, future formats), heavy cache sharing between games.
- Delete `loader::DeckPack`, `cardDb.load_deck_pack()`, `data/deck_cards/`, `data/deck_index.json`. The `tokens.bin` global stays (token partitioning is out of scope; see Q10).

### Q9. Deploy script (`scripts/deploy-cloud.sh`)
- No script change required: rsyncs the `web/` tree, so `web/data/sets/*.bin` ride along.
- Document in the commit message that the deploy uploads ~600 new small files (~32 MB total, comparable to the old single `cards.bin` plus `deck_cards/`). One-time cache miss; subsequent visits are largely cache hits.

### Q10. Out of scope
- Service-worker offline caching of `sets/*.bin`.
- Progressive sets (streaming a set file via Range requests).
- Token-file partitioning (tokens stay in monolithic `tokens.bin`; ~32 KB, not a latency concern).
- Image-asset partitioning.
- HTTP/2 push or 103 Early Hints for set prefetch.
- Versioned/hashed filenames for cache-busting (assume immutable + revisit later).
- A fancy WASM-side LRU set eviction (browser HTTP cache suffices).
- Splitting `decks.bin` (already ~18 KB, not a bottleneck).

## Critical Files (entry points for the implementer)
- `mtg-engine/src/main.rs:3520-3840` — `run_export_wasm`; primary writer to rewrite.
- `mtg-engine/src/loader/edition.rs` — `CardEditionIndex`, source of `(year, set_code)` truth.
- `mtg-engine/src/wasm/mod.rs:140-260` — `WasmCardDatabase` loader API.
- `web/tui_game.html:1180-1454, 1308-1454, 3914-3920` — JS loader paths.
- `web/index.html:281` and `web/native_game.html:1210-1290` — secondary JS loaders.
- `Makefile:614-628` — `wasm-export` target.

## Rollout sequence
Single PR is fine; the change is mechanically large but logically atomic. Order within the PR:
1. Add `run_export_wasm` per-set output alongside existing `cards.bin` (temporarily both). Land the round-trip Rust test.
2. Switch JS loaders to per-set fetch path. Land the WASM e2e test.
3. Delete `cards.bin`, `tokens.bin`-as-fallback-cards path (NOT tokens.bin itself), `deck_cards/`, `deck_index.json`, `DeckPack`, `load_cards`, `load_deck_pack`. Single commit at end of PR for the cleanup.

Final state: `web/data/` contains `decks.bin`, `tokens.bin`, `sets/index.json`, `sets/manifest.json`, `sets/<YYYY>-<CODE>.bin × N`.
