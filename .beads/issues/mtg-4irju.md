---
title: 'CAS cache-hardening: eliminate the runtime asset-manifest; forward-only immutable hash graph with index.html as sole entry'
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T22:43:03.509560121+00:00
updated_at: 2026-06-02T22:43:03.509560121+00:00
---

# Description

User-confirmed design (2026-06-02). Today the launcher->game nav resolves through a RUNTIME manifest (web/asset_manifest.js + served asset-manifest.json) because the CAS renamer (asset_hash.rs) breaks the genuine {tui_game <-> native_game <-> lobby_launcher.js} cycle that way ('intra-cycle references resolve through a served runtime manifest'). That runtime resolution is a CACHE VULNERABILITY: a stale cached manifest/launcher serves an OLD hash -> 404 (hit live by the user; hard-refresh works around). Proper fix: break the cycle STRUCTURALLY so the hashed graph is a forward-only DAG with index.html the SOLE unhashed/no-cache entrypoint — point cycle back-edges at index.html (not the hashed launcher), eliminating the runtime manifest entirely. Then every link is immutable-by-content-hash and caching is always safe. Web/asset_hash.rs + page nav; separate from netarch. Relates mtg-682.
