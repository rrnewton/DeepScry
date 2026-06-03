---
title: 'CAS cache-hardening: eliminate the runtime asset-manifest; forward-only immutable hash graph with index.html as sole entry'
status: in_progress
priority: 3
issue_type: task
created_at: 2026-06-02T22:43:03.509560121+00:00
updated_at: 2026-06-03T00:56:14.971077079+00:00
---

# Description

CAS cache-hardening — AGREED DESIGN (user-confirmed 2026-06-02). Eliminate the runtime asset-manifest; make the hashed graph a forward-only immutable DAG with index.html as the sole mutable entry; thread a per-release token through navigation.

== GOAL ==
Today the launcher->game nav resolves through a RUNTIME manifest (web/asset_manifest.js + served stable-named asset-manifest.json) because the renamer breaks the {tui_game <-> native_game <-> lobby_launcher.js} cycle that way. That runtime/stable-named layer is a CACHE VULNERABILITY: a stale cached manifest/launcher serves an OLD hash -> 404 (hit live by the user; only a hard-refresh recovers). Fix: collapse ALL mutability into index.html.

== PART 1: DAG-ify the graph (remove the cycle) ==
- Forward DAG: index.html (UNHASHED entry) -> launcher.<hash> -> {native_game.<hash>, tui_game.<hash>, deck_editor.<hash>} -> lobby_launcher.<hash>.js (leaf).
- REMOVE the "switch renderer" native<->tui direct links (user: drop it for now; it likely doesn't work). Kills that cycle edge.
- LEAF-IFY lobby_launcher.js: it must stop REFERENCING the game pages (today it builds redirect targets -> the other half of the cycle). Move "which page" knowledge up to index/launcher (forward edges). lobby_launcher.js becomes pure param-parsing, imported by pages, references nothing back.
- Back-edges (game->launcher/lobby, any cross-nav) go through index.html (see Part 3), NOT a hashed sibling.

== PART 2: eliminate the runtime manifest; hash it ==
- Build the manifest (logical-name -> hashed-name map for this release) and CONTENT-HASH it -> asset-manifest.<hash>.json (IMMUTABLE). This manifest hash transitively fingerprints the WHOLE release graph (a Merkle root) -> it IS the release identity ("release token").
- index.html (the ONE mutable, no-cache file) references asset-manifest.<hash>.json BY HASH (baked at deploy). So index.html's forward links AND its dispatch table come from the same release snapshot, atomically.
- DELETE: the stable-named web/asset_manifest.js loader, the stable-named asset-manifest.json, and the data-asset-href runtime href-rewrite. Replaced by static forward hashing + the index.html dispatcher.
- OPTIONAL: fold build metadata (git SHA, build time, /health sha) into the manifest before hashing, so the release token also pins provenance.

== PART 3: index.html as seed + stateless resolver ==
- index.html roles: (a) fresh-visit landing with raw hashed forward links to the LATEST release's child pages; (b) a stateless token-parameterized resolver: index.html?goto=<logical-name>&release=<manifest-hash> -> fetch asset-manifest.<manifest-hash>.json (immutable, retained) -> resolve -> redirect. Dispatcher logic is INLINE in index.html (never a stable-named loader -> that'd be cacheable again).
- Back-edges become stable unhashed URLs: index.html?goto=lobby&release=<token>. One same-origin hop, only on back-nav (forward nav stays direct hashed links).

== PART 4 (THE forward-compatible requirement the user emphasized): thread `release=` as a PARAM through the DAG ==
- The release token CANNOT be baked into the hashed pages' CONTENT: that's circular (manifest hash depends on page hashes; embedding the hash in pages changes the hash; no crypto-hash fixed point). The token can ONLY be baked into the UNHASHED index.html.
- So thread it as runtime navigation context: index.html (deploy bakes the current release=<token>) seeds it; EVERY forward link carries `&release=<token>` so the next page knows its release; pages RELAY release= from their own URL onto BOTH their forward links and their back-edges. Pages stay token-AGNOSTIC in content (hashes stable, manifest computable).
- CRITICAL (user): forward links must carry release=<hash> AND ALSO keep carrying our EXISTING params (deck, name, seed, ws, ui, mode, p1/p2, allow_local_img_load, images, img_src, debug, lobby_create/join, etc.). i.e. the param-threading helper must MERGE release= into the existing query string, never drop the others. (Today buildRedirectUrl / the sticky-param plumbing in lobby_launcher.js already forwards STICKY_PARAM_KEYS — add release to that set + ensure it propagates on forward links too, not just back.)
- sessionStorage MAY cache release= as a fallback, but the URL is the source of truth (shareable/reproducer/drain-pinned links carry it in the URL).

