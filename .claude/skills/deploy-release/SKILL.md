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

1. `integration` is **green on CI** (`gh -R rrnewton/DeepScry run list --branch
   integration --limit 1` → `completed success`). NEVER promote a red/in-progress
   integration to main.
2. Primary checkout `git status` is clean.
3. `<parent>/.deepscry-deploy.env` exists (REMOTE_USER/REMOTE_HOST/etc.). The VM
   is already bootstrapped (`deploy-cloud.sh config` was run once).

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
#       port; asserts index.json no-cache, hashed bin/wasm/js immutable,
#       logical→hashed resolution, fixed-name pkg 404 on hashed tree). ABORTS
#       before any rsync if it fails — nothing ships on a broken pipeline.
#     - stage web/ + `mtg hash-web-assets` (content-address the pkg pair +
#       rewrite HTML import specifiers), rsync web/ + cardsfolder + binary
#     - restart the systemd (user) service
#     - POST-DEPLOY PROBE against the live origin (/, hashed js/wasm,
#       index.json, hashed bin, /health sha, /lobby)

# 3. Restore the working baseline so coordination/worktrees branch off integration.
git checkout integration
```

## Live verification (do NOT trust the script's exit code alone)

```sh
curl -sk https://<host>:<port>/health                       # sha == the deployed SHA
curl -skI https://<host>:<port>/data/sets/index.json | grep -i cache-control   # no-cache, must-revalidate
curl -skI https://<host>:<port>/pkg/mtg_engine_bg.<hash>.wasm | grep -i cache-control  # immutable, max-age=31536000
curl -sk -o /dev/null -w '%{http_code}\n' https://<host>:<port>/   # 200
```

## Caching model (why HTML is NOT hashed)

- **HTML pages** (`index.html`, `native_game.html`, `tui_game.html`, …) keep
  **stable URLs** (they are entry points / link targets) and are served
  **short-TTL** (`public, max-age=60`). Their *content* changes each deploy
  because the embedded pkg import specifiers point to the new hashed names.
- **`.wasm` / `.js` / per-set `.bin`** are **content-addressed + immutable**
  (`max-age=31536000`). Cache-busting is via the hashed FILENAME, never by
  renaming the HTML.
- **`index.json`** (the logical→hashed resolver) is **no-cache**.
- Net effect across redeploys: unchanged card data → unchanged set-bin hashes →
  stay cached; only changed code (wasm/js) and the short-TTL HTML re-download.

## Notes / gotchas

- `deploy-cloud.sh` must stay a dumb copier. Do NOT add main-branch assertions
  to it; the branch ceremony belongs here.
- A pre-deploy gate failure is a SUCCESS of the safety system — read the error,
  fix, redeploy; nothing was shipped.
- `--force` to `main` is never allowed without explicit user approval.
