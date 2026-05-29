# Content-Addressed Immutable Web-Asset Pipeline — Design (mtg-571)

Transient-info stamp: `2026-05-28_#2408(f0c2aa22)`

Status: **DESIGN + PROTOTYPE.** This document is the concrete design that
follows the options survey in
[`CONTENT_ADDRESSED_ASSETS_RESEARCH_20260528.md`](./CONTENT_ADDRESSED_ASSETS_RESEARCH_20260528.md).
The research doc chose **Option 2 (custom Rust post-build hasher)** over
Trunk / JS-bundler / service-worker. This doc specifies the chosen scheme,
the GC, the invariants, and records what the prototype on branch
`trunk-cas-assets` actually implements vs. what remains deferred.

It is **speculative / research-first** and **NOT merge-ready** — it is
intended for the user's PR review. Where the prototype diverges from the
research doc's recommendation (notably the hash function), this doc flags
it explicitly as an open question rather than silently papering over it.

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

| Asset class      | Source name                | Content-addressed name             | Who names it          |
|------------------|----------------------------|------------------------------------|-----------------------|
| Per-set data bin | `<YYYY>-<CODE>.bin`        | `<YYYY>-<CODE>.<hash>.bin`         | exporter (Rust)       |
| wasm-bindgen JS  | `mtg_forge_rs.js`          | `mtg_forge_rs.<hash>.js`          | `hash_web_assets.sh`  |
| wasm-bindgen WASM| `mtg_forge_rs_bg.wasm`     | `mtg_forge_rs_bg.<hash>.wasm`     | `hash_web_assets.sh`  |

`<hash>` is 16 hex chars (64 bits). Collision probability for a few
hundred assets is negligible (birthday bound: ~2^-44 for 600 assets).

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
`init()` call. `hash_web_assets.sh` rewrites both injection points to the
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
| `/pkg/*` (source tree)   | NO (fixed name)    | `no-cache, must-revalidate` + `?v=<sha>`    |
| `/pkg/*` (deployed tree) | YES (staging hash) | hashed, served immutable-eligible           |
| `/data/sets/index.json`  | NO (mutable ptr)   | `no-cache, must-revalidate`                 |
| `/data/decks.bin`        | NO (fixed name)    | `no-cache, must-revalidate` (dedicated rt)  |
| `/data/tokens.bin`       | NO (fixed name)    | `no-cache, must-revalidate` (dedicated rt)  |
| HTML, `server-config.js` | NO (mutable ptr)   | `public, max-age=60`                        |

