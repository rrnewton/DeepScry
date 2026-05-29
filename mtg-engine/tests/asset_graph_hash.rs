//! Integration test for `asset_hash::asset_graph::hash_full_graph` (mtg-620).
//!
//! Builds a minimal synthetic `web_dir` on a tempdir — small fake `pkg/`,
//! `data/sets/index.json`, and every HTML page named in
//! `HASHED_HTML_PAGES` plus the `ENTRY_HTML` — runs the full-graph
//! hasher, and asserts the structural invariant:
//!
//!   - `index.html` keeps its name (the sole stable URL).
//!   - Every other declared asset is renamed to `<stem>.<16-hex>.<ext>`.
//!   - References inside the hashed tree point at the hashed names
//!     (query strings preserved; cross-page nav broken to `index.html`).
//!
//! This is the unit-level analogue of the JS smoke test (which runs the
//! real server + browser-style HTTP fetches); it catches rewriter bugs
//! without needing a real `mtg server-web` process.

use mtg_engine::asset_hash::asset_graph::{self, ENTRY_HTML, HASHED_HTML_PAGES, HASHED_JS_LEAVES};
use std::fs;
use std::path::Path;

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn is_hashed_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    let hash = parts[parts.len() - 2];
    hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn full_graph_hashes_everything_but_index_html() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path();

    // Fake pkg pair (matches the web_pkg::hash_web_assets requirements).
    write(
        &web.join("pkg/mtg_engine.js"),
        "// fake glue\nexport default function init() {}",
    );
    write(&web.join("pkg/mtg_engine_bg.wasm"), "fake wasm bytes");

    // Data leaf (set resolver).
    write(&web.join("data/sets/index.json"), r#"{"sets":[]}"#);

    // JS leaves.
    write(&web.join("server-config.js"), "window.MTG_WS_URL = 'ws://x';");
    write(&web.join("network.js"), "// network module");
    write(&web.join("bug_report.js"), "// bug report module");

    // Non-entry HTML pages. Include the patterns the rewriter must handle:
    // <a href="other.html"> nav, fetch('./data/sets/index.json'),
    // import './network.js', <script src="server-config.js">, and the
    // query-preserving 'tui_game.html?...' redirect builder.
    write(
        &web.join("native_game.html"),
        r#"<html><head><script src="server-config.js"></script><script src="bug_report.js"></script></head>
<body><a href="tui_game.html">TUI</a> <a href="index.html">lobby</a>
<script>fetch('./data/sets/index.json');</script></body></html>"#,
    );
    write(
        &web.join("tui_game.html"),
        r#"<html><head><script src="server-config.js"></script></head>
<body><a href="native_game.html">GUI</a> <a href="index.html">lobby</a>
<script type="module">import './network.js'; import './bug_report.js'; fetch('/data/sets/index.json');</script></body></html>"#,
    );
    write(
        &web.join("demo.html"),
        r#"<html><body><a href="index.html">lobby</a><script>fetch('./data/sets/index.json');</script></body></html>"#,
    );
    write(
        &web.join("wasm_ai_harness.html"),
        r#"<html><body><script>fetch('/data/sets/index.json');</script></body></html>"#,
    );

    // ENTRY: launch buttons + JS redirect with query string.
    write(
        &web.join("index.html"),
        r#"<html><head><script src="server-config.js"></script></head>
<body>
<a id="launch-native" href="native_game.html">GUI</a>
<a id="launch-tui" href="tui_game.html">TUI</a>
<a id="launch-demo" href="demo.html">demo</a>
<script>
  document.getElementById('launch-tui').href = 'tui_game.html' + suffix;
  window.location.href = 'tui_game.html?' + qp.toString();
</script>
</body></html>"#,
    );

    // Run it.
    let res = asset_graph::hash_full_graph(web).expect("hash_full_graph");

    // Every JS leaf renamed.
    for leaf in HASHED_JS_LEAVES {
        let hashed = res
            .js_leaves
            .get(*leaf)
            .unwrap_or_else(|| panic!("missing leaf {leaf}"));
        assert!(is_hashed_name(hashed), "{leaf} -> {hashed} should be hashed");
        assert!(web.join(hashed).is_file(), "{hashed} exists on disk");
        assert!(!web.join(leaf).exists(), "{leaf} should be renamed away");
    }

    // Data index renamed.
    let (orig, hashed) = &res.data_index;
    assert_eq!(orig, "data/sets/index.json");
    assert!(is_hashed_name(hashed.split('/').next_back().unwrap()));
    assert!(web.join(hashed).is_file());
    assert!(!web.join("data/sets/index.json").exists());

    // Every non-entry HTML page renamed.
    for page in HASHED_HTML_PAGES {
        let hashed = res
            .html_pages
            .get(*page)
            .unwrap_or_else(|| panic!("missing html {page}"));
        assert!(is_hashed_name(hashed), "{page} -> {hashed} should be hashed");
        assert!(web.join(hashed).is_file());
        assert!(!web.join(page).exists());
    }

    // ENTRY kept its name.
    assert!(web.join(ENTRY_HTML).is_file(), "index.html must remain unhashed");

    // Read entry — every ref inside should be hashed; query string preserved.
    let entry_src = fs::read_to_string(web.join(ENTRY_HTML)).unwrap();
    let tui_hashed = res.html_pages.get("tui_game.html").unwrap();
    assert!(
        entry_src.contains(&format!("href=\"{tui_hashed}\"")),
        "entry rewrites <a href=\"tui_game.html\"> → hashed"
    );
    assert!(
        entry_src.contains(&format!("'{tui_hashed}' + suffix")),
        "entry preserves JS string concat with hashed name"
    );
    assert!(
        entry_src.contains(&format!("'{tui_hashed}?'")),
        "entry preserves '?' query trailer on the redirect"
    );
    let cfg_hashed = res.js_leaves.get("server-config.js").unwrap();
    assert!(
        entry_src.contains(&format!("<script src=\"{cfg_hashed}\"")),
        "entry rewrites <script src=\"server-config.js\">"
    );

    // Read one game page — cross-page nav must point to index.html (cycle break).
    let tui_src = fs::read_to_string(web.join(tui_hashed)).unwrap();
    assert!(
        tui_src.contains("href=\"index.html\">GUI"),
        "cross-page nav (tui→native) cycle-broken to index.html"
    );
    assert!(
        tui_src.contains("href=\"index.html\">lobby"),
        "explicit index.html ref preserved"
    );
    let net_hashed = res.js_leaves.get("network.js").unwrap();
    assert!(
        tui_src.contains(&format!("'./{net_hashed}'")),
        "game page rewrites ES import './network.js' → hashed"
    );
    let data_hashed_full = &res.data_index.1;
    assert!(
        tui_src.contains(&format!("'/{data_hashed_full}'")),
        "game page rewrites fetch('/data/sets/index.json') → hashed"
    );
}
