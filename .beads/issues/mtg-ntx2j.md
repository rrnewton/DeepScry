---
title: Content-address decks.bin / tokens.bin so they can be immutable
status: open
priority: 4
issue_type: task
created_at: 2026-05-28T17:00:52.939177199+00:00
updated_at: 2026-05-28T17:00:52.939177199+00:00
---

# Description

Follow-up to mtg-571. decks.bin and tokens.bin are fetched by FIXED name (`fetch('./data/decks.bin')`, `fetch('./data/tokens.bin')`), so per the web_server immutable INVARIANT they currently get no-cache routes (mtg-engine/src/web_server/mod.rs), unlike the now-hashed per-set bins.

GOAL: hash them too — either fold them into a tiny manifest (like sets/index.json) and rewrite the page fetch to read the hashed name from it, or hash inline + rewrite the literal fetch path in hash_web_assets.sh. Then move /data/decks.bin + /data/tokens.bin onto the immutable tier. Small perf + correctness win.
