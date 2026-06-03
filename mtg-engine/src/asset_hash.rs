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

/// Full asset-graph content-addresser (mtg-620, generalized mtg-682,
/// CAS-hardened mtg-4irju).
///
/// A GENERAL, graph-aware renamer for the deploy-staging web tree. It hashes
/// every web asset to an immutable `<stem>.<blake3>.<ext>` name and rewrites
/// every reference to point at the hashed target — EXCEPT the one stable
/// bootstrap entry ([`ENTRY_HTML`] = `index.html`, the server's default URL,
/// the SOLE mutable/no-cache file after this pass).
///
/// ### Why a graph (and not a hardcoded list)
///
/// The earlier version hardcoded the HTML page set and broke when the
/// lobby-redo added `launcher.html` (never listed → served fixed-name → 404 on
/// deploy). This version **auto-discovers** every `*.html` in `web_dir`
/// (sorted → deterministic), so a new page is hashed automatically. It builds
/// the actual reference graph across HTML pages, JS leaves, and the data index
/// and rewrites every reference to its real hashed target.
///
/// ### Pure-DAG topology (no runtime manifest — mtg-4irju)
///
/// The web nav graph is now a strict DAG: `index.html → launcher/game/editor
/// pages`, `launcher.html → {game pages, deck_editor.html}`, `demo.html → game
/// pages`, and the pure leaves (pkg, `lobby_launcher.js`, `network.js`,
/// `server-config.js`, the data index, …) that reference nothing. There are NO
/// cycles: the old `tui_game ⇄ native_game` renderer-switch links were dropped,
/// `lobby_launcher.js` was leaf-ified (it no longer names the game pages), and
/// every BACK-edge (e.g. `deck_editor → launcher`) was rerouted through the
/// stable entry as `index.html?goto=<logical>&release=<token>` — a same-origin
/// hop the entry's inline dispatcher resolves. So every FORWARD edge is
/// statically hashable by **topological hashing**: hash referents before
/// referrers (reverse-topological order), so a referrer bakes in every name it
/// cites. The previous `asset_manifest.js` runtime loader + stable-named
/// `asset-manifest.json` + `[data-asset-href]` rewrite are GONE — they were a
/// cache vulnerability (a stale cached loader/manifest served an old hash →
/// 404).
///
/// ### The release token (content-hashed manifest = Merkle root)
///
/// After hashing every asset we build the full `logical → hashed` map and
/// CONTENT-HASH it via [`asset_hash_hex`] → `asset-manifest.<hash>.json`
/// (itself immutable). Because each hashed name embeds its asset's content
/// hash, this single hash transitively fingerprints the whole release graph —
/// it IS the release identity ("release token"). It is PURELY content-derived
/// (no build SHA / timestamp folded in), so identical `web/` content always
/// yields an identical token across rebuilds (the CAS reproducibility
/// property). The token is baked into `index.html` (the only mutable file) by
/// replacing the [`RELEASE_TOKEN_PLACEHOLDER`] sentinel; `index.html` threads
/// `release=<token>` onto its forward links and resolves `?goto=` back-edges
/// against `asset-manifest.<token>.json`. The token is NEVER baked into a
/// hashed page's content (that would be circular: the manifest hash depends on
/// the page hashes) — hashed pages relay `release=` from their own URL at
/// runtime.
///
/// Concretely the algorithm:
///   1. Hash the pkg pair (delegated to [`web_pkg::hash_web_assets`]).
///   2. Hash the JS leaves (incl. the now-leaf `lobby_launcher.js`) + the data
///      index (pure leaves: no graph edges).
///   3. Build the reference graph over the remaining HTML pages, assert it is
///      ACYCLIC (Tarjan; a multi-node SCC is a hard error naming the offending
///      files — reintroducing a cycle would resurrect the cache vuln), and
///      hash in reverse-topological order, statically baking each already-hashed
///      referent into its referrers.
///   4. Build the full `logical → hashed` manifest, content-hash it → write
///      `asset-manifest.<token>.json`.
///   5. Rewrite the stable entry (`index.html`) statically (every name it cites
///      is hashed now) and bake the release token into it — without renaming.
///
/// ### Rewrite precision
///
/// All rewrites are filename-token aware (regex-bounded, NOT blind substring
/// replace, per the "No Hacky String Operations On Structured Data" rule) and
/// preserve `?query` strings + `#fragments`. See [`rewrite_one_reference`].
pub mod asset_graph {
    use super::{asset_hash_hex, web_pkg};
    use regex::Regex;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    /// Sole unhashed HTML entrypoint — its filename never changes; only its
    /// content is rewritten (to point at hashed dependencies) plus the release
    /// token baked in. This is the server's default URL AND the only mutable /
    /// no-cache file, so it must stay stable.
    pub const ENTRY_HTML: &str = "index.html";

