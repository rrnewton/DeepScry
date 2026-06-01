//! Content-addressed asset hashing — the SINGLE source of truth for the
//! content-addressed (immutable) web-asset pipeline (mtg-571).
//!
//! Both content-addressed asset classes hash their bytes through the SAME
//! function here, so a per-set data bin (`<YYYY>-<CODE>.<hash>.bin`, named by
//! the Rust exporter) and the wasm-bindgen pkg pair (`mtg_engine.<hash>.js`
//! / `mtg_engine_bg.<hash>.wasm`, named by the [`web_pkg`] submodule via the
//! `mtg hash-web-assets` subcommand) are hashed identically. DRY: there is no
//! second hash implementation anywhere — the whole content-addressed pipeline
//! is Rust (the old `scripts/hash_web_assets.sh` shell hasher was retired).
//!
//! ## Algorithm
//!
//! [blake3](https://github.com/BLAKE3-team/BLAKE3) truncated to the first 16
//! hex chars (64 bits). blake3 is fast, has no per-process seed (so the same
//! bytes always produce the same name across builds, Rust versions, and
//! machines — unlike `std`'s `DefaultHasher`/SipHash, which std does not
//! guarantee stable across versions), and is a single small dependency. The
//! only requirement for cache-busting is "different bytes -> different name
//! with overwhelming probability"; 64 bits gives a birthday bound of ~2^-44
//! for ~600 assets, which is ample.

/// Number of hex characters (and thus bytes/2) of the blake3 digest embedded
/// in a content-addressed filename. 16 hex chars = 64 bits.
pub const ASSET_HASH_HEX_LEN: usize = 16;

/// Hash `bytes` and return the first [`ASSET_HASH_HEX_LEN`] hex chars of the
/// blake3 digest. This is the one function that names every content-addressed
/// asset in the pipeline.
pub fn asset_hash_hex(bytes: &[u8]) -> String {
    let digest = blake3::hash(bytes);
    // `to_hex` yields the full 64-hex-char digest; truncate to our width.
    let full = digest.to_hex();
    full[..ASSET_HASH_HEX_LEN].to_string()
}

/// Wasm-bindgen pkg hashing + HTML rewrite — the Rust replacement for the
/// retired `scripts/hash_web_assets.sh` (mtg-571).
///
/// This is the deploy-time complement to the exporter's content-addressed
/// `<set>.<hash>.bin` files: the exporter owns the data bins (hashed name lives
/// in `data/sets/index.json`), this module owns the wasm-bindgen code bundle
/// (the hashed name is rewritten into the HTML that imports it). Both layers
/// hash through [`asset_hash_hex`] — ONE algorithm (blake3), ONE implementation
/// (no shell `sha256sum`, no second hasher anywhere).
///
/// See [`hash_web_assets`] for the operation; this submodule groups the
/// pkg-pair naming + structured HTML rewrite logic so it is unit-testable in
/// isolation from the filesystem.
pub mod web_pkg {
    use super::asset_hash_hex;
    use std::path::Path;

    /// wasm-bindgen JS glue base name (no hash, no extension).
    pub const PKG_JS_STEM: &str = "mtg_engine";
    /// wasm-bindgen WASM base name (no hash, no extension).
    pub const PKG_WASM_STEM: &str = "mtg_engine_bg";