Note the split-brain on `/pkg/*`: the **committed source HTML** still
imports the fixed name (so `make validate`'s e2e tests are unaffected),
so the web_server's `/pkg/*` route stays `no-cache`. The
content-addressing of the pkg pair happens only on the **deploy-staging
copy**. This is the deliberate staging compromise (see §6).

---

## 3. Hash function — chosen + DISCREPANCY to resolve

The research doc recommended **blake3 truncated to 16 hex** for both
layers. The prototype diverged, and the two layers currently use
**different** hash functions:

| Layer            | Hash in prototype                            | Width |
|------------------|----------------------------------------------|-------|
| Per-set bins     | `std` `DefaultHasher` (SipHash 1-3)          | 64b   |
| pkg JS+WASM      | `sha256sum \| cut -c1-16`                    | 64b   |

Two problems flagged for PR review:

1. **Stale doc-comment.** `main.rs:3828` documents the `hash` field as
   "blake3-derived" but the function (`content_hash_hex`, `main.rs:3837`)
   actually uses `DefaultHasher` (SipHash). The comment must be corrected
   regardless of which hash we keep.
2. **Two hash functions in one "content-addressed" pipeline.** SipHash for
   bins, SHA-256-truncated for pkg. This is a DRY / consistency smell. It
   is *not* a correctness bug — both satisfy the only requirement
   ("different bytes → different name w.h.p.") — but a content-addressed
   scheme reads cleaner with one algorithm. SipHash is also technically
   *seeded* (`DefaultHasher` uses a fixed seed in current std, so it is
   deterministic across runs of the same binary, but std does NOT
   guarantee cross-version stability — a Rust upgrade *could* change the
   bin hashes). That is acceptable for a cache-bust (a hash change just
   means a re-upload) but is worth a conscious decision.

**Recommendation for PR:** pick ONE. Either (a) add the `blake3` crate and
use it for both the exporter and provide a `mtg hash-web-assets`
subcommand so the pkg pair is hashed in Rust too (kills the shell
`sha256sum` and unifies the algorithm — cleanest, matches the research
doc); or (b) explicitly accept SHA-256-truncated for both and drop the
SipHash path. Option (a) is the DRY end-state and folds the pkg hashing
into the same Rust flow as the bins.

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
edited**. The script's two rewrites are confined to controlled injection
points (the ES import specifier + the `init()` arg), consistent with the
project's "No Hacky String Operations On Structured Data" rule. The script
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

- The `?v=<sha>` query-string cache-bust is **replaced** by the staging
  rewrite (`hash_web_assets.sh`) + the manifest GC sweep + `rsync
  --delete`.
- The `?v=<sha>` remains only as a residual safety belt for the
  source-tree `/pkg/*` path that is still fixed-name (until the multi-page
  migration lands). Once the source HTML ships hashed pkg names, `?v=` can
  be retired entirely (tracked by mtg-dxig9).
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
2. **pkg pair content-addressed on the staging copy**
   (`scripts/hash_web_assets.sh`): structured rewrite of the import
   specifier + `init({module_or_path})`. Verified on a staging copy: 5
   pages rewritten, hashed JS/WASM serve 200, old names 404.
3. **Cache tiers** (`mtg-engine/src/web_server/mod.rs`): hashed
   `/data/**/*.bin` → immutable 1y; `decks.bin`/`tokens.bin` get dedicated
   no-cache routes (fixed-name → must NOT be immutable); the immutability
   INVARIANT documented inline.
4. **Deploy GC mark-sweep + rsync --delete** (`scripts/deploy-cloud.sh`):
   replaces `?v=`; prunes orphaned hashed bins; post-deploy probe derives
   hashed pkg names from the deployed HTML.
5. **`Trunk.toml`** declared with documented staged-migration path.

### Deferred (filed follow-ups)

- **mtg-dxig9** — full trunk `rel="rust"` migration of the source HTML
  (so the committed HTML ships hashed pkg, retiring `hash_web_assets.sh`
  and the residual `?v=`).
- **mtg-ntx2j** — content-address `decks.bin` / `tokens.bin` so they can
  join the immutable tier instead of no-cache.
- **mtg-1rvug** — hash the game-page HTML (`tui_game.<hash>.html` etc.) so
  `index.html` is the ONLY mutable HTML pointer (the user's stated ideal).

---

## 9. Open questions for PR review

1. **Hash function unification (§3).** Adopt blake3 in Rust for BOTH bins
   and pkg (recommended; matches research doc, kills the shell
   `sha256sum`), or accept SHA-256-truncated for both, or keep the current
   two-function split? The stale "blake3-derived" doc-comment must be
   fixed either way.
2. **SipHash cross-version stability (§3).** `DefaultHasher` is not
   guaranteed stable across Rust versions. Acceptable for a cache-bust
   (worst case: a spurious re-upload after a toolchain bump), or do we
   want a pinned algorithm for reproducible bin names across builds?
3. **GC grace window (§5).** Keep last-N generations of hashed blobs on
   the VM for clients mid-load across a redeploy, or accept the immediate
   `--delete` (rare 404, self-heals on reload)?
4. **Local-tree GC (§5).** Add `mtg export-wasm --prune` / a make target
   to sweep orphan bins from the local `web/data/sets/` between exports?
5. **Staging-copy vs. source-tree pkg hashing (§6).** Accept the
   split-brain (`/pkg` no-cache in source, hashed only on deploy) until
   mtg-dxig9, or prioritize the multi-page trunk migration sooner?
6. **`index.html` mutable pointer.** Confirm `index.html` +
   `server-config.js` + `index.json` are the intended permanent mutable
   tier (everything else immutable). mtg-1rvug would make `index.html`
   the *sole* mutable HTML.

---

## 10. Validation status

This is a research/prototype branch. Per CPU-courtesy (another agent was
mid `make validate`), a full `make validate` was NOT run in this session.
The prior session recorded light checks (`cargo check`, `cargo clippy
-D warnings`, `cargo fmt --check`, `bash -n`, a live `export-wasm`, and a
local static serve). **Before any PR / merge**, a full `make validate`
must run on a tree rebased onto the post-rename integration (the engine
crate `mtg-forge-rs` → `mtg-engine` rename is queued and will touch the
`mtg_forge_rs.js` / `mtg_forge_rs_bg.wasm` names that `hash_web_assets.sh`
and the probes hard-code).

### Rename impact (mtg-forge-rs → mtg-engine)

When the crate rename lands, the wasm-pack output base name changes from
`mtg_forge_rs` to `mtg_engine`, so the following hard-coded references must
update in lockstep:

- `scripts/hash_web_assets.sh`: `mtg_forge_rs.js` / `mtg_forge_rs_bg.wasm`
  literals + the sed import-specifier patterns + the `grep -oE` probe.
- `deploy-cloud.sh`: the `grep -oE "pkg/mtg_forge_rs\.…"` probe patterns.
- Every game-page HTML import specifier (independent of this branch).

This is a mechanical rename, but it is a real coupling point — flagged so
the rebase onto post-rename integration does not silently leave the
hashing script pointed at a name wasm-pack no longer emits.
