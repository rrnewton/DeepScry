---
title: Adopt trunk for hashed/versioned web assets + GC unreferenced files
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T03:35:12.814842813+00:00
updated_at: 2026-05-28T03:35:12.821209487+00:00
---

# Description

CLUSTER: UI/infra. Supersedes the "status quo" option in ai_docs/CONTENT_ADDRESSED_ASSETS_RESEARCH_20260528.md. User decision: USE trunk (despite the research doc's lean toward a custom hasher).

GOAL: content-addressed, immutable web asset filenames (name.<hash>.ext) so caching can be `immutable, max-age=1y` and stale-bundle bugs (mtg-2indh) become structurally impossible. index.html is the sole mutable pointer.

PLAN:
1. Introduce trunk (Trunk.toml) to build web/ as the dist target. Trunk content-hashes the wasm-bindgen JS+WASM pair and rewrites index.html references automatically — this is the part that kills mtg-2indh. wasm-bindgen naming wrinkle is handled because our pages call init() and trunk injects the hashed URL.
2. CAVEAT from research (mtg research doc): trunk copies arbitrary asset dirs (our web/data/sets/*.bin — 315 files, ~32MB) via `copy-dir` UN-HASHED. That's the largest asset and the one we most want immutable. So trunk alone does NOT content-address the .bin files. Options to cover them:
   a. Keep the per-set bins on the existing index.json-manifest scheme and content-hash them with a small post-build step that ALSO updates sets/index.json (index.json is already a name->file manifest — just write hashed names into it). Then content-hash index.json itself.
   b. OR teach trunk's pipeline to hash them (likely needs a custom trunk hook / pre-build).
   Decide during implementation; (a) is the lower-risk path and reuses mtg-6fsjb's manifest.
3. Multi-page wrinkle: trunk is SPA-oriented (single index.html). We have index.html + tui_game.html + native_game.html + demo.html. Verify trunk can emit multiple HTML entry points (data-trunk on each, or multiple build targets) OR keep the game pages on a lighter hashing path. This is the main trunk-fit risk the research flagged.
4. Update scripts/deploy-cloud.sh to build via `trunk build --release` into dist/ and rsync dist/ (with --delete) to the VM. Drop the bespoke `make wasm-network && cp pkg web/pkg` flow OR wrap it.
5. Set Cache-Control: immutable, max-age=31536000 on hashed assets; keep index.html + index.json at no-cache/short max-age (they're the mutable pointers).
6. CI/validate must stay hermetic (no deepscry.net) per CLAUDE.md.

GC of unreferenced files: see the deploy-script change — `trunk build` rebuilds dist/ clean each time (only-referenced trunk-managed assets survive), and `rsync --delete` propagates that pruning to the VM. For the non-trunk .bin files, a small mark-sweep from sets/index.json removes orphaned hashed bins locally before rsync. Document the GC story in the deploy script.

ACCEPTANCE: a redeploy produces hashed immutable filenames for JS/WASM (and ideally .bin); old hashed files no longer referenced by the new index.html/index.json are removed from the VM by rsync --delete; browsers never serve a stale bundle.