    /// The hashed names of the pkg pair, plus the HTML count, returned so the
    /// caller (and tests) can report / assert what happened.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PkgHashResult {
        /// Hashed JS glue filename (e.g. `mtg_engine.<hash>.js`).
        pub js_hashed: String,
        /// Hashed WASM filename (e.g. `mtg_engine_bg.<hash>.wasm`).
        pub wasm_hashed: String,
        /// Number of `*.html` pages whose pkg references were rewritten.
        pub html_pages_rewritten: usize,
    }

    /// Rewrite the two controlled wasm-bindgen injection points in one HTML
    /// page's source text, returning the rewritten string.
    ///
    /// This is a STRUCTURED rewrite of exactly two specifiers we author and
    /// control (per the project's "No Hacky String Operations On Structured
    /// Data" rule), NOT free-form HTML munging:
    ///   1. the ES import specifier `./pkg/mtg_engine.js` (covers both the
    ///      static `from '...'` import and the dynamic `import('...')`), in
    ///      both leading-dot (`./pkg/...`) and leading-slash (`/pkg/...`) forms.
    ///   2. the bare `init()` / `await init()` call → an explicit
    ///      `init({ module_or_path: './pkg/<wasm_hashed>' })`. This is
    ///      wasm-bindgen's documented `module_or_path` override, so the
    ///      generated glue's internal `new URL('mtg_engine_bg.wasm', ...)`
    ///      default is bypassed and the generated glue is never edited.
    ///
    /// Idempotent: an `init(<arg>)` call that already passes an argument is
    /// left untouched (the `init()` match requires empty parens).
    pub fn rewrite_html(src: &str, js_hashed: &str, wasm_hashed: &str) -> String {
        // 1. Import specifier (both `./pkg/` and `/pkg/` forms). We replace the
        //    longer `./pkg/...` token first so the `/pkg/...` pass does not
        //    partially match inside an already-rewritten `./pkg/...`.
        let mut out = src.replace(&format!("./pkg/{PKG_JS_STEM}.js"), &format!("./pkg/{js_hashed}"));
        // Only bare `/pkg/<stem>.js` that is NOT already `./pkg/<stem>.js`
        // (handled above). After the first replace, any remaining
        // `/pkg/<stem>.js` is a genuine leading-slash specifier.
        out = out.replace(&format!("/pkg/{PKG_JS_STEM}.js"), &format!("/pkg/{js_hashed}"));

        // 2. Bare init() call -> explicit module_or_path. Rewrite the longest
        //    form first (`await init()`) so the shorter `init()` pass does not
        //    leave a dangling `await`. We match ONLY empty-paren calls.
        let init_with_path = format!("init({{ module_or_path: './pkg/{wasm_hashed}' }})");
        out = out.replace("await init()", &format!("await {init_with_path}"));
        out = out.replace("init()", &init_with_path);
        out
    }

    /// Content-address the wasm-bindgen pkg pair in `web_dir` IN PLACE.
    ///
    /// `web_dir` must contain `pkg/mtg_engine.js` + `pkg/mtg_engine_bg.wasm`
    /// and the `*.html` pages that import them. Point this at a STAGING COPY,
    /// never the source tree (the committed HTML stays on the fixed-name path
    /// so `make validate`'s e2e tests are unaffected).
    ///
    /// Steps (mirrors the retired `hash_web_assets.sh` exactly, but in Rust so
    /// there is ONE content-addressed pipeline and ONE hash implementation):
    ///   1. hash both files with [`asset_hash_hex`];
    ///   2. rename `pkg/<stem>.js`/`.wasm` to their hashed names (keeping ONLY
    ///      the hashed names, so `rsync --delete` prunes the old names on the VM);
    ///   3. rewrite every `*.html` page's import specifier + `init()` call via
    ///      [`rewrite_html`].
    ///
    /// # Errors
    /// Returns an error if the pkg files are missing or any file I/O fails.
    pub fn hash_web_assets(web_dir: &Path) -> std::io::Result<PkgHashResult> {
        let pkg_dir = web_dir.join("pkg");
        let js = pkg_dir.join(format!("{PKG_JS_STEM}.js"));
        let wasm = pkg_dir.join(format!("{PKG_WASM_STEM}.wasm"));

        if !js.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found (run 'make wasm-network' first)", js.display()),
            ));
        }
        if !wasm.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found (run 'make wasm-network' first)", wasm.display()),
            ));
        }

        let js_hash = asset_hash_hex(&std::fs::read(&js)?);
        let wasm_hash = asset_hash_hex(&std::fs::read(&wasm)?);
        let js_hashed = format!("{PKG_JS_STEM}.{js_hash}.js");
        let wasm_hashed = format!("{PKG_WASM_STEM}.{wasm_hash}.wasm");

        // Rename (keep ONLY the hashed names in the staged tree).
        std::fs::rename(&js, pkg_dir.join(&js_hashed))?;
        std::fs::rename(&wasm, pkg_dir.join(&wasm_hashed))?;

        // Rewrite the HTML pages. Touch ONLY *.html so the .js glue's own
        // internal references are never mangled.
        let mut html_pages_rewritten = 0usize;
        for entry in std::fs::read_dir(web_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                continue;
            }
            let src = std::fs::read_to_string(&path)?;
            let rewritten = rewrite_html(&src, &js_hashed, &wasm_hashed);
            if rewritten != src {
                std::fs::write(&path, rewritten)?;
                html_pages_rewritten += 1;
            }
        }

        Ok(PkgHashResult {
            js_hashed,
            wasm_hashed,
            html_pages_rewritten,
        })
    }
}

