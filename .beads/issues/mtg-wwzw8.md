---
title: Content-address tokens.bin + decks.bin (fix multiplayer cache skew)
status: open
priority: 2
issue_type: bug
created_at: 2026-05-31T16:03:16.931140445+00:00
updated_at: 2026-05-31T16:03:16.931140445+00:00
---

# Description

CONFIRMED production bug on deepscry.net: tokens.bin and decks.bin were served under FIXED names (web/data/tokens.bin, web/data/decks.bin) with cache-control public,max-age=14400 (4h). Their CONTENT changes every deploy (enum layout drift) but the URL never changed, so browsers reused the old cached bin against new WASM and crashed: 'Failed to deserialize tokens: tag for enum is not valid, found 16'. Broke 2/3 of a user's machines; the clean-cache one worked.

FIX (robust, matches the per-set bin pattern):
- mtg export-wasm now emits tokens.<hash>.bin + decks.<hash>.bin (blake3 via asset_hash::asset_hash_hex) and records the hashed names in data/sets/index.json (new 'tokens'/'decks' string fields, relative to data/). index.json is now written LAST (after both bins are hashed). A sweep removes stale tokens.*/decks.* bins each export.
- All fetchers resolve the hashed name from setIndex.tokens / setIndex.decks instead of the fixed ./data/{tokens,decks}.bin: native_game.html, tui_game.html, demo.html, wasm_ai_harness.html, test_decouple_step3_launch_game_session.js. The web_server is-content-addressed middleware already serves <stem>.<16hex>.<ext> immutable, so the hashed bins are now safe to long-cache AND a content change yields a new URL => guaranteed cache-miss.
- deploy-cloud.sh + mtg_wasm_game.py prereq checks resolve the bin via the manifest instead of the retired fixed path.

SMOKE-TEST GAP CLOSED:
- test_web_server_smoke.js (runs in make validate's network-e2e step AND the deploy pre-deploy gate) now asserts index.json.tokens/.decks are content-addressed, exist on disk, served 200+immutable, and the fixed names 404. No fixed-name long-cached data bin remains.
- Added a headless WASM-boot smoke to the deploy pre-deploy gate (scripts/mtg_wasm_game.py): boots the real WASM, deserializes tokens+decks+set bins, launches a game; aborts deploy on any deserialize error. Chromium-gated (skip-with-warning if unavailable). The existing test_decouple_step3 e2e (in make validate) already boots+deserializes+launches a game and was updated to the manifest path, so validate catches code-vs-data skew too.

Negative test verified: corrupting the manifest-resolved decks bin makes the WASM boot fail loudly (game never launches).
