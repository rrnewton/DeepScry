---
title: 'CAS: content-hash top-level image assets (immutable) so index.html is the only mutable file'
status: open
priority: 3
issue_type: task
created_at: 2026-06-03T01:16:12.994848981+00:00
updated_at: 2026-06-03T01:16:12.994848981+00:00
---

# Description

Follow-up to mtg-4irju (CAS pure-DAG cache-hardening). After that work, index.html is the SOLE mutable/no-cache file EXCEPT top-level image assets (e.g. web/deepscry_logo.webp referenced by the hero <img src> in index.html), which the renamer does NOT content-hash — they stay stable-named, served Cache-Control: public, max-age=60 (revalidate), not immutable. Close that consistency gap so EVERY non-index asset is content-addressed/immutable.

SCOPE (~25-30 lines + unit test, in mtg-engine/src/asset_hash.rs::asset_graph::hash_full_graph):
- Auto-discover top-level web/ image files by extension (.webp/.png/.jpg/.jpeg/.gif/.svg/.ico). EXCLUDE the web/images/** art-id namespace — already content-addressed by scryfall art_id + served immutable by its own scheme (web_server is_content_addressed / /images/ branch); do NOT double-hash or walk that subdir.
- Hash each as a pure LEAF (like HASHED_JS_LEAVES): rename to <stem>.<blake3>.<ext>, add to leaf_rules so the entry + page rewrites repoint <img src='logo.webp'> -> <img src='logo.<hash>.webp'> via the existing rewrite_one_reference (already matches quoted src attributes, query/fragment preserved).
- Fold the image leaves into the FULL manifest map (release token = Merkle root then also fingerprints images).
- Unit test: stage a tree with a top-level logo.webp referenced by index.html <img src> + a web/images/<artid>.jpg; assert the top-level logo is hashed + repointed + in the manifest, and the /images/ file is left untouched.
- Cache headers need NO change: is_content_addressed() already serves <stem>.<16hex>.<ext> immutable.

SEQUENCING (team-lead): GRAB only AFTER the logo-assets branch lands on integration, so this rebases onto integration-WITH-the-logo and actually has deepscry_logo.webp to hash. team-lead re-engages cas-dev once logo + CAS deploy are confirmed live.

VALIDATION: make validate green (extend test_deploy_tree_nav.js to stage + assert the hashed image too); confirm served logo carries immutable Cache-Control. owner: cas-dev (netarch team). Refs mtg-4irju.
