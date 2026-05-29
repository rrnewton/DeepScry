# Content-Addressed Immutable Web-Asset Pipeline — Design (mtg-571)

Transient-info stamp: `2026-05-28_#2411(trunk-cas-assets)` (shell-script-free + smoke-test maturation)

Status: **DESIGN + PROTOTYPE.** This document is the concrete design that
follows the options survey in
[`CONTENT_ADDRESSED_ASSETS_RESEARCH_20260528.md`](./CONTENT_ADDRESSED_ASSETS_RESEARCH_20260528.md).
The research doc chose **Option 2 (custom Rust post-build hasher)** over
Trunk / JS-bundler / service-worker. This doc specifies the chosen scheme,
the GC, the invariants, and records what the prototype on branch
`trunk-cas-assets` actually implements vs. what remains deferred.

It is **speculative / research-first** and **NOT merge-ready** — it is
intended for the user's PR review.

> **UPDATE (hash unification, mtg-571):** the hash-function discrepancy
> flagged below has been **RESOLVED**. Both the per-set bins and the wasm
> pkg pair now hash through a single shared Rust function
> (`mtg_forge_rs::asset_hash::asset_hash_hex` — **blake3**, truncated to 16
> hex chars). The exporter calls it directly; the pkg pair is hashed by the
> new `mtg hash-web-assets <web_dir>` subcommand (no more `sha256sum | cut`).
> The SipHash (`DefaultHasher`) path in the exporter is gone. §3 and the
> §9 open question #1 below are kept for historical context but are now
> marked resolved.
>
> **UPDATE (shell-script-free + smoke test, mtg-571, this session):** the
> deploy-staging shell script `scripts/hash_web_assets.sh` has been **DELETED**
> and fully folded into Rust as `mtg hash-web-assets` (hashes the pkg pair,
> renames it, and structurally rewrites the HTML import specifier +
> `init({module_or_path})`). `deploy-cloud.sh` calls the Rust subcommand. The
> web_server now serves a *content-addressed* `/pkg/<stem>.<hash>.<ext>` as
> `immutable` and a *fixed-name* `/pkg/<stem>.<ext>` as `no-cache` (was always
> no-cache) — closing the split-brain so the deployed hashed pkg pair is
> finally immutable-eligible. A NEW hermetic local smoke test
> (`web/test_web_server_smoke.js`) launches `mtg server-web` on a temp port and
> asserts the cache tiers + logical→hashed resolution; it is the deploy
> PRE-rsync gate AND a `make validate` step. See §3.1, §4, §6, §10.

---

## 1. Goal recap

Make the stale-bundle bug class (mtg-475 / mtg-2indh: a cached JS glue
paired with a fresh `.wasm`) **structurally impossible**, and let every
immutable asset be served `Cache-Control: public, max-age=31536000,
immutable` — safe even behind a Cloudflare Cache Rule that overrides
origin headers, because the only way to get new bytes is a new URL.

User's stated ideal: **`index.html` is the sole mutable pointer; every
other shipped asset is content-addressed and immutable.**

---

## 2. The chosen scheme

### 2.1 Naming convention

A content-addressed asset embeds a hash of its own bytes in its filename:

```
<logical-name>.<hash>.<ext>
```

| Asset class      | Source name                | Content-addressed name             | Who names it           |
|------------------|----------------------------|------------------------------------|------------------------|
| Per-set data bin | `<YYYY>-<CODE>.bin`        | `<YYYY>-<CODE>.<hash>.bin`         | exporter (Rust)        |
| wasm-bindgen JS  | `mtg_forge_rs.js`          | `mtg_forge_rs.<hash>.js`          | `mtg hash-web-assets`  |
| wasm-bindgen WASM| `mtg_forge_rs_bg.wasm`     | `mtg_forge_rs_bg.<hash>.wasm`     | `mtg hash-web-assets`  |

`<hash>` is the first 16 hex chars (64 bits) of the **blake3** digest of
the bytes, computed by the single shared function
`mtg_forge_rs::asset_hash::asset_hash_hex` (see §3). Collision probability
for a few hundred assets is negligible (birthday bound: ~2^-44 for 600
assets).

### 2.2 The manifest = the resolver (logical → hashed)

There is no new manifest file. The resolver for the bins is the existing
`data/sets/index.json`, emitted by the exporter (`SetIndex { version,
sets, cards }`). Each set entry already carries `{ file, bytes,
card_count }`; the prototype adds a `hash` field and puts the
content-addressed name into `file` (and the per-card `cards` map):

