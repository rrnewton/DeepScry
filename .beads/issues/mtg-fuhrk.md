---
title: 'Durable deck storage on Cloudflare R2 (OAuth + prefix-scoped temp creds + per-user .tgz) -- GATED: needs human approval + R2 account setup'
status: open
priority: 3
issue_type: task
labels:
- design
- blocked
- web
created_at: 2026-06-03T21:13:12.163158889+00:00
updated_at: 2026-06-03T21:13:12.163158889+00:00
---

# Description

*** DO NOT START — DESIGN ONLY, DOUBLY GATED ***
GATE 1 (human approval): requires EXPLICIT human go-ahead before any implementation. User direction 2026-06-03: "make a minibeads issue for the R2 work, but make it clear it is gated on HUMAN approval to start and on me setting up the R2 account."
GATE 2 (external provisioning): BLOCKED until the user provisions the Cloudflare R2 account — bucket + a parent R2 API token + an OAuth app (GitHub/Google) client id/secret. No agent may pick this up or begin coding until BOTH gates clear. Keep priority low; this is a captured design, not ready work.

WHY THIS MATTERS NOW: today, user-created custom decks live ONLY in browser localStorage (key 'mtg-forge-custom-decks', written by web/launcher.html + web/deck_editor.html). That is a live data-loss risk — Safari ITP wipes script-writable storage after ~7 idle days, "clear browsing data" nukes it, device loss = gone, and nothing syncs cross-device. The single deploy VM has no replication/backup and MUST NOT be anyone's durable data store.

DESIGN (converged 2026-06-03 across several rounds):
- STORE OF RECORD = Cloudflare R2 (11-nines durability, replicated, ZERO egress). Decks are KB; cost is operation-dominated and trivial: FREE through ~100K users (R2 free tier = 10 GB storage + 1M Class A + 10M Class B ops/month), ~$50/month even if a full 1M users are monthly-active. Storage negligible (~$1.50/mo at 1M packed collections), egress $0. Verified against developers.cloudflare.com/r2/pricing.
- IDENTITY = OAuth ("Sign in with GitHub/Google"). No password DB to host or back up. Derive each user's R2 key prefix deterministically from their stable OAuth subject id: decks/<oauth-sub>/. The VM therefore holds ZERO durable user data; durable state lives entirely in R2 + the OAuth provider (both external + durable). VM is fully disposable (crash/redeploy/replace = zero data loss).
- MINIMALIST SERVER = stateless coordinator. Holds the single R2 parent API token; verifies the OAuth session; mints SHORT-TTL (5-15 min), PREFIX-SCOPED R2 TEMPORARY CREDENTIALS (scoped to decks/<oauth-sub>/ + actions GetObject/PutObject/ListObjectsV2) via LOCAL JWT signing of the parent token (no R2 round-trip) — or a single-object presigned URL (max 7-day TTL). The server NEVER proxies bytes; the browser talks directly to <account>.r2.cloudflarestorage.com.
- STORAGE LAYOUT = one mutable per-user collection file, packed as a gzipped tar (.tgz; .zip also fine) of plaintext deck files. Edit loop: browser GET -> native DecompressionStream('gzip') + tiny tar parser (fflate, ~8KB) -> edit -> re-tar+gzip -> PUT with If-Match (ETag) CONDITIONAL WRITE to prevent cross-device last-write-wins clobbering. Debounce saves (R2 same-key write limit ~1/sec). Store the object OPAQUE (Content-Type application/gzip, NO Content-Encoding: gzip) so stored bytes == downloaded bytes == a genuine file. CORS on the bucket for the deepscry.net origin (PUT/GET/HEAD, Content-Type, expose ETag).
- BROWSER CACHE = IndexedDB for offline read/edit only (NOT source of truth; Safari ITP can evict). Hydrate from R2 on load; queue offline edits, sync on reconnect.
- DATA LIBERATION (the elegant property): the storage format IS the export. A "Download my decks" button mints a fresh presigned GET (response-content-disposition: attachment; filename="decks.tgz") -> the user downloads the real .tgz -> `tar xzf` -> every deck as plaintext. Zero export tooling, zero lock-in. The plaintext "N Cardname" (+ "SB:" sideboard) format is universal interop (any tool can import it).

OPTIONAL FUTURE EXTENSION (separate, also design-only): opt-in "Publish deck to a permanent content-addressed PUBLIC URL" — decks.<host>/<blake3> -> plaintext, immutable + CDN-cacheable + shareable + integrity-verifiable + dedup'd (reuse asset_hash.rs blake3 CAS). This makes a deck a first-class "file on the internet with its own URL" — the gap the incumbents leave (Moxfield/Archidekt/etc. are HTML-first with text export as an afterthought). For evolving decks, a short stable id -> latest-hash redirect (IPNS-style mutable pointer). Distinct from the private-by-default collection above. (Survey of incumbent direct-text-URL support in progress.)

RELATED: mtg-3t7gd (scalable P2P + the same minimalist-stateless-server philosophy); asset_hash.rs CAS (content-addressing reuse); the web image CDN-table work (same hashed-static-asset + zero-egress-CDN philosophy).

IMPLEMENTATION ORDER (only after BOTH gates clear): provision R2 + OAuth app -> add OAuth verify + temp-cred-minting endpoint to the axum web_server (alongside /lobby, /health) -> client deck-collection .tgz read/write/sync + IndexedDB cache + Download button + If-Match conditional writes -> migrate existing localStorage decks into R2 on first login.
