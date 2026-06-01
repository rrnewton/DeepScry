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

/// Full asset-graph content-addresser (mtg-620, generalized mtg-682).
///
/// A GENERAL, graph-aware renamer for the deploy-staging web tree. It hashes
/// every web asset to an immutable `<stem>.<blake3>.<ext>` name and rewrites
/// every reference to point at the hashed target — EXCEPT the one stable
/// bootstrap entry ([`ENTRY_HTML`] = `index.html`, the server's default URL)
/// and the stable runtime resolver ([`MANIFEST_JS`] = `asset_manifest.js`).
///
/// ### Why a graph (and not a hardcoded list)
///
/// The earlier version hardcoded the HTML page set (`HASHED_HTML_PAGES`) and
/// broke when the lobby-redo added `launcher.html` (never added to the list →
/// served fixed-name → 404 on deploy). This version **auto-discovers** every
/// `*.html` in `web_dir` (sorted → deterministic), so a new page is hashed
/// automatically. It also builds the actual reference graph across HTML pages,
/// JS leaves, and the data index, and rewrites every reference to its real
/// hashed target — instead of the old "blanket-redirect all cross-page links
/// to `index.html`" hack (which flattened genuine forward navigation).
///
/// ### Topology + cycle handling (the general mechanism)
///
/// Most of the graph is a DAG: `index.html → launcher/game/editor pages`,
/// `launcher.html → deck_editor.html`, `demo.html → game pages`, and the pure
/// leaves (pkg, `network.js`, `server-config.js`, the data index, …) that
/// reference nothing. DAG edges are resolved by **topological hashing**: hash
/// referents before referrers (reverse-topological order over the condensed
/// graph), so when a referrer is hashed every name it cites is already known
/// and is baked in statically.
///
/// But some assets reference each OTHER (a true cycle): `tui_game.html` ⇄
/// `native_game.html` (mutual renderer-switch nav) and the game pages ⇄
/// `lobby_launcher.js` (pages import it; it names the pages via `GAME_PAGE`).
/// A cycle CANNOT be resolved by static topological hashing — a member's hash
/// depends on a co-member's not-yet-computed hash. We break every such cycle
/// the GENERAL way (not the old entry-only hack): intra-cycle references
/// resolve through a **served runtime manifest** (`asset-manifest.json`) and
/// a tiny **stable-named loader** (`asset_manifest.js`, never hashed, so any
/// page can import it). The loader's `MANIFEST` literal is rewritten in place
/// with the real `logical → hashed` mapping; `resolveAsset()` looks names up
/// at runtime and `installManifestHrefRewrite()` fixes up `[data-asset-href]`
/// anchors. On the un-hashed source tree the mapping is empty so the resolver
/// is the identity — dev and deploy share ONE code path.
///
/// Concretely the algorithm:
///   1. Hash the pkg pair (delegated to [`web_pkg::hash_web_assets`]).
///   2. Hash the JS leaves + the data index (pure leaves: no graph edges).
///   3. Build the reference graph over the remaining HTML pages (+ any JS
///      that references HTML, e.g. `lobby_launcher.js`), find its strongly
///      connected components (Tarjan), and hash the condensation in
///      reverse-topological order. While hashing a component, references to
///      EARLIER components (already hashed) are statically rewritten;
///      references WITHIN the same component (cycle edges) are left logical
///      (the manifest resolves them at runtime).
///   4. Rewrite the stable entry (`index.html`) statically — every name it
///      cites is hashed by now — without renaming it.
///   5. Write `asset-manifest.json` and rewrite `asset_manifest.js`'s
///      `MANIFEST` literal with the full `logical → hashed` map.
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
    /// content is rewritten (to point at hashed dependencies). This is the
    /// server's default URL, so it must stay stable.
    pub const ENTRY_HTML: &str = "index.html";

    /// The stable-named runtime manifest LOADER (an ES module). Never hashed:
    /// it is the bootstrap resolver that other (hashed) assets import to look
    /// up cycle-edge targets, so its name must stay stable. Its `MANIFEST`
    /// literal is rewritten in place with the `logical → hashed` map.
    pub const MANIFEST_JS: &str = "asset_manifest.js";

    /// The served JSON manifest (`logical → hashed`). Written fresh each run.
    /// Stable-named (a debugging/inspection artifact; the JS loader carries the
    /// authoritative map inline so no extra fetch is on the critical path).
    pub const MANIFEST_JSON: &str = "asset-manifest.json";

    /// Stable assets that participate in the graph but are NEVER renamed:
    /// the entry page and the manifest loader.
    fn is_stable_html(name: &str) -> bool {
        name == ENTRY_HTML
    }

    /// JS leaves loaded by `<script src>` or ES `import` that reference NO
    /// other hashable HTML (pure leaves). Auto-discovery (step 3) treats any
    /// `.js` that DOES reference HTML as a graph node instead. These are hashed
    /// up front in step 2. (`asset_manifest.js` is excluded — it stays stable.)
    pub const HASHED_JS_LEAVES: &[&str] = &["server-config.js", "network.js", "bug_report.js", "help_dialog.js"];

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
        /// Auto-discovered HTML pages (besides the entry) and any JS that
        /// references HTML (e.g. `lobby_launcher.js`): logical → hashed.
        pub graph_nodes: BTreeMap<String, String>,
        /// `logical → hashed` entries placed in the runtime manifest because
        /// they are referenced across a dependency cycle (cannot be statically
        /// resolved). A subset of `graph_nodes`.
        pub manifest: BTreeMap<String, String>,
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

    /// Scan `src` for the **module-load edges** to candidate `names`: the
    /// references that the BROWSER resolves at load time and that therefore
    /// MUST be statically rewritten to a real hashed name (they cannot be
    /// indirected through the runtime manifest). These are exactly:
    ///
    ///   - ES static import:   `import ... from '<name>'`
    ///   - ES dynamic import:  `import('<name>')`  /  `await import('<name>')`
    ///   - classic script tag: `<script src="<name>">`
    ///
    /// Plain navigation references (`<a href>`, `<a data-asset-href>`) and bare
    /// JS string literals (`GAME_PAGE = { tui: 'tui_game.html' }`,
    /// `window.location.href = 'launcher.html?...'`) are deliberately NOT
    /// module edges: they are resolved at runtime via `asset_manifest.js` when
    /// they point at a not-yet-hashed (cycle) target, or statically rewritten
    /// when their target is already hashed. Keeping them OUT of the dependency
    /// graph is what turns the apparent `tui ⇄ native ⇄ lobby_launcher.js`
    /// cycle into a DAG: the only module edge in that trio is
    /// `game pages → lobby_launcher.js` (a one-way import), so the launcher
    /// hashes first and the pages bake its hashed import in.
    ///
    /// Returns the matched subset (the graph successors of `src`).
    fn module_edges<'a>(src: &str, names: &'a BTreeSet<String>) -> BTreeSet<&'a str> {
        let mut out = BTreeSet::new();
        for name in names {
            let n = regex::escape(name);
            // `from '<name>'` / `from "<name>"` (static import) OR
            // `import('<name>')` (dynamic) OR `src="<name>"` (script tag).
            // Leading `./` or `/` optional; the name is bounded by the quote
            // so a longer name cannot match a shorter as a substring.
            let pat = format!(r#"(?:from\s*|import\s*\(\s*|<script[^>]*\bsrc\s*=\s*)['"](?:\./|/)?{n}['"]"#,);
            let re = Regex::new(&pat).expect("asset_graph module-edge scan: regex compiles");
            if re.is_match(src) {
                out.insert(name.as_str());
            }
        }
        out
    }

    /// Concatenate the text of every served `*.html` / `*.js` file at the top
    /// level of `web_dir` EXCEPT the stable manifest loader (`asset_manifest.js`,
    /// whose doc-comment examples must not produce false "still referenced"
    /// hits). Used to detect which logical asset names survived the static
    /// rewrite as soft cycle edges → the runtime manifest set.
    fn read_all_served_text(web_dir: &Path) -> std::io::Result<String> {
        let mut blob = String::new();
        for entry in std::fs::read_dir(web_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == MANIFEST_JS {
                continue;
            }
            if name.ends_with(".html") || name.ends_with(".js") {
                blob.push_str(&std::fs::read_to_string(entry.path())?);
                blob.push('\n');
            }
        }
        Ok(blob)
    }

    /// Scan `src` for ALL references (module edges + soft refs: href,
    /// data-asset-href, bare JS string literals) to candidate `names`. Used to
    /// build the FULL reference graph whose multi-node SCCs identify genuine
    /// dependency cycles (the manifest-resolved set). Same filename-token
    /// boundary rules as [`rewrite_one_reference`].
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

        // ── 3. build the MODULE-EDGE graph over the remaining HTML + graph-JS ─
        // Graph nodes = every *.html except the stable entry, plus any *.js
        // still on disk (un-hashed by step 2, not the stable manifest loader)
        // that references an HTML node by ANY form — i.e. `lobby_launcher.js`
        // (it names the game pages via GAME_PAGE string literals). The manifest
        // loader (`asset_manifest.js`) is excluded: it stays stable.
        let html_pages = discover_html_pages(web_dir)?;
        let mut node_set: BTreeSet<String> = BTreeSet::new();
        for p in &html_pages {
            if !is_stable_html(p) {
                node_set.insert(p.clone());
            }
        }
        // Snapshot the HTML node names so the graph-JS scan can ask "does this
        // .js reference any HTML page?" (any reference form qualifies it as a
        // node — it must be hashed; its module/soft edges are classified next).
        let html_nodes: BTreeSet<String> = node_set.clone();
        for entry in std::fs::read_dir(web_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            if !fname.ends_with(".js") || fname == MANIFEST_JS {
                continue;
            }
            let src = std::fs::read_to_string(entry.path())?;
            // Any reference form (module OR soft string literal) qualifies the
            // .js as a graph node that must itself be hashed.
            if !all_references(&src, &html_nodes).is_empty() {
                node_set.insert(fname.into_owned());
            }
        }

        // Build adjacency using MODULE EDGES ONLY (import / script-src). Soft
        // references (href / data-asset-href / bare string literals) are NOT
        // edges — they are resolved at runtime via the manifest when they would
        // cycle. This makes the apparent tui⇄native⇄lobby_launcher cycle a DAG
        // (only edge: game pages → lobby_launcher.js).
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for node in &node_set {
            let src = std::fs::read_to_string(web_dir.join(node))?;
            let refs = module_edges(&src, &node_set);
            edges.insert(
                node.clone(),
                refs.into_iter()
                    .filter(|r| *r != node) // ignore self-edges
                    .map(|s| s.to_string())
                    .collect(),
            );
        }

        // Module-edge SCCs: each must be a single node (a multi-node module
        // SCC is a genuine ES-import cycle that NO runtime manifest can break —
        // the browser resolves imports at load). Refuse it loudly. The result
        // is also our module-topological order (referents before referrers).
        let module_sccs = tarjan_sccs(&node_set, &edges);
        if let Some(cycle) = module_sccs.iter().find(|c| c.len() > 1) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "asset_graph: unbreakable MODULE-import cycle among {cycle:?} — \
                     ES imports / <script src> resolve at load and cannot be \
                     indirected through the runtime manifest. Route one edge of \
                     the cycle through asset_manifest.js (a soft reference) instead."
                ),
            ));
        }

        // ── 3b. FULL-reference graph (module + soft) → label CYCLE members ─
        // A node is a "cycle member" iff it sits in a multi-node SCC of the
        // FULL reference graph (e.g. tui ⇄ native ⇄ lobby_launcher.js). Such
        // members CANNOT statically resolve their intra-cycle refs, so those go
        // through the runtime manifest. Non-cycle nodes (demo, deck_editor,
        // launcher, …) reference cycle members only via FORWARD soft edges, so
        // we hash the cycle members FIRST and the non-cycle referrers can then
        // bake the (now-known) hashed cycle names in statically.
        let mut full_edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for node in &node_set {
            let src = std::fs::read_to_string(web_dir.join(node))?;
            let refs = all_references(&src, &node_set);
            full_edges.insert(
                node.clone(),
                refs.into_iter().filter(|r| *r != node).map(|s| s.to_string()).collect(),
            );
        }
        let full_sccs = tarjan_sccs(&node_set, &full_edges);
        let cycle_members: BTreeSet<String> = full_sccs
            .iter()
            .filter(|c| c.len() > 1)
            .flat_map(|c| c.iter().cloned())
            .collect();

        // ── 3c. hash order: cycle members FIRST, then non-cycle nodes; both
        //         in module-topological order (so each node's MODULE imports are
        //         already hashed when it is processed). module_sccs is already
        //         in module-topo order; partition it preserving that order.
        let module_topo: Vec<String> = module_sccs.iter().flat_map(|c| c.iter().cloned()).collect();
        let mut hash_order: Vec<String> = Vec::with_capacity(module_topo.len());
        for n in &module_topo {
            if cycle_members.contains(n) {
                hash_order.push(n.clone());
            }
        }
        for n in &module_topo {
            if !cycle_members.contains(n) {
                hash_order.push(n.clone());
            }
        }

        // ── 3d. hash each node in that order ───────────────────────────────
        // `graph_nodes` accumulates logical→hashed as nodes finish. Static
        // rewrite resolves every ref whose target is already hashed; an
        // intra-cycle ref to a not-yet-hashed co-member is left LOGICAL on
        // purpose (collected into the manifest in step 5, resolved at runtime
        // by asset_manifest.js).
        let mut graph_nodes: BTreeMap<String, String> = BTreeMap::new();
        for node in &hash_order {
            let path = web_dir.join(node);
            let mut src = std::fs::read_to_string(&path)?;
            src = rewrite_all_references(&src, &leaf_rules);
            let static_rules: BTreeMap<String, String> =
                graph_nodes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            src = rewrite_all_references(&src, &static_rules);
            std::fs::write(&path, src)?;
            let (k, v) = hash_in_place(web_dir, node)?;
            graph_nodes.insert(k, v);
        }

        // ── 4. entry page: static rewrite (every dep is hashed now) ───────
        // index.html's soft refs (<a href="tui_game.html">, the JS
        // window.location redirect builders) all point at now-hashed targets,
        // so they are statically rewritten here — they are NOT cycle edges
        // (nothing imports index.html), so they never go through the manifest.
        let entry_path = web_dir.join(ENTRY_HTML);
        if entry_path.is_file() {
            let src = std::fs::read_to_string(&entry_path)?;
            let mut entry_rules = leaf_rules.clone();
            for (k, v) in &graph_nodes {
                entry_rules.insert(k.clone(), v.clone());
            }
            let rewritten = rewrite_all_references(&src, &entry_rules);
            if rewritten != src {
                std::fs::write(&entry_path, rewritten)?;
            }
        }

        // ── 5. manifest = nodes whose LOGICAL name STILL appears in any served
        //        file (a soft cycle edge we deliberately left un-rewritten —
        //        e.g. lobby_launcher.js's GAME_PAGE literal, the game pages'
        //        data-asset-href nav). These are exactly the references
        //        asset_manifest.js must resolve at runtime. Computed AFTER the
        //        entry rewrite so index.html's (statically resolved) soft refs
        //        do not pollute the manifest. Then write the manifest (JSON +
        //        the JS loader literal).
        let mut manifest: BTreeMap<String, String> = BTreeMap::new();
        let served_blob = read_all_served_text(web_dir)?;
        let logical_names: BTreeSet<String> = graph_nodes.keys().cloned().collect();
        let still_referenced = all_references(&served_blob, &logical_names);
        for logical in still_referenced {
            if let Some(hashed) = graph_nodes.get(logical) {
                manifest.insert(logical.to_string(), hashed.clone());
            }
        }
        write_manifest(web_dir, &manifest)?;

        Ok(GraphHashResult {
            pkg,
            js_leaves,
            data_index,
            graph_nodes,
            manifest,
        })
    }

    /// Serialize the cycle-edge `logical → hashed` map into BOTH the served
    /// `asset-manifest.json` and the inline `MANIFEST = { ... }` literal of the
    /// stable `asset_manifest.js` loader. The literal is replaced by a
    /// STRUCTURED splice anchored on the `/* @@ASSET_MANIFEST@@ */` marker
    /// comment + the `export const MANIFEST = {...};` statement — NOT a blind
    /// substring munge of arbitrary JS.
    ///
    /// If `asset_manifest.js` is absent (e.g. a partial staging tree without
    /// the loader) this is a no-op for the JS side; the JSON is still written.
    fn write_manifest(web_dir: &Path, manifest: &BTreeMap<String, String>) -> std::io::Result<()> {
        // Deterministic JSON (sorted keys via BTreeMap).
        let mut json = String::from("{\n");
        for (i, (k, v)) in manifest.iter().enumerate() {
            let comma = if i + 1 < manifest.len() { "," } else { "" };
            json.push_str(&format!("  {}: {}{}\n", json_string(k), json_string(v), comma));
        }
        json.push_str("}\n");
        std::fs::write(web_dir.join(MANIFEST_JSON), &json)?;

        // The JS loader literal.
        let loader_path = web_dir.join(MANIFEST_JS);
        if !loader_path.is_file() {
            return Ok(());
        }
        let src = std::fs::read_to_string(&loader_path)?;
        let object_body = manifest_object_literal(manifest);
        let new_decl = format!("export const MANIFEST = {object_body};");
        // Anchor: the marker comment is immediately followed by the export.
        // We replace exactly the `export const MANIFEST = {...};` statement
        // that follows the marker. The marker guarantees we splice the right
        // declaration even if the file later grows other `MANIFEST` mentions.
        let marker = "/* @@ASSET_MANIFEST@@ */";
        let Some(marker_pos) = src.find(marker) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("asset_graph: {MANIFEST_JS} missing the {marker} marker"),
            ));
        };
        let after_marker = marker_pos + marker.len();
        // The declaration starts at the next `export const MANIFEST`.
        let decl_start_rel = src[after_marker..].find("export const MANIFEST").ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("asset_graph: {MANIFEST_JS} missing 'export const MANIFEST' after marker"),
            )
        })?;
        let decl_start = after_marker + decl_start_rel;
        // It ends at the first `};` that closes the object literal. The literal
        // is authored as a single statement ending in `};` (empty `{}` ends in
        // `{};`), so the first `;` after `decl_start` terminates it.
        let semi_rel = src[decl_start..].find(';').ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("asset_graph: {MANIFEST_JS} MANIFEST declaration not terminated"),
            )
        })?;
        let decl_end = decl_start + semi_rel + 1;
        let mut out = String::with_capacity(src.len() + new_decl.len());
        out.push_str(&src[..decl_start]);
        out.push_str(&new_decl);
        out.push_str(&src[decl_end..]);
        std::fs::write(&loader_path, out)?;
        Ok(())
    }

    /// Render a JS object literal `{ "a": "b", "c": "d" }` from the map
    /// (sorted, JSON-string-escaped keys/values). Empty map → `{}`.
    fn manifest_object_literal(manifest: &BTreeMap<String, String>) -> String {
        if manifest.is_empty() {
            return "{}".to_string();
        }
        let mut s = String::from("{\n");
        for (i, (k, v)) in manifest.iter().enumerate() {
            let comma = if i + 1 < manifest.len() { "," } else { "" };
            s.push_str(&format!("    {}: {}{}\n", json_string(k), json_string(v), comma));
        }
        s.push('}');
        s
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
}
