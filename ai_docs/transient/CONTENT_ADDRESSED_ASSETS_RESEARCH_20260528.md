# Content-Addressed Immutable-Asset Pipeline — Research / Options

Transient-info stamp: `2026-05-28_#2336(0558ace0)`

Status: **RESEARCH ONLY.** No build, deploy-script, or web_server changes
were made. This document surveys options and ends with a phased
recommendation for the user to decide on.

---

## 1. Problem statement

### 1.1 The stale-artifact bug class (mtg-475)

The wasm-bindgen output is **two files that MUST stay version-matched**:

- `web/pkg/mtg_forge_rs.js` — the JS glue. It contains, baked into the
  source, the list of imports the `.wasm` expects, including symbols like
  `__wbindgen_cast_<hash>`.
- `web/pkg/mtg_forge_rs_bg.wasm` — the compiled module.

If a browser (or an intermediate CDN) caches **one** of these across a
redeploy but fetches the **other** fresh, instantiation fails with the
cryptic:

```
WebAssembly.instantiate(): Import #N __wbindgen_cast_<hash>:
function import requires a callable
```

This took several debugging rounds to track down. The current mitigation
(see `mtg-engine/src/web_server/mod.rs` lines ~149-189) is to serve
`/pkg/*` and `/data/*` with `Cache-Control: no-cache, must-revalidate`,
forcing an ETag revalidation round-trip on every request. It works but is
defensive, not structural — the desync is still *possible* the moment a
cache (especially a CDN we don't fully control) ignores `no-cache`.

### 1.2 The Cloudflare cache-control override problem

We deploy behind Cloudflare. Per Cloudflare's docs, a **Cache Response
Rule** with `set_cache_control` takes precedence over origin-set headers
([Cloudflare Cache docs](https://developers.cloudflare.com/cache/concepts/cache-control/),
[Cache Rules settings](https://developers.cloudflare.com/cache/how-to/cache-rules/settings/)).
That means our carefully-tiered origin `Cache-Control` can be silently
overridden at the edge, re-introducing the desync we thought we'd
eliminated. We do not have a guarantee that `no-cache` on `/pkg` survives
the edge.

### 1.3 Why content addressing fixes this *structurally*

If every immutable asset's filename embeds a hash of its own content
(`mtg_forge_rs.<hash>.js`, `mtg_forge_rs_bg.<hash>.wasm`, `1994-LEG.<hash>.bin`),
then:

- A redeploy that changes content produces a **new filename**. Old
  filenames keep serving old (still self-consistent) content; new HTML
  references new filenames. **There is no filename under which a browser
  can hold a stale-but-mismatched copy.**
- We can then serve those files with `Cache-Control: public,
  max-age=31536000, immutable` — the industry best practice
  ([Simon Hearne, Caching Header Best Practices](https://simonhearne.com/2022/caching-header-best-practices/)) —
  and it is *safe even if Cloudflare overrides it to a longer TTL*,
  because the only way to get new bytes is a new URL.
- The revalidation round-trip disappears entirely for everything except
  the single mutable pointer.

The user's stated ideal: **only `index.html` is a mutable pointer;
everything else is content-addressed and immutable.**

---

## 2. Our asset dependency graph

```
                         index.html   (MUTABLE pointer — short TTL)
                         |  |  |  |
        +----------------+  |  |  +--------------------+
        |                   |  +-----------+           |
        v                   v              v           v
  server-config.js    tui_game.html  native_game.html  demo.html
  (generated at        (game pages — structurally identical wrt assets)
   deploy time)                |
                               |  import './pkg/mtg_forge_rs.js'
                               v
                        pkg/mtg_forge_rs.js  ──(new URL('mtg_forge_rs_bg.wasm',
                               |                          import.meta.url))──┐
                               |                                            v
                               |                              pkg/mtg_forge_rs_bg.wasm
                               |
                               |  fetch('./data/decks.bin')
                               |  fetch('./data/tokens.bin')
                               |  fetch('./data/sets/index.json')
                               v
                        data/sets/index.json   ── points at ──> data/sets/<SET>.bin
                          (315 entries, each {file, bytes, card_count})   (315 files)
```

Reference mechanisms, by layer:

| Edge | Referrer | Referent | How the name is encoded |
|------|----------|----------|--------------------------|
| 1 | `index.html` | game HTML, `server-config.js` | static `<a href>` / `<script src>` |
| 2 | game HTML | `pkg/mtg_forge_rs.js` | static ES `import './pkg/...'` |
| 3 | `pkg/mtg_forge_rs.js` | `pkg/mtg_forge_rs_bg.wasm` | **baked into the glue** as `new URL('mtg_forge_rs_bg.wasm', import.meta.url)` (line 2658 of the generated JS). This is the crux — see §4. |
| 4 | game HTML | `data/sets/index.json`, `data/decks.bin`, `data/tokens.bin` | runtime `fetch('./data/...')` |
| 5 | `data/sets/index.json` | `data/sets/<SET>.bin` | runtime: JSON field `file` per set entry |

**Key observation:** `data/sets/index.json` is *already a manifest*. It is
generated by the Rust exporter (`mtg-engine/src/main.rs` ~lines 3800-3848,
the `SetIndex { version, sets, cards }` struct) and each set entry already
carries `{ file, bytes, card_count }`. Making the `.bin` files
content-addressed is therefore the *cheapest* win: hash each bin, rename
to `<SET>.<hash>.bin`, and write the hashed name into the `file` field.
Zero client-code change is needed because the client already reads the
name out of the manifest.

---

## 3. The five options

Each is rated for maturity (verified via web search, May 2026), fit with
our `wasm-pack` + Rust-exporter flow, rough effort, and pros/cons.

### Option 1 — Trunk (Rust-WASM bundler)

**How it works.** `trunk build` runs `cargo build --target wasm32`, runs
**its own** `wasm-bindgen` invocation, optionally `wasm-opt`, and processes
`data-trunk`-annotated link/asset directives in a source `index.html`,
emitting a `dist/` with content-hashed JS+WASM and a rewritten
`index.html`. Trunk auto-downloads and manages the `wasm-bindgen` /
`wasm-opt` binaries itself
([Trunk commands](https://trunkrs.dev/commands/),
[Trunk getting-started / FAQ](https://trunkrs.dev/)).

**Hashing story.** Script/JS-snippet/CSS/SCSS content **is** hashed for
cache-control by default; `rel="copy-file"` / `rel="copy-dir"` assets are
copied **without** hashing
([Trunk assets guide](https://trunkrs.dev/guide/assets/)). Trunk also adds
Subresource-Integrity (`integrity`, default sha384) by default. So Trunk
hashes the JS+WASM pair (great — it owns the §4 wrinkle), but our 315
`.bin` data files would fall under `copy-dir` and **would not be hashed**.

**Maturity.** Actively maintained. Latest release **0.21.14, 2025-05-08**
(92 releases total) per [trunk-rs/trunk on GitHub](https://github.com/trunk-rs/trunk).
Mature and widely used in the Yew/Leptos ecosystem.

**Fit with our flow.** **Poor / disruptive.** Trunk wants to *own* the
build: it runs its own `cargo build` + its own `wasm-bindgen`. Adopting it
means **abandoning `make wasm-network`** (the `wasm-pack build` step) and
restructuring the WASM crate to be a Trunk target with a Trunk-managed
`index.html`. Our `index.html` is a hand-authored landing page with three
downstream game pages, not a single SPA entry — that does not map cleanly
onto Trunk's one-`index.html`-per-target model. And Trunk does **not**
hash the `.bin` data files (the bulk of our cacheable bytes and a real
source of staleness), which is the part we most want addressed. We'd still
need a custom step for the bins, so Trunk buys us only the pkg hashing
while costing us the entire build restructure.

**Effort.** High (build restructure + multi-page reconciliation).
**Verdict.** Too disruptive for the marginal benefit. Not recommended.

### Option 2 — Custom post-build hasher (Rust, blake3)

**How it works.** A small Rust binary / `make` step that runs *after*
`wasm-export` + `wasm-pack`, over the assembled `web/` tree:

1. Hash each immutable asset (`blake3`, truncated to ~16 hex chars).
2. Rename `name.ext` → `name.<hash>.ext`.
3. Rewrite every referrer (see the layer table in §2).
4. Emit a top-level `manifest.json` (optional; useful for the deploy
   script and for debugging).

**Per-layer rewrite plan (the design work):**

- **Layer 5 (bins, EASIEST):** extend the exporter
  (`mtg-engine/src/main.rs`) so that as it writes each `<SET>.bin` it
  hashes the bytes and writes `<SET>.<hash>.bin`, putting the hashed name
  in the existing `file` field of `index.json`. **No client change** —
  the client already reads `file` from the manifest. Optionally add a
  `hash` field alongside `bytes`/`card_count`. This is the natural
  extension of the existing manifest and is the cleanest single win.
- **Layer 4 (index.json itself):** hash `index.json` → `index.<hash>.json`
  and reference it from a tiny mutable bootstrap. Simplest: keep
  `data/sets/index.json` itself mutable+short-TTL (it's small, ~tens of KB)
  and let it be the manifest for the bins. Hashing index.json adds a
  bootstrap indirection for marginal benefit — defer.
- **Layer 2 (HTML → pkg JS):** static rewrite at build time. Replace
  `import './pkg/mtg_forge_rs.js'` and the `await init()` call. Because
  the import specifier is static, the hasher edits the HTML text after
  computing the JS hash.
- **Layer 3 (JS glue → WASM, THE CRUX):** see §4. Resolved by passing the
  hashed `.wasm` URL explicitly to `init({ module_or_path: ... })`, set
  from a value the hasher injects into the HTML. No edit of the glue
  needed.

**Crates.** `blake3` (fast, ~one dep) for hashing; std `fs` for
rename/copy. For HTML rewriting, prefer structured edits over blind string
replace where feasible — but the references here (`<script src>`, the ES
`import` specifier, the `init(...)` arg) are well-defined injection points
we control, so a templated bootstrap (see §4) is cleaner than regex over
arbitrary HTML. The bin rewriting needs **no** string ops at all — it's a
field in a struct the exporter already serializes.

**Estimate.** Phase 1 (bins only, in the exporter): ~40-80 LOC of Rust in
`main.rs` + the `hash` field. Phase 2 (pkg JS+WASM + HTML/init injection):
~150-250 LOC for a `mtg hash-web-assets` subcommand + Makefile wiring.

**Maturity / fit.** **Excellent fit.** Stays entirely inside our Rust +
`make` + `wasm-pack` flow, adds no Node toolchain, reuses the manifest we
already emit. `blake3` is mature and stable.
**Verdict.** Recommended path.

### Option 3 — JS bundler (Vite / esbuild / Parcel / rsbuild)

**How it works.** Industry-standard web bundlers fingerprint
JS/CSS/asset filenames and rewrite references automatically. Vite (Rollup-
based) and Parcel both have first-class WASM + asset-hashing support;
esbuild is the fastest but lower-level; rsbuild (Rspack) is the newer
Rust-core entrant.

**Cost/benefit for us.** Pulls a **full Node toolchain** (`web/` already
has a vestigial `node_modules` + `package.json`, but only for the Puppeteer
e2e tests, not for building shipped assets) into the *build-and-ship*
critical path of an otherwise pure-Rust deploy. For our asset graph —
3 game HTML pages, one wasm-bindgen pair, 315 opaque `.bin` blobs already
indexed by a Rust-emitted manifest — a JS bundler is heavyweight and
introduces a second source of truth for what gets shipped. The `.bin`
files are not JS-module imports; they're runtime `fetch`es keyed off a
manifest we control in Rust, so the bundler's import-graph analysis adds
little. **Little gain for meaningful added complexity and a new toolchain
dependency.**
**Verdict.** Not recommended (overkill; toolchain mismatch).

### Option 4 — Status quo: ETag revalidation (the baseline)

**What we have.** `ServeDir` sets ETags; `/pkg` + `/data` are
`no-cache, must-revalidate`; HTML is `max-age=60`. Browsers re-validate
every `/pkg` + `/data` request and get a cheap `304` when unchanged.

**What content-hashing buys *over* this:**

1. **Eliminates the revalidation round-trip.** Today every page load
   re-validates the JS, the WASM, the index.json, and every `.bin`
   touched — a `304` each. With immutable hashed names those become
   *zero* network requests after first cache. For 315 potential bin
   fetches this is material on repeat/slow connections.
2. **Removes the Cloudflare-override exposure (§1.2).** `no-cache` only
   protects us if every cache layer honors it. A Cache Response Rule can
   override it. Content addressing makes staleness *impossible* regardless
   of TTL, so a longer edge TTL is harmless instead of dangerous.
3. **Lets us use `immutable, max-age=31536000` safely** — the documented
   best practice that pairs immutable directive with unique filenames
   ([Cloudflare community: immutable](https://community.cloudflare.com/t/cache-control-immutable/32814),
   [Simon Hearne](https://simonhearne.com/2022/caching-header-best-practices/)).

**Being fair.** ETag revalidation is *correct* and already deployed; the
desync risk only materializes when a cache disobeys `no-cache`. If we were
not behind a CDN that can override headers, the status quo would be a
defensible permanent answer. Given that we *are* behind Cloudflare and
have *already been bitten* by the desync, content-hashing the pkg pair is
worth it. The `.bin` hashing is a smaller, mostly-performance win.
**Verdict.** Acceptable fallback; keep as the policy for the *mutable*
pointer files. Not sufficient as the whole story given §1.2.

### Option 5 — Service worker / app-shell

**How it works.** A service worker precaches an app-shell and intercepts
fetches, giving full client-side control of caching/versioning.

**Fit.** **Orthogonal / overkill.** It's a client-side cache layer, not a
content-addressing scheme; it adds its own versioning/cache-invalidation
complexity (the very class of bug we're trying to make *structurally*
impossible). Content addressing at the filename level is simpler and
needs no SW lifecycle management. Mentioned for completeness only.
**Verdict.** Do not pursue for this goal.

---

## 4. The wasm-bindgen `.js` ↔ `.wasm` naming wrinkle (THE CRUX)

The generated glue resolves the binary itself:

```js
// web/pkg/mtg_forge_rs.js, ~line 2658
if (typeof module_or_path === 'undefined') {
    module_or_path = new URL('mtg_forge_rs_bg.wasm', import.meta.url);
}
```

Our game pages call `await init()` with **no argument** (e.g.
`web/tui_game.html` line 1203), so this default fires and the `.wasm`
filename `mtg_forge_rs_bg.wasm` is effectively hard-coded inside the glue.
Naively renaming the `.wasm` to a hashed name would break the glue's
self-reference. This is exactly why §1.1's desync is possible.

**Resolution — pass the hashed URL explicitly (verified).** The `init`
function accepts `module_or_path`; you can pass the `.wasm` URL in:

```js
import init, { /* … */ } from './pkg/mtg_forge_rs.<JSHASH>.js';
await init({ module_or_path: './pkg/mtg_forge_rs_bg.<WASMHASH>.wasm' });
```

This is documented wasm-bindgen behavior
([wasm-bindgen guide / without-a-bundler](https://rustwasm.github.io/docs/wasm-bindgen/examples/without-a-bundler.html),
[wasm-bindgen CLI reference](https://rustwasm.github.io/docs/wasm-bindgen/reference/cli.html))
and is precisely what JS bundlers do under the hood (`import wasmUrl from
'…_bg.wasm?url'; await init({ module_or_path: wasmUrl })`). Because the
`init()` arg is supplied by our HTML (which the hasher rewrites/templates),
**we do not have to edit the generated glue at all** when `init()` is
called with an explicit URL.

How each option handles the wrinkle:

| Option | Handling |
|--------|----------|
| **Trunk** | Owns it automatically — Trunk runs wasm-bindgen and rewrites the references for its hashed output. (But costs the build restructure; §3.1.) |
| **Custom hasher** | Inject the hashed `.wasm` URL into the `init({ module_or_path })` call in the HTML; rename both files; **no glue edit.** Cleanest within our flow. |
| **JS bundler** | Handles it via the `?url` import pattern automatically. (But Node toolchain; §3.3.) |
| **Status quo** | Avoids it by NOT renaming + forcing revalidation. Fragile under CDN override (§1.2). |
| **Service worker** | Would need its own version key; doesn't address the wrinkle structurally. |

Secondary option if we ever call `init()` in a context where injecting the
arg is awkward: wasm-bindgen's `--out-name` only changes the *base* name,
not the hash, and post-editing the single `new URL('…', import.meta.url)`
literal in the glue is a structured one-line replace — but the explicit
`init({ module_or_path })` approach is strictly cleaner and is preferred.

---

## 5. Recommendation (phased)

Adopt the **custom Rust post-build hasher (Option 2)**. It fits our
existing `wasm-export` + `wasm-pack` + `make` + `deploy-cloud.sh` flow,
adds no new toolchain, and reuses the manifest the exporter already emits.
Do it in three phases so value lands incrementally and risk stays low.

**Phase 1 — content-address the `.bin` data files (cheapest, do first).**
The exporter already writes `data/sets/index.json` with a `file` field per
set. Extend `mtg-engine/src/main.rs` to hash each `.bin` (blake3) and write
`<SET>.<hash>.bin`, putting the hashed name in `file` (optionally add a
`hash` field). **Zero client change** — the client reads `file` from the
manifest. Do the same for `decks.bin` / `tokens.bin` (these are referenced
directly by HTML `fetch`, so either keep them mutable+short-TTL or add them
to a tiny manifest). Then have `web_server` serve `/data/sets/*.bin` with
`immutable, max-age=1yr`, keeping `index.json` itself short-TTL (it's the
per-bin pointer). This alone removes the bulk of cacheable bytes from the
revalidation path and makes bin staleness impossible.

**Phase 2 — content-address the pkg bundle (fixes the actual mtg-475
bug class).** Add an `mtg hash-web-assets` step (or a Makefile shim) that,
after `wasm-pack`, hashes `mtg_forge_rs.js` + `mtg_forge_rs_bg.wasm`,
renames both, and rewrites the game pages' `import './pkg/…'` specifier +
the `init({ module_or_path: '…_bg.<hash>.wasm' })` call (§4). Serve `/pkg/*`
`immutable, max-age=1yr`. This makes the JS↔WASM desync **structurally
impossible**, which is the whole point of mtg-475.

**Phase 3 — `index.html` as the sole mutable pointer.** Hash the game
HTML pages (`tui_game.html`, `native_game.html`, `demo.html`) and have
`index.html` link to the hashed names. `index.html` and the deploy-
generated `server-config.js` remain the only mutable, short-TTL files.
This realizes the user's stated ideal exactly.

Keep ETag/short-TTL (Option 4 mechanics) for the residual mutable pointers
(`index.html`, `server-config.js`, `index.json`).

---

## 6. What we should NOT do

- **Do NOT adopt Trunk.** It would force abandoning `make wasm-network`
  and restructuring the WASM crate around a single Trunk `index.html`,
  yet it would still not hash our 315 `.bin` files (those are `copy-dir`,
  un-hashed). High cost, partial coverage.
- **Do NOT pull in a JS bundler (Vite/esbuild/Parcel/rsbuild)** for
  shipping assets. It adds a Node build dependency to a pure-Rust deploy
  for a graph (opaque bins behind a Rust-emitted manifest) it doesn't
  model well. (The existing `web/node_modules` is for Puppeteer e2e tests
  only — do not let it metastasize into the ship pipeline.)
- **Do NOT use a service worker** for versioning. It re-introduces a
  client-side cache-invalidation problem — the opposite of "structurally
  impossible."
- **Do NOT post-edit the wasm-bindgen glue's `new URL(...)` literal** if
  the explicit `init({ module_or_path })` injection works (it does, §4).
  Prefer passing the URL over rewriting generated code.
- **Do NOT hand-roll string-replacement over arbitrary HTML.** Confine
  rewrites to the controlled injection points (the ES import specifier and
  the `init()` argument), ideally via a small template, consistent with
  the project's "No Hacky String Operations On Structured Data" rule.
- **Do NOT rely on origin `Cache-Control` alone behind Cloudflare** for
  correctness — a Cache Response Rule can override it (§1.2). Correctness
  must come from content-addressed filenames, not header politeness.

---

## 7. Open questions for the user

1. **Hash length / algorithm.** blake3 truncated to 16 hex chars (64 bits)
   — acceptable collision margin for a few hundred assets? Or sha256
   truncated for "boring" familiarity? (blake3 is faster; recommended.)
2. **Phase scope now.** Land Phase 1 (bins) only, or Phase 1+2 (bins +
   pkg) together? Phase 2 is the one that actually kills mtg-475.
3. **`decks.bin` / `tokens.bin` placement.** Fold them into the
   `index.json` manifest, give them their own tiny manifest, or keep them
   mutable+short-TTL? (They're referenced directly by HTML `fetch`.)
4. **Where does hashing run** — extend the existing `export-wasm`
   subcommand vs. a new `mtg hash-web-assets` subcommand vs. a Makefile
   step? (Recommend: bins inside the exporter; pkg/HTML in a new
   subcommand invoked by the Makefile after `wasm-pack`.)
5. **Deploy `--delete` interaction.** `deploy-cloud.sh` rsyncs `web/` with
   `--delete`. With hashed names, old immutable files would be deleted on
   redeploy — fine for a single-origin deploy (no clients mid-load from the
   old version expected), but worth confirming we don't want a grace
   window keeping the previous generation's hashed files around.
6. **`server-config.js`** is generated at deploy time and is inherently
   mutable — confirm it stays in the short-TTL pointer tier (recommended).