== DEFERRED (do NOT build now; forward-compatible only) ==
- MULTI-DEPLOYMENT / concurrent-drain resolver: when we run last/next versions concurrently, a session must stay on ITS release. The release= token already carried by back-edges (and forward links) makes this purely an OPS change: retain old asset-manifest.<hash>.json + their asset closures during the drain, and index.html resolves whatever release= it's handed (defaulting to latest if absent/GC'd). NO link-format or page change needed later. Build the param contract NOW; defer the actual multi-release retention/resolver.
- RETENTION POLICY: how long to keep each asset-manifest.<hash>.json + its transitive asset closure after a newer release supersedes it. Governs how long ?release=<hash> links (drain-pins, durable reproducer links) keep working. Decide when we actually need durable repro links / concurrent drains.
- Pinned-vs-latest reproducer links: ?goto=game&release=<token>&seed=... (pinned, exact-build repro, survives if retained) vs omit release (latest). Same token mechanism serves both.

== VALIDATION (mandatory before merge/deploy) ==
- Extend web/test_deploy_tree_nav.js for the new structure: walk the forward DAG + every back-edge; assert (1) no stable-named manifest/loader remains, (2) only index.html is unhashed, (3) every forward link carries release= AND preserves the other params, (4) back-edges resolve via index dispatch, (5) a stale-manifest scenario no longer 404s (the bug this fixes).
- make validate green; a real deploy + a returning-user (pre-cached) check that confirms no hard-refresh needed.
- Cache headers: index.html no-cache; everything else (incl. asset-manifest.<hash>.json) immutable/long-TTL.

Issue: this is mtg-4irju. Separate from netarch (web/asset_hash.rs + page nav). Branch off integration.

== IMPLEMENTED 2026-06-02 on branch cas-immutable-graph (make validate GREEN; deploy-tree nav gate PASS) ==
Pure-DAG rework landed:
- asset_hash.rs: Tarjan ASSERTS acyclicity (multi-node SCC = hard error naming files + 'route via index.html?goto='). Deleted MANIFEST_JS loader rewrite / stable asset-manifest.json / cycle-member machinery. lobby_launcher.js -> HASHED_JS_LEAVES (pure leaf). Builds FULL logical->hashed manifest (incl pkg+wasm = Merkle root), content-hashes it -> asset-manifest.<token>.json, bakes token into index.html via __MTG_RELEASE_TOKEN__ sentinel. Token PURELY content-derived (no build SHA/time). +4 in-module e2e tests + rewrote tests/asset_graph_hash.rs.
- Pages: leaf-ified lobby_launcher.js (buildRedirectUrl->buildRedirectQuery; 'release' in STICKY_PARAM_KEYS -> merges, never clobbers). launcher.html owns hashed game-page literals + relays release. Removed native<->tui switch links + asset_manifest imports. deck_editor back-edge -> index.html?goto=launcher (relays release), deleted resolveAssetName bridge. index.html: self-contained inline ?goto dispatcher (release-FIRST -> baked-latest -> identity; never hangs; short-circuits before lobby code for mtg-04ls8) + seeds release on forward links. Deleted asset_manifest.js.
- deploy-cloud.sh post-probe + test_deploy_tree_nav.js rewritten (5 new invariants incl STALE-MANIFEST no-404). Cache headers already correct -> NO server change.
DEFERRED items untouched. NEXT: team-lead diff-gate + merge + deploy + returning-user pre-cached check.
