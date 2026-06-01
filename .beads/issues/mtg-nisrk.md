---
title: 'Cloudflare cache purge: wire a purge command + creds (one-time card-image purge pending)'
status: open
priority: 3
issue_type: task
created_at: 2026-05-31T20:13:58.075526783+00:00
updated_at: 2026-05-31T20:29:56.411401631+00:00
---

# Description

One-time Cloudflare purge is the CLEANUP step AFTER fixing the root cause: the server caches 404s as immutable (mtg-NEW immutable-404 bug — see below). For normal play the purge is NOT needed (card art loads from Scryfall/Gatherer by default; correct content-addressed assets return 200). BLOCKER for the purge itself: no CF API token/zone in <parent>/.deepscry-deploy.env and no purge script. To enable: add a scoped Zone.Cache-Purge token + zone id and a 'scripts/deploy-cloud.sh purge [--images|--all]' subcommand; until then purge via the CF dashboard. Sequence: fix the immutable-404 bug FIRST, then purge once. Cross-ref the immutable-404 bug.
