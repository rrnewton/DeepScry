---
name: deploy-release
description: The release ceremony for DeepScry — promote integration→main, then deploy the CAS web build from main to the live VM with pre/post smoke gates. Use whenever shipping a new version to deepscry.net. Covers the integration→main promotion (ff-only), the deploy-from-main rule, the hermetic pre-deploy CAS gate, the post-deploy probe, live verification, and restoring the working baseline. The promote/checkout logic lives HERE (workflow layer), NOT in deploy-cloud.sh (which is a dumb copier of the local checkout).
---

# Release / Deploy Ceremony — DeepScry

This skill is the **deploy-from-main** workflow (mtg-590). The actual copying
script `scripts/deploy-cloud.sh` is intentionally "dumb": it builds + content-
addresses + rsyncs **whatever the local checkout currently is**, plus runs its
own hermetic pre-deploy gate and post-deploy probe. It does NOT know about
branches. The branch ceremony (promote → checkout main → deploy from main)
lives here, in the workflow, so the script stays a simple copier.

Run from the **primary checkout** (`parent/mtg-forge-rs/`), as the coordinator.

## Preconditions

1. `integration` is **green on CI** (`gh -R DeepScryAI/DeepScry run list --branch
   integration --limit 1` → `completed success`). NEVER promote a red/in-progress
   integration to main.
2. Primary checkout `git status` is clean.
3. `<parent>/.deepscry-deploy.env` exists (REMOTE_USER/REMOTE_HOST/etc.). The VM
   is already bootstrapped (`deploy-cloud.sh config` was run once).

## Login + cloud-deck secrets (GitHub/Google OAuth + R2) — mtg-742

The login and cloud-deck-storage SECRETS live in their own gitignored files in
the dev-harness parent dir, kept SEPARATE from `.deepscry-deploy.env`:

- `<parent>/.oauth.env` — `GITHUB_OAUTH_CLIENT_ID/SECRET`,
  `GOOGLE_OAUTH_CLIENT_ID/SECRET`, `OAUTH_CALLBACK_BASE`.
- `<parent>/.r2.env` — `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
  `R2_ENDPOINT`, `R2_BUCKET`.

`deploy-cloud.sh` SOURCES both files (fail-soft) via `load_deploy_secrets` and
`render_env_file()` forwards whatever they set into the production systemd
EnvironmentFile (`~/.config/<svc>/deploy.env`) on **both `config` AND `deploy`**.
A missing/empty file simply leaves that feature disabled — it never breaks the
deploy. Only the vars that are actually set are emitted (per-var fail-soft).

**One-step enable flow:** populate the secret file(s), then run a normal
`scripts/deploy-cloud.sh deploy`. The deploy regenerates + pushes the env file
and restarts the service, so the secrets go live with the SAME single operation
used for code (no separate `config` run required). Verify after with
`curl -sk https://<host>/auth/status` (advertises which providers are
configured) — see the live-verification section.

**Out-of-band operator steps (one-time, NOT done by the deploy):**
- Register the OAuth callback URLs at each provider:
  `https://deepscry.net/auth/callback/github` and `.../google` (must match
  `OAUTH_CALLBACK_BASE`).
- Apply the R2 bucket CORS policy from `scripts/r2-cors.json` (Cloudflare
  dashboard / admin token); steps in `ai_docs/reference/R2_DECK_STORAGE.md`.
- Rotate the OAuth client secrets + the R2 token before public launch.

Secret values NEVER enter the repo; the tracked template
`scripts/deepscry-deploy.env.example` documents the file layout only.

## (Optional but recommended) CAS determinism confidence check

Content-addressed set-bins must be byte-stable across rebuilds (only changed
card data → changed hash). To confirm before a release, export twice and diff:

```sh
ls web/data/sets/*.bin | xargs -n1 basename | sort > /tmp/a
cargo run --release --bin mtg -- export-wasm >/dev/null
ls web/data/sets/*.bin | xargs -n1 basename | sort > /tmp/b
diff /tmp/a /tmp/b   # EXPECT empty: identical hashed filenames = deterministic
```

## Ceremony

```sh
cd <parent>/mtg-forge-rs
git fetch origin

# 1. Promote integration -> main (ff-only; main is protected; NEVER force-push).
git checkout main
git merge --ff-only origin/integration
git push origin main           # sanctioned promotion push (not a dev commit)

# 2. Deploy FROM main. deploy-cloud.sh copies the CURRENT checkout, so we must
#    be on main. It ALWAYS rebuilds binary+wasm now (no stale-artefact reuse;
#    --skip-build/--skip-wasm are explicit opt-outs only).
git checkout main
scripts/deploy-cloud.sh deploy
#   This runs, in order:
#     - rebuild release-deploy mtg binary (--features network) + WASM/data
#     - PRE-DEPLOY GATE: hermetic web-asset smoke (mtg server-web on a temp
#       port; asserts hashed bin/wasm/js + hashed index.<h>.json + manifest
#       immutable, logical→hashed resolution, and EVERY fixed name — incl.
#       /data/sets/index.json — 404s on the hashed tree, i.e. index.html is
#       the sole mutable file — mtg-727). ABORTS before any rsync if it fails.
#     - PRE-DEPLOY GATE: headless WASM-boot smoke (deserialize tokens+decks+
#       sets against the fresh glue, launch a game). ABORTS on any deserialize
#       error. (Chromium-gated: SKIPS with a loud warning if Playwright absent.)
#     - PRE-DEPLOY GATE: networked-game desync smoke (mtg-703). Plays ONE
#       short, fully-deterministic NETWORK game — native `mtg server` + two
#       `mtg connect` clients, avatar-draft mirror, seed=3, zero controllers —
#       reusing tests/network_vs_local_equivalence_e2e.sh, and asserts
#       local↔server gamelogs are byte-identical AND the perspective-aware
#       server↔client public-zone oracle finds ZERO divergence. A desyncing
#       build ABORTS here, BEFORE any rsync. This is the leg that catches a
#       network-desync regression the web-asset/WASM-boot gates cannot see
#       (the failure mode behind 7b235b32 shipping despite an open ~30%
#       desync). NATIVE-only (no Node/WASM/Chromium) so it is NOT env-gated
#       and cannot be silently skipped. Override the seed for debugging with
#       DEPLOY_NET_SMOKE_SEED=<n>.
#     - stage web/ + `mtg hash-web-assets` (content-address the pkg pair +
#       rewrite HTML import specifiers), rsync web/ + cardsfolder + binary
#     - restart the systemd (user) service
#     - POST-DEPLOY PROBE against the live origin (/, hashed js/wasm,
#       hashed index.<h>.json + its cache-control=immutable, fixed
#       /data/sets/index.json must 404 (mtg-727: only the hashed name
#       resolves), hashed bin, /health sha, /lobby)

# 3. Restore the working baseline so coordination/worktrees branch off integration.
git checkout integration
```

