---
title: Content-address tokens.bin + decks.bin (fix multiplayer cache skew)
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-31T16:03:16.931140445+00:00
updated_at: 2026-05-31T17:20:51.916663006+00:00
---

# Description

RESOLVED + DEPLOYED @17dfdef0 (2026-05-31). tokens.bin/decks.bin were fixed-name with max-age=14400 → stale-cached-vs-new-WASM enum-tag skew ('Failed to deserialize tokens: tag for enum is not valid, found 16') broke multiplayer on 2/3 machines. Fix: content-address tokens+decks bins via blake3 (tokens.<hash>.bin, decks.<hash>.bin) recorded in data/sets/index.json manifest (index.json itself also content-addressed by hash-web-assets → index.<hash>.json); fetches resolve via manifest; no fixed-name cacheable data bin can skew anymore. Added headless WASM-boot smoke to deploy pre-gate + content-address assertions to web-asset smoke. VERIFIED LIVE: web/smoke_test_live.js 2-client run → 0 HTTP failures, tokens+decks+5 set bins deserialize cleanly for both clients; /health sha=17dfdef0; old fixed-name /data/tokens.bin 404. NOTE: content-addressing means bookmarked native_game.<oldhash>.html URLs 404 after redeploy (GC); enter via deepscry.net/ landing (index.html, max-age=60).
