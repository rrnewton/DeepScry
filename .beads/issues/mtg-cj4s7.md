---
title: 'deploy-cloud.sh post-deploy probe false-fails: uses :8080 HTTPS through Cloudflare (stale URL)'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-30T20:52:58.321899695+00:00
updated_at: 2026-06-01T13:46:24.033094595+00:00
---

# Description

## deploy-cloud.sh post-deploy probe false-fails: uses :8080 HTTPS through Cloudflare — POSSIBLY FIXED

GARDENING (2026-06-01): possibly-stale, needs re-check — the probe code was updated but may or may not fix the original issue.

ORIGINAL BUG: probe hits https://deepscry.net:8080 which CF doesn't forward (CF proxies :443 → origin:8080).

CURRENT CODE (deploy-cloud.sh:676-685):
- Comment says 'we probe the ORIGIN directly (scheme://host:port)' 
- Uses -k flag to skip TLS cert verification for direct-origin probe
- Addresses the mtg-581 false-failure case

UNCLEAR: if PUBLIC_HOST=deepscry.net and REMOTE_PORT=8080, the probe URL is https://deepscry.net:8080 which may still fail if CF doesn't pass through port 8080 connections.

NEEDS: a live deploy to verify the probe correctly passes or fails on a healthy deploy. The mtg-590 DONE note mentions the post-deploy probe passes now (/health sha=f85d828d, hashed wasm/js immutable, index.json no-cache, /lobby up) — which suggests the probe IS working correctly. May be already resolved.

Original evidence: 2026-05-30 deploy of c3266d41 returned EXIT=1 despite healthy service.
