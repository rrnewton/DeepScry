---
title: 'CDN image migration: ship compact name→Scryfall-CDN table; kill api.scryfall + gatherer (task #7, subsumes mtg-722)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-03T20:20:45.159784057+00:00
updated_at: 2026-06-03T20:24:17.068729319+00:00
---

# Description

Owner slot04, branch fix-mtg-722 (off integration 6da00024). Subsumes mtg-722. Team-lead task #7.

GOAL: the ONLY scryfall.com fetch is unique_artwork.json (cached locally). All image loads hit cards.scryfall.io CDN (immutable ~1yr) via URLs the client/downloader COMPUTE locally from a compact hashed-CAS table. Remove every api.scryfall.com builder/ref/test (no api fallback).

CASCADE (CORRECTED 2026-06-03 per user): [local-if-allowed -> CDN-from-table -> gatherer]. api.scryfall KILLED entirely; gatherer RETAINED as the table-MISS safety net (keep gatherer_url(), its cascade slot, the launcher/solo_launcher Gatherer checkbox, the gatherer classification). Coverage aliasing keeps gatherer rarely hit. (Earlier 'kill both' is reversed for gatherer only.)

DESIGN (locked w/ team-lead):
- CDN URL shape (verified live): https://cards.scryfall.io/<size>/front/<id0>/<id1>/<id>.jpg?<version>; DFC back = front->back substitution, same uuid+version.
- Table = encoding D (columnar), shipped RAW card-lookup.<blake3>.bin (transport-compressed via tower-http CompressionLayer + Cloudflare; ~866KB wire / ~1.3MB raw / ~35k entries). Layout: MAGIC SCDT | format_version=1 | reserved | u32 count | u32 names_len | names('\n'-joined,sorted,UTF-8) | N x 16B uuid col | N x u32 ver col (bit31=DFC flag). MAGIC+version -> future art-variant picker = format_version 2 (additive).
- Generator = an 'mtg' CLI subcommand: sole unique_artwork.json fetch + FORMAT-DRIFT hard-error (host==cards.scryfall.io, path <size>/<face>/<s1>/<s2>/<uuid>.<ext>?<ver>, s1==uuid[0] s2==uuid[1], url-uuid==.id, ext in {jpg,png}, ver parses, DFC front==back uuid+ver) -> refuse + keep old table on drift. Redownload IS regenerate.
- Selection: OLDEST art per identity (min released_at, prefer non-digital + image_status not missing/placeholder).
- Keys: real cards by NAME; TOKENS by composite (name + P/T + colors) — P/T strings ('*' for variable), colors sorted WUBRG set (empty=colorless). Normalize IDENTICALLY engine-side (CardDefinition.{power:Option<i8>,toughness:Option<i8>,colors:Vec<Color>}) and table-build-side (unique_artwork power/toughness/colors). Lookup: composite FIRST, then name-only fallback. Aliasing: Alchemy 'A-'->de-prefixed; DFC 'Front // Back'->both faces+combined; ' Token' suffix->bare name. EDGE: variable-* P/T tokens (engine stores Option<i8>) need care.
- Ship table folded into asset-manifest Merkle graph + index.html dispatcher (same CAS mechanism as data/sets/index.json, mtg-727).
- Client (image_overlay.rs + native_game.html/tui_game.html JS): cascade local->CDN->gatherer; DROP api.scryfall only. ~14 call sites (download.rs:138, image_overlay.rs:68/95 + cascade + 11 tests, tui_game.html, native_game.html:1205, launcher/solo_launcher Gatherer checkboxes RETAINED + add a CDN/Scryfall source, lobby_launcher.js IMAGE_SOURCE_IDS, smoke_test_live.js, test_redo_lobby_e2e.js).
- Prong B: 'mtg download' builds CDN URLs from the table (drop api.scryfall); prepopulate local /images incl. tokens (oldest art), rsync to VM (coordinate w/ team-lead).
- Visible attribution: "Card images via Scryfall - Magic: The Gathering (c) Wizards of the Coast" lobby footer + game page.

PROGRESS: scryfall shared core (cdn_image_url + image_version_from_url + CdnSize, 0a5efdad); encode/decode_card_lookup official SCDT layout + uuid<->bytes + token_lookup_key + DFC bit31 (f092ea90); beads filed (2511dd97). Dependency-free (native + wasm). 5 hermetic tests green incl. Clue-token end-to-end + DFC round-trip. fmt/clippy/wasm clean.

NEXT: generator subcommand (bulk fetch+cache + oldest-art selection + drift check + aliasing -> emit SCDT; show real table sizes to team-lead) -> asset_hash registration + CompressionLayer; client JS decoder + cascade (local->CDN->gatherer, drop api) at ~14 sites; download.rs Prong B; attribution; tests; validate + cite log; redeploy after merge. No MTG-rules-review.