/// Full asset-graph content-addresser (mtg-620).
///
/// Extends the pkg-pair rewriter in [`web_pkg`] into a recursive,
/// reference-graph-aware hasher rooted at `index.html`. Everything
/// reachable from `index.html` is hashed (immutable URL); `index.html`
/// itself stays unhashed (the sole stable entrypoint, served short-TTL).
///
/// ### What gets hashed
///
/// - Other HTML pages: `native_game.html`, `tui_game.html`, `demo.html`,
///   `wasm_ai_harness.html`.
/// - JS leaves loaded by `<script src>` / ES `import` / `await import`:
///   `server-config.js`, `network.js`, `bug_report.js`, `lobby_launcher.js`,
///   `help_dialog.js`.
/// - The set-resolver `data/sets/index.json` (fetched as a plain JS
///   string literal from the HTML pages).
/// - The wasm-bindgen pkg pair (delegated to [`web_pkg::hash_web_assets`],
///   run FIRST so its hashed names land inside the HTML before the HTML
///   files are themselves hashed).
///
/// ### What does NOT get hashed
///
/// - `index.html` — the stable entrypoint. Its content IS rewritten to
///   point at the hashed names of its dependencies, but its own
///   filename never changes.
/// - `pkg/package.json` — wasm-pack metadata, never fetched by the
///   browser.
/// - Per-set `<set>.<hash>.bin` files — already content-addressed by
///   the exporter; left untouched.
///
/// ### Cycle break
///
/// The HTML pages cross-link to each other (`tui_game.html` ↔
/// `native_game.html` in their nav bars). With every HTML page hashed,
/// those references form a cycle whose fixpoint we cannot compute
/// post-build (a referrer's hash depends on the hashed names it cites).
/// We break the cycle by rewriting **cross-page HTML→HTML navigation
/// links** to point at `index.html` instead — the lobby is the entry
/// and re-launches the chosen game page with the current hashed name.
/// References to `index.html` itself are kept as-is (unhashed).
///
/// ### Rewrite precision
///
/// All rewrites are filename-token aware (not blind substring replace)
/// and preserve `?query` strings + `#fragments`. The patterns we match:
///
/// - HTML: `<script src="NAME">`, `<a href="NAME[?...][#...]">`.
/// - JS / inline JS: `import ... from './NAME'` / `from '/NAME'`,
///   `import('./NAME')` / `await import('./NAME')`,
///   `fetch('./NAME[?...]')` / `fetch('/NAME[?...]')`,
///   bare string literals like `'./tui_game.html?...'` /
///   `'tui_game.html?...'` (the lobby's redirect builder).
///
/// Each pattern uses an exact filename token bounded by quote/brackets,
/// so a longer name that contains a shorter one as a prefix/suffix
/// cannot accidentally match.
pub mod asset_graph {
    use super::{asset_hash_hex, web_pkg};
    use regex::Regex;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// Sole unhashed entrypoint — its filename never changes; only its
    /// content is rewritten (to point at hashed dependencies).
    pub const ENTRY_HTML: &str = "index.html";

    /// HTML pages (besides `ENTRY_HTML`) that participate in hashing.
    /// Listed explicitly so the staging-tree walk is deterministic and
    /// surfaces any new HTML page as a deliberate addition.
    pub const HASHED_HTML_PAGES: &[&str] = &[
        "native_game.html",
        "tui_game.html",
        "demo.html",
        "wasm_ai_harness.html",
        "deck_editor.html",
    ];

    /// JS leaves loaded by `<script src>` or ES `import`. None of these
    /// have internal JS imports of their own (verified 2026-05-31), so
    /// they are pure leaves: hash bytes once, rename, rewrite referrers.
    /// (`help_dialog.js` added mtg-1vwpd: the shared help-modal module both
    /// game pages import.)
    pub const HASHED_JS_LEAVES: &[&str] = &[
        "server-config.js",
        "network.js",
        "bug_report.js",
        "lobby_launcher.js",
        "help_dialog.js",
    ];

