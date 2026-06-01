---
title: 'Bug: lobby-redo CAS deploy break — launcher.html + game-page cross-nav 404 on the content-hashed deploy'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-01T23:16:55.325082702+00:00
updated_at: 2026-06-01T23:16:55.325082702+00:00
---

# Description

## Bug (deploy-only): lobby-redo launcher hub + game-page cross-nav 404 on the content-hashed deploy

ROOT CAUSE (two design flaws in mtg-engine/src/asset_hash.rs, called out by the user):
1. Hardcoded HASHED_HTML_PAGES list. launcher.html (added by the lobby redo, mtg-682) was never added to it → it was not hashed → index.html's lobby redirect still emitted a bare 'launcher.html' → 404 on deploy (server serves only hashed names). Auto-discovery would have prevented this.
2. redirect_cross_html_to_entry rewrote EVERY non-entry page's cross-HTML link to index.html — a star-topology cycle-breaker. The redo added launcher.html as a SECOND forward hub and lobby_launcher.js hardcodes 'tui_game.html'/'native_game.html' (its GAME_PAGE map, a hashed leaf whose string literals were NOT rewritten) → game-page redirects 404'd / forward-nav flattened to the lobby.

A prior attempt (branch redo-fixed-name-html @93fccb72) backed off by un-hashing ALL HTML — REJECTED by the user. CAS-everywhere is the deployment model.

FIX (branch cas-general-renamer, mtg-682; keeps CAS for ALL assets incl. HTML):
- AUTO-DISCOVER the HTML set (dir glob of *.html, sorted), drop the hardcoded HASHED_HTML_PAGES. index.html (ENTRY_HTML) is the ONE stable bootstrap; asset_manifest.js is a second stable name (the manifest loader). Everything else hashed.
- GRAPH-AWARE rewrite: build the real reference graph; MODULE edges (import/<script src>) form a DAG → hash in topological order so each referrer's imports bake in already-hashed names. NO blanket redirect-to-entry.
- CYCLE handling: the genuine cycle is {tui_game.html, native_game.html, lobby_launcher.js}. Module edge (game pages → lobby_launcher.js) is one-way, so lobby_launcher hashes FIRST and the pages statically import its hashed name. The SOFT cycle edges (tui⇄native nav, lobby_launcher's GAME_PAGE literals) resolve at RUNTIME via a served manifest: asset-manifest.json + stable web/asset_manifest.js loader (resolveAsset / installManifestHrefRewrite). Identity on the dev tree (empty manifest), so dev and deploy share one code path. Documented in the asset_hash.rs module docs.

GATES (all green):
- web/test_deploy_tree_nav.js: HASHED-tree nav gate (stages via 'mtg hash-web-assets', serves, walks index→launcher→deck_editor + game pages + lobby_launcher GAME_PAGE through the manifest, asserts hashed 200s). Proven FAIL on the OLD renamer (6 failures: launcher.html not hashed; GAME_PAGE literals → /tui_game.html /native_game.html 404) / PASS on the new. Wired into make validate (validate-network-e2e-step).
- tests/asset_graph_hash.rs: reflects the auto-discovered set + manifest cycle-break.
- web/test_web_server_smoke.js: HTML still hashed + immutable; pkg/data/JS leaves stay hashed.

Web-asset-pipeline plumbing, not gameplay → no MTG rules review. base integration a9e64bfc.