    /// Stem of the content-hashed manifest. The full served name is
    /// `asset-manifest.<token>.json` (immutable, content-addressed), where the
    /// token is the blake3 hash of the manifest JSON = the release identity.
    pub const MANIFEST_STEM: &str = "asset-manifest";

    /// The sentinel `index.html` author-places where the release token belongs.
    /// [`hash_full_graph`] replaces this exact string with the computed token
    /// (a structured replace of a unique sentinel — NOT a blind munge). On the
    /// un-hashed source/dev tree it stays as-is; the entry's dispatcher treats
    /// a non-resolving token as the identity, so dev nav still works.
    pub const RELEASE_TOKEN_PLACEHOLDER: &str = "__MTG_RELEASE_TOKEN__";

    /// Stable assets that participate in the graph but are NEVER renamed:
    /// only the entry page (`index.html`) qualifies now.
    fn is_stable_html(name: &str) -> bool {
        name == ENTRY_HTML
    }

    /// JS leaves loaded by `<script src>` or ES `import` that reference NO
    /// other hashable HTML (pure leaves). Auto-discovery (step 3) treats any
    /// `.js` that DOES reference HTML as a graph node instead. These are hashed
    /// up front in step 2. `lobby_launcher.js` is now a pure leaf (mtg-4irju
    /// leaf-ified it: it no longer names the game pages), so it lives here and
    /// the game pages' `import './lobby_launcher.js'` is statically rewritten.
    pub const HASHED_JS_LEAVES: &[&str] = &[
        "server-config.js",
        "network.js",
        "bug_report.js",
        "help_dialog.js",
        "lobby_launcher.js",
    ];

    /// Compute the content-hashed manifest filename from its JSON bytes:
    /// `asset-manifest.<blake3>.json`. Returned alongside the token.
    fn manifest_name(token: &str) -> String {
        format!("{MANIFEST_STEM}.{token}.json")
    }

    /// The data leaf — the set→bin resolver. Fetched as a JS string literal
    /// `fetch('./data/sets/index.json')` (or `/data/...`) from every HTML page
    /// that loads card data.
    pub const DATA_INDEX_JSON: &str = "data/sets/index.json";

    /// Result of [`hash_full_graph`]. The maps log original→hashed for each
    /// asset class so the caller (and tests) can assert / print what happened.
    #[derive(Debug, Clone)]
    pub struct GraphHashResult {
        /// Pkg pair (delegated to `web_pkg`).
        pub pkg: web_pkg::PkgHashResult,
        /// `server-config.js` → `server-config.<hash>.js`, etc. (pure JS leaves).
        pub js_leaves: BTreeMap<String, String>,
        /// `data/sets/index.json` → `data/sets/index.<hash>.json`.
        pub data_index: (String, String),
        /// Auto-discovered HTML pages (besides the entry): logical → hashed.
        pub graph_nodes: BTreeMap<String, String>,
        /// The FULL `logical → hashed` map for the whole release (pkg pair, JS
        /// leaves, data index, and every hashed HTML page). Its deterministic
        /// JSON serialization is content-hashed to produce [`release_token`];
        /// it is served as `asset-manifest.<token>.json` for the entry's
        /// `?goto=` dispatcher.
        pub manifest: BTreeMap<String, String>,
        /// The release token = blake3 of the manifest JSON = a Merkle root over
        /// the whole release graph. Baked into `index.html`.
        pub release_token: String,
        /// The served manifest filename, `asset-manifest.<token>.json`.
        pub manifest_file: String,
    }

    /// Insert `.<hash>` before the final extension of `name`. E.g.
    /// `network.js` → `network.<hash>.js`, `data/sets/index.json` →
    /// `data/sets/index.<hash>.json`. Panics if `name` has no extension
    /// (none of our inputs lack one).
    fn hashed_name(name: &str, hash: &str) -> String {
        let (stem, ext) = name
            .rsplit_once('.')
            .expect("asset_graph: every hashable name must have an extension");
        format!("{stem}.{hash}.{ext}")
    }