```json
{
  "version": "...",
  "sets": [
    { "file": "1994-LEG.3fa1c0de9b2e4d51.bin",
      "bytes": 1064943, "card_count": 310,
      "hash": "3fa1c0de9b2e4d51" }
  ],
  "cards": { "Black Lotus": "1993-LEA.<hash>.bin", ... }
}
```

**Key property — zero client change for bins.** Every game page already
does `fetch(`./data/sets/${s.file}`)` (verified: `tui_game.html:1347`,
`native_game.html:1309`, `demo.html:292`, `wasm_ai_harness.html:95`). The
client only ever fetches a name it read out of `index.json`, so hashing
the bin names is invisible to the client. `index.json` is the **single
mutable pointer** for the bin layer.

For the **pkg pair**, the resolver is the HTML page itself: the ES import
specifier (`import init, {…} from './pkg/mtg_forge_rs.js'`) and the
`init()` call. `mtg hash-web-assets` rewrites both injection points to the
hashed names on a deploy-staging copy (see §4).

### 2.3 The immutability invariant (the load-bearing rule)

> A route may be served `immutable` **only if** its URL is
> content-addressed (the bytes uniquely determine the filename).

This is documented inline in `web_server/mod.rs`. It is the rule that
keeps the cache tiers honest: adding an `immutable` tier for a fixed-name
asset re-opens the desync bug. Consequences for the current tree:

| Route                    | Content-addressed? | Cache tier                                  |
|--------------------------|--------------------|---------------------------------------------|
| `/data/**/*.bin` (sets)  | YES (exporter)     | `public, max-age=31536000, immutable`       |
| `/images/**`             | YES (scryfall id)  | `public, max-age=31536000, immutable`       |
| `/pkg/<stem>.<hash>.ext` | YES (staging hash) | `public, max-age=31536000, immutable`       |
| `/pkg/<stem>.ext` (fixed)| NO (fixed name)    | `no-cache, must-revalidate`                 |
| `/data/sets/index.json`  | NO (mutable ptr)   | `no-cache, must-revalidate`                 |
| `/data/decks.bin`        | NO (fixed name)    | `no-cache, must-revalidate` (dedicated rt)  |
| `/data/tokens.bin`       | NO (fixed name)    | `no-cache, must-revalidate` (dedicated rt)  |
| HTML, `server-config.js` | NO (mutable ptr)   | `public, max-age=60`                        |

The previous split-brain on `/pkg/*` (always-no-cache, hashed only on the
staging copy) is **resolved**: `web_server::pkg_cache_header` inspects the
request filename and serves a content-addressed `<stem>.<hash>.<ext>` as
`immutable` while keeping a fixed `<stem>.<ext>` (the committed source-tree
import target used by `make validate` e2e) on `no-cache`. One binary serves
both the source tree and a hashed deploy tree correctly. The `?v=<sha>`
cache-bust is fully retired.

---

## 3. Hash function — UNIFIED on blake3 (RESOLVED, mtg-571)

The research doc recommended **blake3 truncated to 16 hex** for both
layers. The original prototype diverged (SipHash for bins, truncated
SHA-256 for the pkg pair) — a DRY/consistency smell. **That divergence is
now fixed.** The chosen end-state is the research doc's recommendation,
option (a): a single shared Rust function used by both layers.

| Layer            | Hash (current)                               | Width |
|------------------|----------------------------------------------|-------|
| Per-set bins     | blake3 (`asset_hash_hex`)                    | 64b   |
| pkg JS+WASM      | blake3 (`asset_hash_hex` via `mtg hash-web-assets`) | 64b |

**Single source of truth:** `mtg_forge_rs::asset_hash::asset_hash_hex`
(`mtg-engine/src/asset_hash.rs`) is the ONE function that names every
content-addressed asset. It computes `blake3::hash(bytes)` and truncates
to `ASSET_HASH_HEX_LEN = 16` hex chars.

- The exporter (`main.rs`, `run_export_wasm`) calls `asset_hash_hex`
  directly for the per-set bins — the old `content_hash_hex`
  (`DefaultHasher`/SipHash) helper is **deleted**.
- The pkg pair is hashed + renamed + HTML-rewritten by
  `asset_hash::web_pkg::hash_web_assets`, exposed as the
  `mtg hash-web-assets <web_dir>` subcommand. It calls `asset_hash_hex`
  directly — no shell, no `sha256sum`, no second hash implementation
  anywhere. (`mtg hash-asset <file>` remains a thin one-off CLI wrapper
  over the same function for ad-hoc scripting.)