## Live verification (do NOT trust the script's exit code alone)

```sh
curl -sk https://<host>:<port>/health                       # sha == the deployed SHA
# The data set-index is CONTENT-ADDRESSED on a clean deploy → the FIXED name
# 404s; its hashed form is immutable. (Discover the hashed name from a game page.)
curl -sk -o /dev/null -w '%{http_code}\n' https://<host>:<port>/data/sets/index.json  # 404 (renamed to hashed)
curl -skI https://<host>:<port>/data/sets/index.<hash>.json | grep -i cache-control   # immutable, max-age=31536000
curl -skI https://<host>:<port>/pkg/mtg_engine_bg.<hash>.wasm | grep -i cache-control  # immutable, max-age=31536000
curl -sk -o /dev/null -w '%{http_code}\n' https://<host>:<port>/   # 200
# Login + cloud-deck secrets wired in (mtg-742): confirm the providers the
# server now sees as configured (reflects .oauth.env forwarded into the unit).
curl -sk https://<host>:<port>/auth/status            # lists configured OAuth providers
```

## Caching model (mtg-620 full-graph hashing + mtg-704 pure-DAG)

`index.html` is the **SOLE** stable/unhashed URL. EVERY other reachable asset
is content-addressed (`<stem>.<16hex>.<ext>`) and immutable.

- **`index.html`** — stable URL, served **short-TTL** (`public, max-age=60`).
  A stale cached copy is recoverable: it carries a release token, and the CAS
  dispatcher (`index.html?goto=…`) falls back to the *latest* manifest if the
  baked release is gone, so a ≤60 s-stale entry never hard-404s.
- **Everything else is hashed + immutable** (`max-age=31536000, immutable`):
  the game pages (`native_game.<hash>.html`, `tui_game.<hash>.html`,
  `launcher.<hash>.html`, …), the JS leaves (`server-config.<hash>.js`, …),
  the pkg pair (`mtg_engine.<hash>.js` / `mtg_engine_bg.<hash>.wasm`), the
  per-set `<YYYY>-<CODE>.<hash>.bin`, AND the **data set-index**
  (`index.<hash>.json`). Cache-busting is via the hashed FILENAME. On a clean
  deploy the corresponding FIXED names (`/data/sets/index.json`,
  `/pkg/mtg_engine.js`, `/native_game.html`, …) **404** — they were renamed by
  `mtg hash-web-assets`.
- **No special-cased resolver (mtg-727).** The data set-index is NOT a
  second mutable/no-cache file: it is folded into the CAS graph exactly like
  the bins/wasm/js. Its content lists the hashed `.bin` names, so its own hash
  transitively covers them — it is a Merkle parent rolled up by the release
  token. Therefore the FIXED `/data/sets/index.json` **404s on a clean deploy**
  (only `index.<hash>.json` resolves, immutable), and `index.html` is the
  GENUINE sole mutable/no-cache file. The mtg-727 live symptom (fixed
  `/data/sets/index.json` served `200, max-age=60`) was a STALE/incomplete
  build (an old binary predating mtg-620's full-graph hashing). Guardrails so
  it can't recur silently: (a) the post-deploy probe asserts the fixed
  `/data/sets/index.json` returns **404** and the hashed `index.<h>.json` is
  immutable; (b) `web/test_web_server_smoke.js` (in `make validate`) asserts
  the fixed name 404s on a staged hashed tree.
- Net effect across redeploys: unchanged card data → unchanged set-bin hashes →
  stay cached; only changed code (wasm/js) and the short-TTL `index.html`
  re-download.

> **Open follow-up (mtg-705):** `index.html` is the sole mutable file at the
> *file* level, but it still inlines the splash/lobby/login PROSE. Splitting
> that into a hashed child page (`index.html` → pure release dispatcher) is
> tracked there, along with confirming the data-index is never fetched by a
> fixed name anywhere.

## Notes / gotchas

- `deploy-cloud.sh` must stay a dumb copier. Do NOT add main-branch assertions
  to it; the branch ceremony belongs here.
- A pre-deploy gate failure is a SUCCESS of the safety system — read the error,
  fix, redeploy; nothing was shipped.
- `--force` to `main` is never allowed without explicit user approval.