    /// One reference-rewrite rule: replace every occurrence of the FILENAME
    /// TOKEN `from` (matched precisely, query/fragment preserved) with `to` in
    /// `src`. Returns the rewritten string.
    ///
    /// The matched contexts are the union of every place HTML/inline-JS
    /// references a sibling asset by name:
    ///
    /// 1. HTML attribute values inside `src="..."` / `href="..."` (and the
    ///    `data-asset-href="..."` logical-name attribute the manifest loader
    ///    consumes), possibly with a leading `./` or `/`, possibly trailed by
    ///    `?query` and/or `#fragment`.
    /// 2. JS string literals (single or double quoted), same shapes. Covers
    ///    `import(...)`, `fetch(...)`, `window.location.href = '...'`, and the
    ///    lobby's redirect builders like `'tui_game.html' + suffix`.
    ///
    /// Filename-token boundaries are enforced via the leading quote (+ optional
    /// `./`/`/`) and the trailing quote/`?`/`#`, so a longer filename that
    /// contains the shorter as a substring cannot accidentally match.
    fn rewrite_one_reference(src: &str, from: &str, to: &str) -> String {
        let from_re = regex::escape(from);
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

    /// Apply EVERY rule in `rules` to `src`. Rules are applied longest `from`
    /// first to avoid one rule's `from` being a prefix of another's.
    fn rewrite_all_references(src: &str, rules: &BTreeMap<String, String>) -> String {
        let mut keys: Vec<&str> = rules.keys().map(|s| s.as_str()).collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        let mut out = src.to_string();
        for k in keys {
            out = rewrite_one_reference(&out, k, &rules[k]);
        }
        out
    }

    /// Scan `src` for ALL references (module edges + soft refs: href, bare JS
    /// string literals) to candidate `names`. Used to build the FULL reference
    /// graph; in the CAS pure-DAG model any multi-node SCC is a hard error
    /// (a reintroduced cycle). Same filename-token boundary rules as
    /// [`rewrite_one_reference`].
    fn all_references<'a>(src: &str, names: &'a BTreeSet<String>) -> BTreeSet<&'a str> {
        let mut out = BTreeSet::new();
        for name in names {
            let n = regex::escape(name);
            let pat = format!(r#"['"](?:\./|/)?{n}(?:['"]|[?#])"#);
            let re = Regex::new(&pat).expect("asset_graph all-ref scan: regex compiles");
            if re.is_match(src) {
                out.insert(name.as_str());
            }
        }
        out
    }

    /// Hash one file in place: read, hash bytes, rename to its hashed name,
    /// return the (logical, hashed) pair. `rel` is the path relative to
    /// `web_dir`; the hashed name lands in the same directory.
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

