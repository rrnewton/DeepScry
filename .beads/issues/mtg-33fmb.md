---
title: 'Content-addressing hole: cards.bin + harness pages still use fixed-name data bins'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-31T20:13:58.069662776+00:00
updated_at: 2026-05-31T20:18:40.473097476+00:00
---

# Description

Finish the content-addressing from mtg-wwzw8 (tokens+decks now hashed; fixed-name /data/tokens.bin + /data/decks.bin 404 live). REMAINING fixed-name data-bin fetches: web/wasm_ai_harness.html:83 (cards.bin), :90 (decks.bin); web/tui_game.html:1197 (cards.bin). cards.bin may be dead after the per-set split (mtg-6fsjb) — confirm; then either content-address it via data/sets/index.json or delete the dead refs. Every consumer must resolve through the manifest (no fixed-name data-bin fetch anywhere). Add an assertion to web/test_web_server_smoke.js.
Exporter: mtg-engine/src/main.rs (run_export_wasm); hashing: mtg-engine/src/asset_hash.rs. Closes the residual of mtg-612 (close that when this lands). Same bug class as mtg-wwzw8.
