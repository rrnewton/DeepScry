---
title: Content-address tokens.bin + decks.bin (fix multiplayer cache skew)
status: open
priority: 2
issue_type: bug
created_at: 2026-05-31T16:03:16.931140445+00:00
updated_at: 2026-05-31T16:29:15.441209737+00:00
---

# Description

CONFIRMED production bug on deepscry.net: tokens.bin and decks.bin were served under FIXED names (web/data/tokens.bin, web/data/decks.bin) with cache-control public,max-age=14400 (4h). Their CONTENT changes every deploy (enum layout drift) but the URL never changed, so browsers reused the old cached bin against new WASM and crashed: 'Failed to deserialize tokens: tag for enum is not valid, found 16'. Broke 2/3 of a user's machines; the clean-cache one worked.

FIX (commit 9807c63b — robust, matches the per-set bin pattern):
- mtg export-wasm now emits tokens.<hash>.bin + decks.<hash>.bin (blake3 via asset_hash::asset_hash_hex) and records the hashed names in data/sets/index.json (new 'tokens'/'decks' string fields, relative to data/). index.json is written LAST, after both bins are hashed. A sweep removes stale tokens.*/decks.* bins each export.
- All fetchers resolve the hashed name from setIndex.tokens / setIndex.decks instead of fixed ./data/{tokens,decks}.bin: native_game.html, tui_game.html, demo.html, wasm_ai_harness.html, test_decouple_step3. The web_server is-content-addressed middleware already serves <stem>.<16hex>.<ext> immutable, so hashed bins are safe to long-cache AND a content change => new URL => guaranteed cache-miss.
- deploy-cloud.sh artefact check + mtg_wasm_game.py prereq resolve via the manifest.

SMOKE-TEST GAP CLOSED:
- test_web_server_smoke.js (in make validate network-e2e step AND deploy pre-deploy gate) now asserts index.json.{tokens,decks} are content-addressed, exist, served 200+immutable, fixed names 404. No fixed-name long-cached data bin remains.
- Added headless WASM-boot smoke to the deploy pre-deploy gate (scripts/mtg_wasm_game.py): boots WASM, deserializes tokens+decks+set bins, launches a game; aborts deploy on any deserialize error. Chromium-gated. The existing test_decouple_step3 e2e in make validate already boots+deserializes+launches a game (updated to manifest path) so validate catches code-vs-data skew.
- Negative test verified: corrupting the manifest-resolved decks bin makes the WASM boot FAIL loudly (game never launches; e2e exits 1).

FOLLOW-UP (commit cf651192): the first full make validate at 9807c63b FAILED only because four additional WASM-driver/equivalence harnesses still referenced the retired fixed web/data/decks.bin name (the STRICT native-vs-WASM equiv sweep hard-failed on its '[ -f web/data/decks.bin ]' readiness probe). Updated all four to resolve the hashed name from data/sets/index.json: bug_finding/native_wasm_equiv_sweep.sh, bug_finding/native_wasm_equiv_sweep.py, agentplay/lib/wasm_process.py, agentplay/agent_game.py. Verified: the STRICT equiv sweep now PASSES (seed=1 the_deck_classic, 0 diverged) — the WASM side loaded the manifest-resolved decks+set bins and replayed byte-identically to native.

Files touched: mtg-engine/src/main.rs (export), mtg-engine/src/wasm/{mod.rs,README.md} (doc), web/{native_game,tui_game,demo,wasm_ai_harness}.html, web/test_{web_server_smoke,decouple_step3_launch_game_session}.js, scripts/{deploy-cloud.sh,mtg_wasm_game.py}, bug_finding/native_wasm_equiv_sweep.{sh,py}, agentplay/{agent_game.py,lib/wasm_process.py}. NO engine/card/protocol/undo changes; no tracked images.