    /// Auto-discover every `*.html` file directly in `web_dir`, sorted for a
    /// deterministic order. Includes the entry (the caller filters it out
    /// where renaming is concerned). Does NOT recurse — all our pages are flat
    /// in `web/`.
    fn discover_html_pages(web_dir: &Path) -> std::io::Result<Vec<String>> {
        let mut pages = Vec::new();
        for entry in std::fs::read_dir(web_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".html") {
                pages.push(name.into_owned());
            }
        }
        pages.sort();
        Ok(pages)
    }

    /// Tarjan strongly-connected components over the graph `nodes` with
    /// adjacency `edges` (node → set of nodes it references). Returns the SCCs
    /// in REVERSE-topological order (a component appears before the components
    /// that depend on it) — exactly the order to hash in (referents first).
    ///
    /// Tarjan naturally emits SCCs in reverse-topological order, which is what
    /// we want: hash the leaves of the condensation first.
    fn tarjan_sccs(nodes: &BTreeSet<String>, edges: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
        // Iterative Tarjan to avoid recursion-depth concerns (small graphs in
        // practice, but keep it robust + allocation-light).
        #[derive(Clone)]
        struct Frame {
            node: String,
            child_idx: usize,
        }
        let mut index_of: BTreeMap<String, usize> = BTreeMap::new();
        let mut lowlink: BTreeMap<String, usize> = BTreeMap::new();
        let mut on_stack: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = Vec::new();
        let mut next_index = 0usize;
        let mut sccs: Vec<Vec<String>> = Vec::new();

        // Deterministic iteration over a sorted node list.
        for root in nodes {
            if index_of.contains_key(root) {
                continue;
            }
            let mut call_stack: Vec<Frame> = vec![Frame {
                node: root.clone(),
                child_idx: 0,
            }];
            while let Some(frame) = call_stack.last_mut() {
                let node = frame.node.clone();
                if frame.child_idx == 0 {
                    index_of.insert(node.clone(), next_index);
                    lowlink.insert(node.clone(), next_index);
                    next_index += 1;
                    stack.push(node.clone());
                    on_stack.insert(node.clone());
                }
                let empty = BTreeSet::new();
                let children: Vec<&String> = edges.get(&node).unwrap_or(&empty).iter().collect();
                if frame.child_idx < children.len() {
                    let child = children[frame.child_idx].clone();
                    frame.child_idx += 1;
                    if !index_of.contains_key(&child) {
                        call_stack.push(Frame {
                            node: child,
                            child_idx: 0,
                        });
                    } else if on_stack.contains(&child) {
                        let cl = index_of[&child];
                        let e = lowlink.get_mut(&node).unwrap();
                        if cl < *e {
                            *e = cl;
                        }
                    }
                } else {
                    // Done with `node`. Propagate lowlink to parent.
                    if lowlink[&node] == index_of[&node] {
                        let mut comp = Vec::new();
                        loop {
                            let w = stack.pop().unwrap();
                            on_stack.remove(&w);
                            let is_root = w == node;
                            comp.push(w);
                            if is_root {
                                break;
                            }
                        }
                        comp.sort();
                        sccs.push(comp);
                    }
                    let finished_low = lowlink[&node];
                    call_stack.pop();
                    if let Some(parent) = call_stack.last() {
                        let p = parent.node.clone();
                        let e = lowlink.get_mut(&p).unwrap();
                        if finished_low < *e {
                            *e = finished_low;
                        }
                    }
                }
            }
        }
        sccs
    }

    /// Run the full asset-graph hasher on `web_dir` IN PLACE.
    ///
    /// See the module-level docs for the algorithm + cycle mechanism.
    ///
    /// # Errors
    /// Returns an error if a required asset is missing or any filesystem I/O
    /// fails (read, write, or rename).
    pub fn hash_full_graph(web_dir: &Path) -> std::io::Result<GraphHashResult> {
        // ── 1. pkg pair via the existing rewriter ────────────────────────
        let pkg = web_pkg::hash_web_assets(web_dir)?;

        // ── 2. pure JS leaves + the data index ───────────────────────────
        let mut js_leaves: BTreeMap<String, String> = BTreeMap::new();
        for leaf in HASHED_JS_LEAVES {
            if web_dir.join(leaf).is_file() {
                let (k, v) = hash_in_place(web_dir, leaf)?;
                js_leaves.insert(k, v);
            }
        }
        let data_index = hash_in_place(web_dir, DATA_INDEX_JSON)?;

        // The "leaf rules" are applied to EVERY remaining file (HTML pages, the
        // graph-JS like lobby_launcher.js, and the entry): they have no cycles.
        let mut leaf_rules: BTreeMap<String, String> = js_leaves.clone();
        leaf_rules.insert(data_index.0.clone(), data_index.1.clone());

        // ── 3. build the reference graph over the non-entry HTML pages ────
        // Graph nodes = every *.html except the stable entry. (After the
        // mtg-4irju leaf-ification, NO served *.js references an HTML page, so
        // there are no graph-JS nodes; lobby_launcher.js is a pure leaf hashed
        // in step 2. We still scan *.js defensively and refuse — see below — if
        // one reintroduces an HTML reference, since that would be a NEW edge the
        // pure-DAG model does not expect.)
        let html_pages = discover_html_pages(web_dir)?;
        let mut node_set: BTreeSet<String> = BTreeSet::new();
        for p in &html_pages {
            if !is_stable_html(p) {
                node_set.insert(p.clone());
            }
        }
        let html_nodes: BTreeSet<String> = node_set.clone();
        for entry in std::fs::read_dir(web_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            if !fname.ends_with(".js") {
                continue;
            }
            // A *.js that names an HTML page would re-create the leaf↔page edge
            // the leaf-ification removed. The only such reference today is a
            // doc-comment in test-only helpers (game_boot_params.js), so we DO
            // pull such a file in as a node (it is hashed; harmless if nothing
            // imports it) — keeping the graph correct if a real one is added.
            let src = std::fs::read_to_string(entry.path())?;
            if !all_references(&src, &html_nodes).is_empty() {
                node_set.insert(fname.into_owned());
            }
        }

        // ── 3a. FULL reference graph (module + soft) → assert ACYCLIC ──────
        // The mtg-4irju design makes the web nav graph a strict DAG: forward
        // edges only, every back-edge rerouted through index.html?goto=. A
        // multi-node SCC means someone reintroduced a cycle (e.g. a direct
        // game→game or editor→launcher link), which would resurrect the cache
        // vulnerability the runtime manifest used to paper over. Refuse loudly
        // and name the offending files + the fix.
        let mut full_edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for node in &node_set {
            let src = std::fs::read_to_string(web_dir.join(node))?;
            let refs = all_references(&src, &node_set);
            full_edges.insert(
                node.clone(),
                refs.into_iter().filter(|r| *r != node).map(|s| s.to_string()).collect(),
            );
        }
        let sccs = tarjan_sccs(&node_set, &full_edges);
        if let Some(cycle) = sccs.iter().find(|c| c.len() > 1) {
            let members = cycle.join(", ");
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "asset_graph: dependency CYCLE among [{members}] — the CAS pipeline \
                     (mtg-4irju) requires a strict forward DAG so every reference is \
                     statically hashable. A direct link between these pages forms a cycle. \
                     Reroute the BACK-edge through the stable entry as \
                     index.html?goto=<logical>&release=<token> (the entry's inline \
                     dispatcher resolves it via the content-hashed manifest) instead of \
                     linking the hashed sibling directly."
                ),
            ));
        }

        // ── 3b. hash each node in reverse-topological order ────────────────
        // Tarjan emits SCCs (here all singletons) referents-first, so when a
        // referrer is hashed every name it cites is already in `graph_nodes`
        // and is baked in statically. No logical/runtime indirection remains.
        let hash_order: Vec<String> = sccs.iter().flat_map(|c| c.iter().cloned()).collect();
        let mut graph_nodes: BTreeMap<String, String> = BTreeMap::new();
        for node in &hash_order {
            let path = web_dir.join(node);
            let mut src = std::fs::read_to_string(&path)?;
            src = rewrite_all_references(&src, &leaf_rules);
            src = rewrite_all_references(&src, &graph_nodes);
            std::fs::write(&path, src)?;
            let (k, v) = hash_in_place(web_dir, node)?;
            graph_nodes.insert(k, v);
        }

        // ── 4. build the FULL logical→hashed manifest + content-hash it ────
        // Every hashed asset: pkg pair, JS leaves, data index, HTML pages. Its
        // deterministic JSON is content-hashed → the release token (a Merkle
        // root over the whole release graph). PURELY content-derived: identical
        // web/ content ⇒ identical token (no build SHA / time folded in).
        let mut manifest: BTreeMap<String, String> = BTreeMap::new();
        manifest.insert(
            format!("pkg/{}.js", web_pkg::PKG_JS_STEM),
            format!("pkg/{}", pkg.js_hashed),
        );
        manifest.insert(
            format!("pkg/{}.wasm", web_pkg::PKG_WASM_STEM),
            format!("pkg/{}", pkg.wasm_hashed),
        );
        for (k, v) in &js_leaves {
            manifest.insert(k.clone(), v.clone());
        }
        manifest.insert(data_index.0.clone(), data_index.1.clone());
        for (k, v) in &graph_nodes {
            manifest.insert(k.clone(), v.clone());
        }
        let manifest_json = render_manifest_json(&manifest);
        let release_token = asset_hash_hex(manifest_json.as_bytes());
        let manifest_file = manifest_name(&release_token);
        std::fs::write(web_dir.join(&manifest_file), &manifest_json)?;

        // ── 5. entry page: static forward-link rewrite + bake the token ────
        // index.html's forward refs (<a href="launcher.html">, the JS redirect
        // builders) all point at now-hashed targets → statically rewritten.
        // Then the release token replaces the RELEASE_TOKEN_PLACEHOLDER sentinel
        // so the entry can (a) thread release=<token> onto its forward links and
        // (b) resolve ?goto= back-edges against asset-manifest.<token>.json. The
        // token lives ONLY here (the sole mutable file) — never in a hashed page.
        let entry_path = web_dir.join(ENTRY_HTML);
        if entry_path.is_file() {
            let src = std::fs::read_to_string(&entry_path)?;
            let mut entry_rules = leaf_rules.clone();
            for (k, v) in &graph_nodes {
                entry_rules.insert(k.clone(), v.clone());
            }
            let mut rewritten = rewrite_all_references(&src, &entry_rules);
            if !rewritten.contains(RELEASE_TOKEN_PLACEHOLDER) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "asset_graph: {ENTRY_HTML} is missing the {RELEASE_TOKEN_PLACEHOLDER} \
                         sentinel — the release token cannot be baked in. The entry must carry \
                         the placeholder so its forward links + ?goto dispatcher know the release."
                    ),
                ));
            }
            rewritten = rewritten.replace(RELEASE_TOKEN_PLACEHOLDER, &release_token);
            std::fs::write(&entry_path, rewritten)?;
        }

        Ok(GraphHashResult {
            pkg,
            js_leaves,
            data_index,
            graph_nodes,
            manifest,
            release_token,
            manifest_file,
        })
    }

    /// Render the FULL `logical → hashed` manifest as deterministic JSON
    /// (sorted keys via `BTreeMap`, JSON-string-escaped). This exact byte
    /// sequence is what [`hash_full_graph`] content-hashes to produce the
    /// release token, so it MUST be reproducible: same map ⇒ same bytes ⇒ same
    /// token. Empty map → `{}\n`.
    fn render_manifest_json(manifest: &BTreeMap<String, String>) -> String {
        if manifest.is_empty() {
            return "{}\n".to_string();
        }
        let mut json = String::from("{\n");
        for (i, (k, v)) in manifest.iter().enumerate() {
            let comma = if i + 1 < manifest.len() { "," } else { "" };
            json.push_str(&format!("  {}: {}{}\n", json_string(k), json_string(v), comma));
        }
        json.push_str("}\n");
        json
    }

    /// Minimal JSON string encoder for our ASCII filename tokens (escapes `"`
    /// and `\\`; filenames never contain control chars). Keeps the manifest
    /// emission structured rather than `format!`-with-raw-quotes.
    fn json_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }

    /// Test-only re-export of [`rewrite_all_references`].
    #[doc(hidden)]
    pub fn __test_rewrite_all(src: &str, rules: &BTreeMap<String, String>) -> String {
        rewrite_all_references(src, rules)
    }

    /// Test-only re-export of [`tarjan_sccs`].
    #[doc(hidden)]
    pub fn __test_sccs(nodes: &BTreeSet<String>, edges: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
        tarjan_sccs(nodes, edges)
    }

    /// Convenience: list every file path (relative to `web_dir`) the hasher
    /// hashes that is KNOWN up front (pure leaves + data). HTML pages and
    /// graph-JS are auto-discovered at run time, so they are not listed here.
    pub fn declared_assets() -> Vec<PathBuf> {
        let mut v = Vec::new();
        v.push(PathBuf::from(DATA_INDEX_JSON));
        for s in HASHED_JS_LEAVES {
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
            // Mimic the inline-JS pattern from index.html.
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
        fn graph_rewrite_handles_data_asset_href_attribute() {
            // The cycle-break loader rewrites <a data-asset-href="..."> anchors
            // at runtime, but the deploy hasher must ALSO be able to rewrite a
            // data-asset-href value that names a non-cycle (DAG) page.
            use crate::asset_hash::asset_graph;
            let mut rules = std::collections::BTreeMap::new();
            rules.insert(
                "native_game.html".to_string(),
                "native_game.0123456789abcdef.html".to_string(),
            );
            let src = r#"<a href="native_game.html" data-asset-href="native_game.html">GUI</a>"#;
            let out = asset_graph::__test_rewrite_all(src, &rules);
            assert!(out.contains(r#"href="native_game.0123456789abcdef.html""#));
            assert!(out.contains(r#"data-asset-href="native_game.0123456789abcdef.html""#));
        }

        #[test]
        fn tarjan_finds_the_game_page_cycle() {
            use crate::asset_hash::asset_graph;
            use std::collections::{BTreeMap, BTreeSet};
            // Model the real topology: tui ⇄ native ⇄ lobby_launcher.js cycle,
            // demo → {tui, native} (DAG into the cycle), deck_editor leaf.
            let nodes: BTreeSet<String> = [
                "tui_game.html",
                "native_game.html",
                "lobby_launcher.js",
                "demo.html",
                "deck_editor.html",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            edges.insert(
                "tui_game.html".into(),
                ["native_game.html", "lobby_launcher.js"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
            edges.insert(
                "native_game.html".into(),
                ["tui_game.html", "lobby_launcher.js"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
            edges.insert(
                "lobby_launcher.js".into(),
                ["tui_game.html", "native_game.html"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
            edges.insert(
                "demo.html".into(),
                ["tui_game.html", "native_game.html"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
            edges.insert("deck_editor.html".into(), BTreeSet::new());
            let sccs = asset_graph::__test_sccs(&nodes, &edges);
            // Exactly one multi-node SCC: {tui, native, lobby_launcher.js}.
            let cycle: Vec<&Vec<String>> = sccs.iter().filter(|c| c.len() > 1).collect();
            assert_eq!(cycle.len(), 1, "exactly one cycle SCC");
            let mut got = cycle[0].clone();
            got.sort();
            assert_eq!(
                got,
                vec![
                    "lobby_launcher.js".to_string(),
                    "native_game.html".to_string(),
                    "tui_game.html".to_string()
                ]
            );
            // The cycle must appear BEFORE its dependents (demo) in reverse-topo
            // order — i.e. demo's component index > the cycle's.
            let pos = |name: &str| sccs.iter().position(|c| c.contains(&name.to_string())).unwrap();
            assert!(pos("tui_game.html") < pos("demo.html"), "cycle hashed before demo");
        }

        #[test]
        fn does_not_touch_unrelated_init_identifiers() {
            let src = "tui_init_logging();";
            let out = rewrite_html(src, JS_HASHED, WASM_HASHED);
            assert_eq!(out, src, "non-empty-paren / unrelated calls untouched");
        }
    }

    /// End-to-end tests for the CAS pure-DAG pipeline (mtg-4irju): the
    /// content-hashed immutable manifest, the baked release token, and the
    /// acyclicity guard. These stage a minimal web tree in a tempdir and run
    /// the real [`asset_graph::hash_full_graph`].
    mod full_graph {
        use crate::asset_hash::{asset_graph, asset_hash_hex};
        use std::path::Path;

        /// Write `(relpath, content)` pairs into `dir`, creating parent dirs.
        fn write_tree(dir: &Path, files: &[(&str, &str)]) {
            for (rel, content) in files {
                let path = dir.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&path, content).unwrap();
            }
        }

        /// A minimal but representative pure-DAG web tree:
        ///   index.html (entry) → launcher/native/tui/deck_editor
        ///   launcher.html → {native, tui, deck_editor}
        ///   native/tui → import lobby_launcher.js (leaf); back-edge → index
        ///   deck_editor → index.html?goto=launcher (back-edge via dispatcher)
        /// No cycles: no native⇄tui, lobby_launcher.js references no page.
        fn dag_tree() -> Vec<(&'static str, &'static str)> {
            vec![
                ("pkg/mtg_engine.js", "export default function init(){}\nawait init();\n"),
                ("pkg/mtg_engine_bg.wasm", "\0\0wasm-bytes\0\0"),
                ("data/sets/index.json", "{\"sets\":[]}\n"),
                ("server-config.js", "window.MTG_WS_URL='';\n"),
                ("network.js", "export const x=1;\n"),
                // Leaf: param-plumbing only, references NO html page.
                (
                    "lobby_launcher.js",
                    "export function buildRedirectQuery(o){return o;}\n",
                ),
                (
                    "index.html",
                    "<!doctype html><html><head>\
                     <script>const MTG_RELEASE_TOKEN='__MTG_RELEASE_TOKEN__';</script>\
                     </head><body>\
                     <a id=l href=\"launcher.html\">L</a>\
                     <a id=n href=\"native_game.html\">N</a>\
                     <a id=t href=\"tui_game.html\">T</a>\
                     <a id=d href=\"deck_editor.html\">D</a>\
                     </body></html>\n",
                ),
                (
                    "launcher.html",
                    "<!doctype html><html><body>\
                     <a href=\"deck_editor.html\">edit</a>\
                     <script type=module>import './lobby_launcher.js';\
                     const P={tui:'tui_game.html',native:'native_game.html'};</script>\
                     </body></html>\n",
                ),
                (
                    "native_game.html",
                    "<!doctype html><html><body><a href=\"index.html\">back</a>\
                     <script type=module>import './lobby_launcher.js';\
                     import init from './pkg/mtg_engine.js'; await init();</script>\
                     </body></html>\n",
                ),
                (
                    "tui_game.html",
                    "<!doctype html><html><body><a href=\"index.html\">back</a>\
                     <script type=module>import './lobby_launcher.js';</script>\
                     </body></html>\n",
                ),
                (
                    "deck_editor.html",
                    "<!doctype html><html><body>\
                     <a href=\"index.html?goto=launcher\">Back to Launcher</a>\
                     <a href=\"index.html\">Back to Lobby</a></body></html>\n",
                ),
            ]
        }

        #[test]
        fn pure_dag_bakes_token_and_writes_immutable_manifest() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path();
            write_tree(dir, &dag_tree());

            let res = asset_graph::hash_full_graph(dir).expect("pure DAG must hash cleanly");

            // (1) Only index.html stays unhashed; the other pages are renamed.
            assert!(dir.join("index.html").is_file(), "entry stays unhashed");
            assert!(!dir.join("launcher.html").is_file(), "launcher.html renamed away");

            // (2) NO stable-named manifest/loader survives — only the hashed one.
            assert!(!dir.join("asset_manifest.js").exists(), "stable loader must be gone");
            assert!(
                !dir.join("asset-manifest.json").exists(),
                "stable manifest must be gone"
            );
            let manifest_path = dir.join(&res.manifest_file);
            assert!(manifest_path.is_file(), "asset-manifest.<token>.json written");
            assert_eq!(
                res.manifest_file,
                format!("asset-manifest.{}.json", res.release_token),
                "manifest filename embeds the token"
            );

            // (3) The token IS the content hash of the served manifest bytes
            //     (the Merkle-root / immutability property).
            let manifest_bytes = std::fs::read(&manifest_path).unwrap();
            assert_eq!(
                asset_hash_hex(&manifest_bytes),
                res.release_token,
                "token == blake3(manifest content) — self-consistent immutable name"
            );

            // (4) The token is baked into index.html (placeholder consumed).
            let index = std::fs::read_to_string(dir.join("index.html")).unwrap();
            assert!(!index.contains("__MTG_RELEASE_TOKEN__"), "placeholder replaced");
            assert!(index.contains(&res.release_token), "real token baked into entry");

            // (5) index.html forward links are statically rewritten to hashed names.
            let launcher_hashed = &res.graph_nodes["launcher.html"];
            assert!(
                index.contains(launcher_hashed.as_str()),
                "entry forward-links the hashed launcher"
            );

            // (6) The manifest resolves every back-edge target + every asset class.
            for logical in ["launcher.html", "native_game.html", "tui_game.html", "deck_editor.html"] {
                assert!(res.manifest.contains_key(logical), "manifest maps {logical}");
            }
            assert!(res.manifest.contains_key("pkg/mtg_engine.js"), "manifest pins pkg js");
            assert!(
                res.manifest.contains_key("pkg/mtg_engine_bg.wasm"),
                "manifest pins wasm"
            );
            assert!(res.manifest.contains_key("data/sets/index.json"), "manifest pins data");
            assert!(res.manifest.contains_key("lobby_launcher.js"), "manifest pins the leaf");
        }

        #[test]
        fn token_is_purely_content_derived_reproducible() {
            // Identical content in two independent trees ⇒ identical token.
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            write_tree(a.path(), &dag_tree());
            write_tree(b.path(), &dag_tree());
            let ra = asset_graph::hash_full_graph(a.path()).unwrap();
            let rb = asset_graph::hash_full_graph(b.path()).unwrap();
            assert_eq!(ra.release_token, rb.release_token, "same content ⇒ same Merkle root");
            assert_eq!(ra.manifest, rb.manifest, "same content ⇒ same logical→hashed map");
        }

        #[test]
        fn reintroduced_cycle_is_a_hard_error() {
            // Two pages that reference each OTHER form a cycle; the pure-DAG
            // pipeline must refuse rather than silently fall back to a runtime
            // manifest (which would resurrect the cache vulnerability).
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path();
            write_tree(
                dir,
                &[
                    ("pkg/mtg_engine.js", "function init(){}\ninit();\n"),
                    ("pkg/mtg_engine_bg.wasm", "wasm"),
                    ("data/sets/index.json", "{}\n"),
                    ("index.html", "<a href=\"a.html\">__MTG_RELEASE_TOKEN__</a>\n"),
                    ("a.html", "<a href=\"b.html\">a</a>\n"),
                    ("b.html", "<a href=\"a.html\">b</a>\n"),
                ],
            );
            let err = asset_graph::hash_full_graph(dir).expect_err("a⇄b cycle must error");
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            let msg = err.to_string();
            assert!(msg.contains("CYCLE"), "names the cycle: {msg}");
            assert!(
                msg.contains("a.html") && msg.contains("b.html"),
                "names the members: {msg}"
            );
            assert!(msg.contains("goto="), "points to the index.html?goto= fix: {msg}");
        }

        #[test]
        fn missing_release_token_placeholder_is_a_hard_error() {
            // index.html without the sentinel can't carry the token → refuse.
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path();
            write_tree(
                dir,
                &[
                    ("pkg/mtg_engine.js", "function init(){}\ninit();\n"),
                    ("pkg/mtg_engine_bg.wasm", "wasm"),
                    ("data/sets/index.json", "{}\n"),
                    ("index.html", "<a href=\"native_game.html\">no token here</a>\n"),
                    ("native_game.html", "<a href=\"index.html\">back</a>\n"),
                ],
            );
            let err = asset_graph::hash_full_graph(dir).expect_err("missing placeholder must error");
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("__MTG_RELEASE_TOKEN__"), "names the sentinel");
        }
    }
}