### 3.1 Shell-script-free (mtg-571, this session)

`scripts/hash_web_assets.sh` is **deleted**. All pkg hashing + renaming +
HTML rewriting now lives in Rust (`asset_hash::web_pkg`), unit-tested in
isolation: `rewrite_html` has 8 tests covering static/dynamic import
specifiers (dot- and slash-form), `await init()` / `init()`, idempotency,
and the no-clobber guards. The structured-rewrite property (only the two
controlled injection points are touched) is preserved exactly as the shell
version did, but now type-checked and DRY with the exporter.

**Why blake3 over the previous options.** blake3 is fast, is a single
small dependency, and (unlike `std`'s `DefaultHasher`) has **no per-process
seed and is stable across machines / Rust versions** — so identical bytes
always produce the identical content-addressed filename, which is exactly
what a content-addressed scheme wants. This closes the
SipHash-cross-version-stability worry (former §9 open question #2): blake3
is reproducible by construction.

Verified: `mtg hash-asset` of the exported `.js`/`.wasm` files produces
the same hash that `hash_web_assets.sh` embeds in the rewritten HTML, and
a re-`hash-asset` of any exported `<set>.<hash>.bin` reproduces the hash
embedded in its filename and in `index.json`'s `hash` field. Same input →
same hash; bins and pkg use the identical function.

---

## 4. The wasm-bindgen JS↔WASM crux (resolved)

The generated glue defaults `module_or_path` to
`new URL('mtg_forge_rs_bg.wasm', import.meta.url)`, so a naive rename of
the `.wasm` would break the glue's self-reference. Resolution (verified in
the research doc, implemented in `hash_web_assets.sh`): the pages call
`init()` and the script rewrites it to

```js
await init({ module_or_path: './pkg/mtg_forge_rs_bg.<hash>.wasm' })
```

— wasm-bindgen's documented override — so **the generated glue is never
edited**. The two rewrites are confined to controlled injection points (the
ES import specifier + the `init()` arg), consistent with the project's "No
Hacky String Operations On Structured Data" rule. `mtg hash-web-assets`
operates **in place on a staging copy only** (never the source tree).

---

## 5. Garbage collection of orphaned hashed blobs

Content addressing accumulates blobs: every content change of a set yields
a new `<set>.<newhash>.bin`, and the old `<set>.<oldhash>.bin` becomes an
orphan. The GC is a **manifest-driven mark-sweep at deploy time**, in
`deploy-cloud.sh`:

1. **Mark.** Parse the staging `data/sets/index.json`; the set of live bin
   names is `{ s["file"] for s in idx["sets"] }`.
2. **Sweep.** For every `*.bin` in the staging `data/sets/` directory not
   in the live set, delete it from the staging copy.
3. **Propagate.** `rsync --delete` then prunes the orphans from the VM
   (and never re-uploads them), and also prunes any old hashed pkg name
   the new HTML no longer references.

`index.json` is the authoritative manifest of live bin names, so the GC
needs no separate refcount DB. The pkg pair is GC'd implicitly: the
staging copy contains only the freshly-hashed pair, and `rsync --delete`
removes the previous generation's hashed pkg files.

**GC design notes / open issues:**

- **No grace window.** `rsync --delete` removes the previous generation
  immediately. A client mid-load across a redeploy (holding old
  `index.html` that references an old hashed bin) would 404 on a
  just-deleted blob. For a single-origin hobby deploy this is acceptable
  (rare, self-heals on reload). A future "keep last N generations" or
  time-based grace window is a possible enhancement — filed as an open
  question, not implemented.
- **GC is deploy-side only.** There is no GC of the local source
  `web/data/sets/` between exports — a developer who re-exports
  repeatedly accumulates orphan bins locally. The deploy GC sweeps the
  *staging copy*, so they never reach the VM, but the local tree grows.
  These bins are gitignored (see §7), so it is a disk-hygiene concern, not
  a repo concern. A `mtg export-wasm --prune` or a make target could clean
  the local tree — deferred.

---

## 6. Deploy + the `?v=<sha>` scheme

**Before (status quo):** `deploy-cloud.sh` injected a `?v=<git-sha>`
query string onto asset URLs as a cache-bust, and `/pkg` + `/data` were
served `no-cache`.

**After (this design):**

- The `?v=<sha>` query-string cache-bust is **fully retired**. Pkg
  freshness now comes from the staging rewrite (`mtg hash-web-assets`) +
  the manifest GC sweep + `rsync --delete`; fixed-name source-tree pkg
  files are served `no-cache` by `web_server::pkg_cache_header`, so there
  is no residual reliance on a query string. (No `?v=` references remain in
  any HTML or script.)
- A **PRE-rsync local smoke-test gate** runs `web/test_web_server_smoke.js`
  against a temp `mtg server-web` before anything touches the VM; a broken
  pipeline aborts the deploy locally (see §10).
- `build.rs` still emits `MTG_BUILD_SHA` (used by `/health` and the
  residual `?v=`); no change needed there.
- The post-deploy probe was updated to *derive* the hashed pkg names from
  the deployed `tui_game.html` import specifier (rather than probing a
  fixed name), which also implicitly verifies the HTML rewrite landed.

### Why staging-copy rewrite instead of full Trunk adoption

Trunk's `rel="rust"` model owns the cargo+wasm-bindgen build and replaces
a `<link>` with its own injected bootstrap exposing the module only as
`window.wasmBindings`. Our four game pages instead statically import 20+
named exports from `./pkg/mtg_forge_rs.js`. Trunk's bootstrap cannot serve
hand-authored named-export static imports of a hashed filename, and Trunk
does **not** hash our 315 `.bin` files (they'd be `copy-dir`, un-hashed).
Migrating all four pages to `window.wasmBindings.*` is a large, risky
rewrite. So:

- `Trunk.toml` is committed as the **declared** build tool with a
  documented migration path, but trunk does NOT yet own the build.
- The content-addressing that actually kills the stale-bundle bug is
  delivered NOW via the exporter (bins) + `hash_web_assets.sh` (pkg).
- Full trunk adoption is deferred (mtg-dxig9).

---

## 7. Git hygiene — references, not blobs

The pipeline tracks **references/manifests**, never the generated blobs:

- `web/.gitignore` already ignores generated `pkg/` and `data/` outputs;
  the prototype adds `dist/` (trunk output dir).
- Hashed bins, hashed pkg files, and the staging copy are all generated /
  gitignored — none are committed.
- What IS committed: the exporter code, `hash_web_assets.sh`,
  `deploy-cloud.sh`, the web_server cache tiers, `Trunk.toml`, and this
  design doc.

This satisfies the project's "NEVER commit images/binaries" rule: the
asset pipeline is about the naming scheme + manifest, not the bytes.

---

## 8. What the prototype implements vs. what is deferred

### Implemented on `trunk-cas-assets` (commit `c213c485`, rebased onto integration)

1. **Bins content-addressed** (`mtg-engine/src/main.rs`): `<set>.<hash>.bin`
   names in `index.json` `file`/`cards` + a `hash` field. Verified in the
   prior session: 315 hashed bins, manifest references match.
2. **pkg pair content-addressed on the staging copy — now in RUST**
   (`asset_hash::web_pkg::hash_web_assets`, exposed as
   `mtg hash-web-assets`): structured rewrite of the import specifier +
   `init({module_or_path})`. `scripts/hash_web_assets.sh` is DELETED. 8
   unit tests on `rewrite_html`; verified live by the smoke test.
3. **Cache tiers** (`mtg-engine/src/web_server/mod.rs`): hashed
   `/data/**/*.bin` → immutable 1y; hashed `/pkg/<stem>.<hash>.<ext>` →
   immutable 1y, fixed `/pkg/<stem>.<ext>` → no-cache (content-aware
   `pkg_cache_header`); `decks.bin`/`tokens.bin` dedicated no-cache routes;
   immutability INVARIANT documented inline.
4. **Deploy GC mark-sweep + rsync --delete** (`scripts/deploy-cloud.sh`):
   replaces `?v=`; prunes orphaned hashed bins; PRE-rsync local smoke-test
   gate; post-deploy probe derives hashed pkg names from the deployed HTML.
5. **Hermetic local smoke test** (`web/test_web_server_smoke.js`): launches
   `mtg server-web` on a temp port against a hashed staging tree and asserts
   the cache tiers + logical→hashed resolution. Wired into BOTH the deploy
   gate and `make validate` (`validate-network-e2e-step`).
6. **`Trunk.toml`** declared with documented staged-migration path.

### Deferred (filed follow-ups)

- **mtg-dxig9** — full trunk `rel="rust"` migration of the source HTML
  (so the committed HTML ships hashed pkg directly). NOTE: this is now an
  *optimization*, not a correctness requirement — `hash_web_assets.sh` and
  the `?v=` cache-bust are ALREADY retired (the staging-copy `mtg
  hash-web-assets` + content-aware `pkg_cache_header` cover correctness).
- **mtg-ntx2j** — content-address `decks.bin` / `tokens.bin` so they can
  join the immutable tier instead of no-cache.
- **mtg-1rvug** — hash the game-page HTML (`tui_game.<hash>.html` etc.) so
  `index.html` is the ONLY mutable HTML pointer (the user's stated ideal).

---

## 9. Open questions for PR review

1. **Hash function unification (§3).** ✅ **RESOLVED** — adopted blake3 in
   Rust for BOTH bins and pkg via the shared
   `mtg_forge_rs::asset_hash::asset_hash_hex` + the `mtg hash-asset`
   subcommand. Killed the shell `sha256sum` and the exporter's SipHash
   path. Matches the research doc recommendation.
2. **SipHash cross-version stability (§3).** ✅ **RESOLVED** — blake3 has
   no per-process seed and is stable across Rust versions / machines, so
   bin (and pkg) names are reproducible across builds by construction.
3. **GC grace window (§5).** Keep last-N generations of hashed blobs on
   the VM for clients mid-load across a redeploy, or accept the immediate
   `--delete` (rare 404, self-heals on reload)?
4. **Local-tree GC (§5).** Add `mtg export-wasm --prune` / a make target
   to sweep orphan bins from the local `web/data/sets/` between exports?
5. **Staging-copy vs. source-tree pkg hashing (§6).** ✅ **RESOLVED for
   correctness** — `pkg_cache_header` is content-aware, so the source tree's
   fixed pkg name is correctly `no-cache` and the deploy tree's hashed name
   is correctly `immutable`, from ONE binary. Migrating the *source* HTML to
   ship hashed pkg directly (mtg-dxig9) is now a nice-to-have, not required.
6. **`index.html` mutable pointer.** Confirm `index.html` +
   `server-config.js` + `index.json` are the intended permanent mutable
   tier (everything else immutable). mtg-1rvug would make `index.html`
   the *sole* mutable HTML.

---

## 10. Validation status

**This session (mtg-571 maturation):**

- `cargo fmt --all -- --check` — clean.
- `cargo clippy -p mtg-forge-rs --all-targets --all-features --features
  network -- -D warnings` — clean (matches CI).
- `cargo test --lib asset_hash` — 12 passed (4 hash + 8 `rewrite_html`).
- **Hermetic pre-deploy smoke test** (`web/test_web_server_smoke.js`) RUN
  and **PASS**: launched `mtg server-web` on a temp localhost port against a
  hashed staging tree; asserted landing 200, `/data/sets/index.json` 200 +
  no-cache, a hashed per-set bin 200 + immutable, the hashed wasm 200 +
  immutable, the hashed JS glue 200 + immutable, the fixed pkg name 404 on
  the hashed tree, and (against the source tree) the fixed pkg name 200 +
  no-cache. No orphaned `mtg server-web` processes left behind.
- Full `make validate` — see the cited `validate_logs/validate_<sha>.log`
  in the commit / PR (run on this branch; CPU-courtesy serialized behind
  the sibling agents).

### Rename impact (mtg-forge-rs → mtg-engine)

This branch deliberately PREDATES the crate rename and keeps `mtg_forge_rs`
naming (it will be rebased onto post-rename integration later). When the
rename lands, the wasm-pack output base name changes from `mtg_forge_rs` to
`mtg_engine`, so these references update in lockstep:

- `mtg-engine/src/asset_hash.rs`: `PKG_JS_STEM` / `PKG_WASM_STEM` consts
  (single point of truth for the pkg base names — the rename touches ONLY
  these two consts in the hashing logic now that the shell script is gone).
- `deploy-cloud.sh`: the `grep -oE "pkg/mtg_forge_rs\.…"` probe patterns.
- `web/test_web_server_smoke.js`: the `pkg/mtg_forge_rs…` regexes.
- Every game-page HTML import specifier (independent of this branch).

Folding the hashing into Rust SHRANK this coupling surface: the old shell
script hard-coded the names + sed patterns; now the naming lives in two
Rust consts.
