# R2 Durable Deck Storage (mtg-742)

How a player's custom decks are stored in the cloud so they follow the
player between devices. This is the OAuth-independent storage half; the
login leg is tracked separately (still blocked on the OAuth app).

## What it does, in plain language

Your custom decks normally live only in one browser (`localStorage`). With
cloud storage enabled, the website packs all your decks into one compressed
file (a `.tgz`) and uploads it to Cloudflare R2 cloud storage, so the same
decks appear when you open the site on another device. A "Download my decks"
button hands you that exact file, so your data is never locked in.

## Architecture

- **Store of record:** Cloudflare R2 bucket `deepscry-decks`. Each user has
  ONE object: `decks/<identity>/collection.tgz` — a gzipped tar of plaintext
  `<deck name>.dck` files.
- **The server never proxies deck bytes.** It holds ONE long-lived "parent"
  R2 API token (from env) and, on `GET /api/deck-storage/credentials`, mints
  **short-TTL (10 min), prefix-scoped presigned URLs** (PUT / GET / HEAD +
  an attachment-download GET). The browser uses those URLs to talk to R2
  directly. (`mtg-engine/src/web_server/r2.rs`, `web/deck_storage.js`.)
- **Identity is a swappable seam.** `r2::Identity` is a trait; today the
  `DevIdentity` stub returns a fixed `dev` prefix. When OAuth lands, a real
  identity drops in WITHOUT touching the storage path.
- **Cross-device safety:** writes use an `If-Match` conditional PUT keyed on
  the object's ETag. If another device wrote first, R2 returns 412 and the
  client surfaces a "reload to merge" hint instead of clobbering.
- **Debounced saves:** ≤1 R2 PUT/sec to the same key (R2's same-key write
  limit).
- **IndexedDB** is an offline read/edit cache, NOT the source of truth.
- **Opaque bytes:** uploaded `Content-Type: application/gzip`, NO
  `Content-Encoding`, so R2 stores byte-clean (no CDN re-compression).

## Configuration (env vars)

The server reads these at startup (mirrors how `.r2.env` is sourced):

```
AWS_ACCESS_KEY_ID
AWS_SECRET_ACCESS_KEY
R2_ENDPOINT           # https://<account-id>.r2.cloudflarestorage.com
R2_BUCKET             # deepscry-decks
R2_PUBLIC_BASE_URL    # optional — see "Public bucket" below
```

The first four must be present or the endpoint returns 503 (the rest of the
server is unaffected). `deploy-cloud.sh` forwards them into the systemd
`EnvironmentFile`; see `scripts/deepscry-deploy.env.example`.

## Public bucket (optional — enables permanent "Direct link" URLs)

By default R2 objects are private (access only via presigned URLs that expire
after ~10 minutes). Setting `R2_PUBLIC_BASE_URL` enables **permanent, stable
Direct link URLs** — no expiry, no re-mint needed.

**What it does:**
- The credentials endpoint includes a `public_url` field, e.g.
  `https://pub-<hash>.r2.dev/decks/<user-id>/collection.tgz`
- The deck editor's "Direct link" button uses this URL instead of the expiring
  presigned URL when it is present
- The link is stable as long as the user's identity doesn't change — the URL
  can be bookmarked or shared

**Required operator step (one-time, out-of-band):**
1. In the Cloudflare dashboard: R2 → `deepscry-decks` → Settings →
   "Public access" → click "Allow Access"
2. Note the `pub-<hash>.r2.dev` subdomain shown
3. Add to `.r2.env`: `R2_PUBLIC_BASE_URL=https://pub-<hash>.r2.dev`
4. Re-deploy: `./scripts/deploy-cloud.sh deploy`

**Security note:** deck collections contain only MTG deck lists (card names +
counts) — no personal data, no secrets. Making the bucket public means any
user's collection URL is world-readable if discovered, but there is nothing
sensitive to expose. This is why the feature is opt-in (requires the operator
to enable it) rather than the default.

## Feature flag (client)

The cloud path is ADDITIVE and OFF by default — the existing localStorage
flow is untouched. Enable it per-browser:

```js
localStorage['mtg-deck-cloud'] = '1'   // then reload deck_editor.html
```

When on, the deck editor reveals "Download my decks", runs a one-time
additive migration of existing localStorage decks into R2, and mirrors
subsequent saves to the cloud.

## CORS (MANUAL STEP — parent token cannot set it)

The browser talks to R2 cross-origin, so the bucket needs CORS allowing the
`deepscry.net` origin to PUT/GET/HEAD and exposing the `ETag` header (the
If-Match flow depends on reading ETag from JS).

The live parent R2 token has object access but NOT bucket-config admin —
`PutBucketCors`/`GetBucketCors` both return `403 AccessDenied`. So CORS must
be applied by the bucket owner via one of:

**Option A — Cloudflare dashboard:** R2 → `deepscry-decks` → Settings → CORS
Policy → paste the JSON in `scripts/r2-cors.json`.

**Option B — `wrangler` / S3 API with an admin token:** apply
`scripts/r2-cors.json` (S3 `PutBucketCors`) with a token that has bucket
admin scope.

The required policy (`scripts/r2-cors.json`):

```json
[
  {
    "AllowedOrigins": ["https://deepscry.net", "https://www.deepscry.net"],
    "AllowedMethods": ["GET", "PUT", "HEAD"],
    "AllowedHeaders": ["*"],
    "ExposeHeaders": ["ETag"],
    "MaxAgeSeconds": 3600
  }
]
```

## Verified end-to-end

A live round-trip against the real bucket (PUT → GET bytes match → stale
If-Match PUT → 412) is implemented as an `#[ignore]`d test in
`r2.rs::tests::live_round_trip`, run manually with creds in env (NOT in
`make validate`, which must stay hermetic). The hermetic browser e2e
(`web/test_deck_storage.js`, wired into validate) exercises the same
pack→PUT→GET→unpack + If-Match-conflict + additive-migration paths against a
mocked R2.

## Still blocked on the OAuth app

- Real per-user identity (replace `DevIdentity` with an OAuth-backed
  `Identity`). Today every caller shares the `decks/dev/` prefix.
- The login UI + token verification leg.
- CORS application by the bucket owner (above) — independent of OAuth but
  also needs the owner's hands.
