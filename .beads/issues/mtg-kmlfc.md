---
title: 'CDN image migration: ship compact name→Scryfall-CDN table; kill api.scryfall + gatherer (task #7, subsumes mtg-722)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-03T20:20:45.159784057+00:00
updated_at: 2026-06-03T20:21:23.098753258+00:00
---

# Description

Owner slot04, branch fix-mtg-722 (off integration 6da00024). Subsumes mtg-722 (replaces the suffix-strip band-aid). Team-lead task #7.

GOAL: the ONLY scryfall.com fetch is unique_artwork.json (cached locally). All image loads hit cards.scryfall.io CDN (immutable ~1yr) via URLs the client/downloader COMPUTE locally from a compact hashed-CAS table. Remove every api.scryfall.com + gatherer.wizards.com reference (no network fallback even on table-miss -> graceful name placeholder).

DESIGN (locked w/ team-lead):
- CDN URL shape (verified live 2026-06-03): https://cards.scryfall.io/<size>/front/<id0>/<id1>/<id>.jpg?<version>; DFC back = front->back substitution, same uuid+version.
- Table = encoding D (columnar), shipped as RAW card-lookup.<blake3>.bin (transport-compressed via tower-http CompressionLayer + Cloudflare; ~866KB wire / ~1.3MB raw / ~35k entries). Layout: MAGIC SCDT | format_version=1 | reserved | u32 count | u32 names_len | names('\n'-joined,sorted,UTF-8) | N x 16B uuid col | N x u32 ver col (bit31=DFC flag). MAGIC+version -> future art-variant picker = format_version 2 (additive).
- Generator = an 'mtg' CLI subcommand: sole unique_artwork.json fetch + FORMAT-DRIFT hard-error (host==cards.scryfall.io, path <size>/<face>/<s1>/<s2>/<uuid>.<ext>?<ver>, s1==uuid[0] s2==uuid[1], url-uuid==.id, ext in {jpg,png}, ver parses, DFC front==back uuid+ver) -> refuse + keep old table on drift. The redownload IS the regenerate flow.
- Selection: OLDEST art per identity (min released_at, prefer non-digital + image_status not missing/placeholder).
- Keys: real cards by NAME; TOKENS by composite (name + P/T + colors) — power/toughness strings with '*' for variable, colors sorted WUBRG set (empty=colorless). Normalize IDENTICALLY engine-side and table-build-side. Lookup: composite FIRST, then name-only fallback (never 404 a token). Coverage aliasing: Alchemy 'A-' prefix->de-prefixed; DFC 'Front // Back'->both faces+combined; ' Token' suffix->bare name.
- Ship table folded into the asset-manifest Merkle graph + index.html dispatcher (NOT a dynamic /api endpoint) — same CAS mechanism as data/sets/index.json (mtg-727).
- Client (image_overlay.rs + native_game.html/tui_game.html JS): cascade = [local-if-allowed -> CDN-from-table] ONLY. Drop gatherer + api.scryfall. ~14 call sites (download.rs:138, image_overlay.rs:68/95 + cascade + 11 tests, tui_game.html, native_game.html:1205, launcher/solo_launcher Gatherer checkboxes->rebrand CDN, lobby_launcher.js IMAGE_SOURCE_IDS, smoke_test_live.js, test_redo_lobby_e2e.js).
- Prong B: 'mtg download' builds CDN URLs from the table (drop api.scryfall); prepopulate local /images incl. tokens (oldest art), rsync to VM (coordinate w/ team-lead).
- Visible attribution: "Card images via Scryfall - Magic: The Gathering (c) Wizards of the Coast" on lobby footer + game page.

PROGRESS (this branch): mtg_engine::scryfall shared core landed — cdn_image_url + image_version_from_url + CdnSize (0a5efdad); encode/decode_card_lookup (official SCDT layout) + uuid<->bytes + token_lookup_key + DFC bit31 (f092ea90). Dependency-free (compiles native + wasm). 5 hermetic tests green incl. Clue-token end-to-end CDN URL + DFC round-trip. fmt/clippy/wasm clean.

NEXT: generator subcommand (bulk fetch+cache + oldest-art selection + drift check + aliasing -> emit SCDT) -> diagnose engine token P/T+colors accessor; asset_hash registration + CompressionLayer; client JS decoder + cascade rewrite + drop api/gatherer at the ~14 sites; download.rs Prong B; attribution; tests (smoke/deploy-tree assert hashed-immutable table + a token resolves); validate + cite log; redeploy after merge. No MTG-rules-review (no gameplay change).