    /// The data leaf — the set→bin resolver. Fetched as a JS string
    /// literal `fetch('./data/sets/index.json')` (or `/data/...`) from
    /// every HTML page that loads card data. Its inner `file:` /
    /// `cards:` fields already reference per-set hashed bins (exporter),
    /// so this file changes whenever those bins change.
    pub const DATA_INDEX_JSON: &str = "data/sets/index.json";

    /// Result of [`hash_full_graph`]. The maps log original→hashed for
    /// each asset class so the caller (and tests) can assert / print
    /// exactly what happened.
    #[derive(Debug, Clone)]
    pub struct GraphHashResult {
        /// Pkg pair (delegated to `web_pkg`).
        pub pkg: web_pkg::PkgHashResult,
        /// `server-config.js` → `server-config.<hash>.js`, etc.
        pub js_leaves: BTreeMap<String, String>,
        /// `data/sets/index.json` → `data/sets/index.<hash>.json`.
        pub data_index: (String, String),
        /// `native_game.html` → `native_game.<hash>.html`, etc.
        pub html_pages: BTreeMap<String, String>,
    }

    /// Insert `.<hash>` before the final extension of `name`. E.g.
    /// `network.js` → `network.<hash>.js`, `data/sets/index.json` →
    /// `data/sets/index.<hash>.json`. Panics if `name` has no extension
    /// (none of our inputs lack one — guarded by the const lists above).
    fn hashed_name(name: &str, hash: &str) -> String {
        let (stem, ext) = name
            .rsplit_once('.')
            .expect("asset_graph: every hashable name must have an extension");
        format!("{stem}.{hash}.{ext}")
    }

