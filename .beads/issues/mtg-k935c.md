---
title: 'CAS: content-hash top-level image assets (immutable) so index.html is the only mutable file'
status: in_progress
priority: 3
issue_type: task
created_at: 2026-06-03T01:16:12.994848981+00:00
updated_at: 2026-06-03T05:43:55.115722170+00:00
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

== IMPLEMENTED 2026-06-02 on branch cas-image-hashing (off integration f24b242c; make validate GREEN) ==
asset_hash.rs::hash_full_graph now content-hashes TOP-LEVEL web/ image assets + site.webmanifest as immutable leaves:
- discover_top_level_images() auto-discovers by extension (HASHED_IMAGE_EXTS = webp/png/jpg/jpeg/gif/svg/ico), top-level only (no recurse) so web/images/ card-art is EXCLUDED.
- Image leaves hashed up front, folded into leaf_rules → every <img src>/<link href> repointed to <stem>.<hash>.<ext>.
- site.webmanifest hashed AFTER images (its icon-192/512 refs rewritten first), folded into leaf_rules so index.html's <link rel=manifest> repoints too → site.<hash>.webmanifest (immutable).
- image_leaves + webmanifest folded into the FULL manifest → release-token Merkle root now fingerprints images.
- GraphHashResult gained image_leaves + webmanifest fields; main.rs prints them.
Cache headers unchanged (is_content_addressed → immutable for free). Verified end-to-end on the real synced tree: 7 images (deepscry_logo/emblem-64/favicon/favicon-32/apple-touch-icon/icon-192/icon-512) + site.webmanifest hashed; all <img>/<link>/manifest refs repointed; stable names 404; web/images/ untouched.
TESTS: new in-module unit test top_level_images_and_webmanifest_are_content_hashed (hermetic, incl /images/ exclusion); extended test_deploy_tree_nav.js with a CONDITIONAL image block (images are gitignored → present locally after sync-web-assets.sh, absent in CI; hermetic coverage = the unit test). make validate GREEN, nav gate PASS incl 'image-asset hashing verified (7 top-level images staged)'. NEXT: team-lead diff-gate + merge + deploy. index.html is now the TRULY only mutable web file.
