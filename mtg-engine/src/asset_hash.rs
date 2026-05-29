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