    /// One reference-rewrite rule: replace every occurrence of the
    /// FILENAME TOKEN `from` (matched precisely, query/fragment
    /// preserved) with `to` in `src`. Returns the rewritten string.
    ///
    /// The matched contexts are the union of every place HTML/inline-JS
    /// references a sibling asset by name:
    ///
    /// 1. HTML attribute values inside `src="..."` / `src='...'` and
    ///    `href="..."` / `href='...'`, possibly with leading `./` or
    ///    `/`, possibly trailed by `?query` and/or `#fragment`.
    /// 2. JS string literals (single or double quoted), with the same
    ///    leading-prefix + trailing-query/fragment shapes. Covers
    ///    `import(...)`, `fetch(...)`, `window.location.href = '...'`,
    ///    `el.href = '...'`, and the lobby's redirect builders like
    ///    `'tui_game.html' + suffix`.
    ///
    /// Filename-token boundaries are enforced: the character before the
    /// name must be `"`, `'`, `/`, OR begin a quoted JS string (handled
    /// via the `/` and `./` alternatives), and the character after the
    /// name must be `"`, `'`, `?`, or `#`. A longer filename that
    /// contains the shorter as a substring (e.g. `index.html` vs
    /// `super_index.html`) therefore cannot accidentally match.
    fn rewrite_one_reference(src: &str, from: &str, to: &str) -> String {
        // Escape the filename for regex use (dots etc.). Filenames in
        // our const lists are safe ASCII so this is short, but do it
        // properly.
        let from_re = regex::escape(from);
        // The pattern, broken down:
        //   (?P<lead>['"]|['"]\./|['"]/)       -- opening quote + optional prefix
        //   NAME                                 -- the literal filename
        //   (?P<tail>['"]|[?#][^'"<>\s]*['"])    -- closing quote OR query/fragment then quote
        // We allow both single and double quotes (HTML + JS).
        let pat = format!(
            r#"(?P<lead>['"](?:\./|/)?){name}(?P<tail>['"]|[?#][^'"<>\s]*['"])"#,
            name = from_re,
        );
        let re = Regex::new(&pat).expect("asset_graph rewrite: regex compiles");
        re.replace_all(src, |caps: &regex::Captures<'_>| {
            format!("{}{}{}", &caps["lead"], to, &caps["tail"])
        })
        .into_owned()
    }

    /// Apply EVERY rule in `rules` to `src`. Rules are applied longest
    /// `from` first to avoid one rule's `from` being a prefix of
    /// another's (none of our current names overlap that way, but
    /// future-proof the ordering anyway).
    fn rewrite_all_references(src: &str, rules: &BTreeMap<String, String>) -> String {
        let mut keys: Vec<&str> = rules.keys().map(|s| s.as_str()).collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        let mut out = src.to_string();
        for k in keys {
            out = rewrite_one_reference(&out, k, &rules[k]);
        }
        out
    }

    /// Cycle-break helper: in a non-entry HTML page, rewrite every
    /// reference to ANOTHER non-entry hashed HTML page to point at
    /// `ENTRY_HTML` instead. The hashed entry remains `index.html`
    /// (unchanged name), so these become navigation-back-to-lobby
    /// links — exactly the design's "index.html is the entry" model.
    ///
    /// `self_name` is the page being processed, excluded from the
    /// rewrite so it does not self-rewrite (we never link to ourselves
    /// in practice, but be defensive).
    fn redirect_cross_html_to_entry(src: &str, self_name: &str) -> String {
        let mut out = src.to_string();
        for &other in HASHED_HTML_PAGES {
            if other == self_name {
                continue;
            }
            out = rewrite_one_reference(&out, other, ENTRY_HTML);
        }
        out
    }

    /// Hash one file in place: read, hash bytes, rename to its hashed
    /// name, and return the (logical, hashed) pair. `rel` is the path
    /// relative to `web_dir`; the hashed name is in the same directory.
    fn hash_in_place(web_dir: &Path, rel: &str) -> std::io::Result<(String, String)> {
        let src = web_dir.join(rel);
        if !src.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("asset_graph: missing required asset {}", src.display()),
            ));
        }
        let bytes = std::fs::read(&src)?;
        let hash = asset_hash_hex(&bytes);
        let hashed_rel = hashed_name(rel, &hash);
        let dst = web_dir.join(&hashed_rel);
        std::fs::rename(&src, &dst)?;
        Ok((rel.to_string(), hashed_rel))
    }

    /// Run the full asset-graph hasher on `web_dir` IN PLACE.
    ///
    /// Order (matters — a referrer's hash depends on its already-hashed
    /// referents):
    ///   1. pkg pair (delegated). Renames + rewrites every `*.html`.
    ///   2. JS leaves: hash + rename. No internal refs.
    ///   3. `data/sets/index.json`: hash + rename. No internal refs.
    ///   4. Each non-entry HTML page: rewrite refs (JS leaves, data
    ///      leaf, cross-HTML → ENTRY), hash the rewritten content,
    ///      rename. The hashed pkg names are already present from step 1.
    ///   5. ENTRY_HTML (`index.html`): rewrite refs (JS leaves, data
    ///      leaf, other-HTML → their hashed names). Do NOT rename.
    ///
    /// # Errors
    ///
    /// Returns an error if any required file is missing from `web_dir`
    /// or if any filesystem I/O fails (read, write, or rename).
    pub fn hash_full_graph(web_dir: &Path) -> std::io::Result<GraphHashResult> {
        // ── 1. pkg pair via the existing rewriter ────────────────────
        let pkg = web_pkg::hash_web_assets(web_dir)?;

        // ── 2. JS leaves ────────────────────────────────────────────
        let mut js_leaves: BTreeMap<String, String> = BTreeMap::new();
        for leaf in HASHED_JS_LEAVES {
            let (k, v) = hash_in_place(web_dir, leaf)?;
            js_leaves.insert(k, v);
        }

        // ── 3. data/sets/index.json ─────────────────────────────────
        let data_index = hash_in_place(web_dir, DATA_INDEX_JSON)?;

        // Aggregate the "JS + data" rules — applied to every HTML page
        // (entry and non-entry alike).
        let mut leaf_rules: BTreeMap<String, String> = js_leaves.clone();
        leaf_rules.insert(data_index.0.clone(), data_index.1.clone());

        // ── 4. non-entry HTML pages: rewrite then hash ──────────────
        // Two-pass: first pass writes the rewritten (cycle-broken)
        // content back to the logical filename so its bytes-on-disk
        // exactly match what the hash will be computed over. Second
        // pass hashes that content and renames the file.
        let mut html_pages: BTreeMap<String, String> = BTreeMap::new();
        for &page in HASHED_HTML_PAGES {
            let path = web_dir.join(page);
            if !path.is_file() {
                // Some pages may not exist in older staging trees; skip
                // gracefully so a partial deploy tree (e.g. without
                // wasm_ai_harness.html) is still hashable.
                continue;
            }
            let src = std::fs::read_to_string(&path)?;
            let with_leaves = rewrite_all_references(&src, &leaf_rules);
            let cycle_broken = redirect_cross_html_to_entry(&with_leaves, page);
            // Write the rewritten content under the logical name so the
            // bytes we hash are exactly the bytes the server will serve.
            std::fs::write(&path, &cycle_broken)?;
            let (k, v) = hash_in_place(web_dir, page)?;
            html_pages.insert(k, v);
        }

        // ── 5. ENTRY_HTML: rewrite (do NOT rename) ──────────────────
        let entry_path = web_dir.join(ENTRY_HTML);
        if entry_path.is_file() {
            let src = std::fs::read_to_string(&entry_path)?;
            // Entry rewrites: leaves + ALL hashed HTML pages (entry's
            // launch buttons point at the hashed game pages directly).
            let mut entry_rules = leaf_rules.clone();
            for (k, v) in &html_pages {
                entry_rules.insert(k.clone(), v.clone());
            }
            let rewritten = rewrite_all_references(&src, &entry_rules);
            if rewritten != src {
                std::fs::write(&entry_path, rewritten)?;
            }
        }

        Ok(GraphHashResult {
            pkg,
            js_leaves,
            data_index,
            html_pages,
        })
    }

    // Re-export the path constant for callers that want to assert it.
    pub const _: () = ();

    /// Test-only re-export of [`rewrite_all_references`] so the unit
    /// tests in `super::tests` can exercise the rewrite primitive
    /// without needing a real `web_dir` on disk.
    #[doc(hidden)]
    pub fn __test_rewrite_all(src: &str, rules: &BTreeMap<String, String>) -> String {
        rewrite_all_references(src, rules)
    }

    /// Test-only re-export of [`redirect_cross_html_to_entry`].
    #[doc(hidden)]
    pub fn __test_redirect(self_name: &str, src: &str) -> String {
        redirect_cross_html_to_entry(src, self_name)
    }

    /// Convenience: list every file path (relative to `web_dir`) that
    /// `hash_full_graph` will TRY to hash (regardless of whether it
    /// exists on disk). Used by the CLI summary + tests.
    pub fn declared_assets() -> Vec<PathBuf> {
        let mut v = Vec::new();
        v.push(PathBuf::from(DATA_INDEX_JSON));
        for s in HASHED_JS_LEAVES {
            v.push(PathBuf::from(s));
        }
        for s in HASHED_HTML_PAGES {
            v.push(PathBuf::from(s));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_input_same_hash() {
        let a = asset_hash_hex(b"hello content-addressed world");
        let b = asset_hash_hex(b"hello content-addressed world");
        assert_eq!(a, b, "same bytes must yield the same hash");
        assert_eq!(a.len(), ASSET_HASH_HEX_LEN);
    }

    #[test]
    fn different_input_different_hash() {
        let a = asset_hash_hex(b"alpha");
        let b = asset_hash_hex(b"beta");
        assert_ne!(a, b, "different bytes should (w.h.p.) yield different hashes");
    }

    #[test]
    fn matches_known_blake3_prefix() {
        // blake3("") full digest starts af1349b9f5f9a1a6... ; we keep 16 chars.
        let empty = asset_hash_hex(b"");
        assert_eq!(empty, "af1349b9f5f9a1a6");
    }

    #[test]
    fn is_lowercase_hex() {
        let h = asset_hash_hex(b"some bytes");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    mod web_pkg_rewrite {
        use crate::asset_hash::web_pkg::rewrite_html;

        const JS_HASHED: &str = "mtg_engine.deadbeefdeadbeef.js";
        const WASM_HASHED: &str = "mtg_engine_bg.cafef00dcafef00d.wasm";

        #[test]
        fn rewrites_static_import_dot_form() {
            let src = "import init, { foo } from './pkg/mtg_engine.js';";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, format!("import init, {{ foo }} from './pkg/{JS_HASHED}';"));
        }

        #[test]
        fn rewrites_static_import_slash_form() {
            let src = "import init, * as wasm from '/pkg/mtg_engine.js';";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, format!("import init, * as wasm from '/pkg/{JS_HASHED}';"));
        }

        #[test]
        fn rewrites_dynamic_import() {
            let src = "const m = await import('./pkg/mtg_engine.js');";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, format!("const m = await import('./pkg/{JS_HASHED}');"));
        }

        #[test]
        fn rewrites_await_init() {
            let src = "await init();";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, format!("await init({{ module_or_path: './pkg/{WASM_HASHED}' }});"));
        }

        #[test]
        fn rewrites_bare_init() {
            let src = "init();";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, format!("init({{ module_or_path: './pkg/{WASM_HASHED}' }});"));
        }

        #[test]
        fn leaves_init_with_existing_arg_untouched() {
            // An init call that already passes an arg must NOT be double-rewritten.
            let src = "await init({ module_or_path: './pkg/already.wasm' });";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, src, "init(<arg>) must be left untouched");
        }

        #[test]
        fn idempotent_on_already_rewritten_html() {
            let src = "import init from './pkg/mtg_engine.js';\nawait init();";
            let once = rewrite_html(src, JS_HASHED, WASM_HASHED);
            let twice = rewrite_html(&once, JS_HASHED, WASM_HASHED);
            assert_eq!(once, twice, "re-running the rewrite must be a no-op");
        }

        #[test]
        fn graph_rewrite_preserves_query_string() {
            // The lobby's redirect builder: tui_game.html?lobby=&game=&pass=
            // MUST keep its query string when the filename is rewritten.
            use crate::asset_hash::asset_graph;
            let mut rules = std::collections::BTreeMap::new();
            rules.insert(
                "tui_game.html".to_string(),
                "tui_game.deadbeefdeadbeef.html".to_string(),
            );
            // Mimic the inline-JS pattern from index.html lines 853/743.
            let src = "window.location.href = 'tui_game.html?' + qp.toString();\n\
                       el.href = 'tui_game.html' + suffix;\n\
                       <a href=\"tui_game.html#anchor\">go</a>";
            let out = asset_graph::__test_rewrite_all(src, &rules);
            assert!(out.contains("'tui_game.deadbeefdeadbeef.html?'"));
            assert!(out.contains("'tui_game.deadbeefdeadbeef.html'"));
            assert!(out.contains("\"tui_game.deadbeefdeadbeef.html#anchor\""));
        }

        #[test]
        fn graph_rewrite_handles_dot_slash_and_slash_prefixes() {
            use crate::asset_hash::asset_graph;
            let mut rules = std::collections::BTreeMap::new();
            rules.insert(
                "data/sets/index.json".to_string(),
                "data/sets/index.cafef00dcafef00d.json".to_string(),
            );
            let src = "await fetch('./data/sets/index.json');\n\
                       await fetch('/data/sets/index.json');";
            let out = asset_graph::__test_rewrite_all(src, &rules);
            assert!(out.contains("'./data/sets/index.cafef00dcafef00d.json'"));
            assert!(out.contains("'/data/sets/index.cafef00dcafef00d.json'"));
        }

        #[test]
        fn graph_rewrite_does_not_clobber_unrelated_names() {
            use crate::asset_hash::asset_graph;
            let mut rules = std::collections::BTreeMap::new();
            rules.insert("network.js".to_string(), "network.aaaaaaaaaaaaaaaa.js".to_string());
            // `super_network.js` must NOT be rewritten.
            let src = "import './super_network.js';\nimport './network.js';\nawait import('./network.js');";
            let out = asset_graph::__test_rewrite_all(src, &rules);
            assert!(out.contains("'./super_network.js'"), "longer name preserved");
            assert!(out.contains("'./network.aaaaaaaaaaaaaaaa.js'"));
        }

        #[test]
        fn graph_redirect_cross_html_to_entry() {
            use crate::asset_hash::asset_graph;
            let src = "<a href=\"native_game.html\">GUI</a>\n<a href=\"index.html\">lobby</a>";
            let out = asset_graph::__test_redirect("tui_game.html", src);
            assert!(out.contains("href=\"index.html\">GUI"), "cross-page → entry");
            assert!(out.contains("href=\"index.html\">lobby"), "index.html untouched");
        }

        #[test]
        fn does_not_touch_unrelated_init_identifiers() {
            // A method/identifier ending in init must not be clobbered: only a
            // bare `init()` call is a target. `foo.init()` IS matched (ends in
            // `init()`), so guard the common false-positive: `reinit()` style.
            let src = "tui_init_logging();";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, src, "non-empty-paren / unrelated calls untouched");
        }
    }
}
