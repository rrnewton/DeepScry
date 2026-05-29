---
title: Full CAS asset-graph hashing — hash everything except index.html
status: open
priority: 2
issue_type: feature
created_at: 2026-05-29T15:04:08.883549125+00:00
updated_at: 2026-05-29T15:04:08.883549125+00:00
---

# Description

Full CAS asset-graph hashing: content-address everything except index.html

User request 2026-05-29: for consistency + deploy observability (hash changes ⇒ new deploy shipped), content-address VIRTUALLY EVERY web asset, leaving ONLY the top-level entrypoint index.html unhashed (it points into the hashed rest).

CURRENT STATE: `mtg hash-web-assets` only content-addresses the wasm-bindgen pkg pair (mtg_engine.<h>.js + mtg_engine_bg.<h>.wasm) + rewrites HTML import/init. Per-set .bin already hashed by the exporter (logical→hashed map in data/sets/index.json). Everything else (index.json 1.7MB, native_game.html/tui_game.html/demo.html >100K, network.js, bug_report.js, server-config.js) is served short-TTL/no-cache but NOT hashed.

TARGET: extend hash-web-assets into a recursive asset-graph content-addresser rooted at index.html:
- index.html = the ONLY unhashed file (stable URL, short-TTL/no-cache). Everything reachable from it is hashed (immutable).
- Hash + rewrite-references for: the other HTML pages (native_game/tui_game/demo/wasm_ai_harness), index.json (the set resolver), network.js, bug_report.js, server-config.js, the pkg pair (already), set bins (already).

FEASIBILITY (confirmed by reference-graph trace 2026-05-29): all references are rewriteable post-build; NONE require editing compiled wasm:
- index.json is fetched via JS string `fetch('./data/sets/index.json')` in tui_game.html/demo.html/wasm_ai_harness.html (NOT a hardcoded wasm path) → rewrite the JS string to the hashed name.
- HTML→HTML: index.html references native_game.html/tui_game.html/demo.html (launch buttons + lobby JS redirect with ?lobby=&game=&pass=&name=&ws= query params — rewriter MUST preserve the query string, replace only the filename token).
- JS loads: index.html→server-config.js; native_game.html/tui_game.html→bug_report.js+network.js; bug_report.js→network.js → rewrite each loader reference.

IMPLEMENTATION NOTES:
- Recursive walk from index.html; hash leaves first, rewrite referrers to hashed names (bottom-up), so a file's hash is computed AFTER its own references are rewritten (a referrer's content—and thus hash—depends on the hashed names it points to). Order matters: hash + rewrite in dependency order.
- Reference rewriting must be precise (filename-token aware), not blind substring replace — keep DRY with the existing pkg-pair rewriter in asset_hash::web_pkg. Preserve query strings + fragments.
- Server side (web_server/mod.rs): index.html → short-TTL/no-cache; the dedicated /data/sets/index.json no-cache carve-out should move to "the hashed index.<h>.json is immutable" + only index.html stays no-cache. Update the cache-tier logic accordingly.
- Update web/test_web_server_smoke.js (the hermetic pre-deploy gate) to assert the NEW invariant: only index.html unhashed+no-cache; index.json/html/js all hashed+immutable; references resolve.
- Observability bonus: since index.html is the only stable URL, diffing its bytes (or the set of hashed names it points to) across deploys shows exactly what changed.

GOTCHAS: deep-links/bookmarks to native_game.html/tui_game.html will change each deploy (acceptable per the design — index.html is the entry; the lobby builds the redirect). Anything that hardcodes a non-index.html path (tests, docs) must go through index.html or be updated.

This edits web/*.html heavily — SEQUENCE AFTER the branding/version web work (mtg-faxy5/czueh/1iu9b) to avoid conflicts.
