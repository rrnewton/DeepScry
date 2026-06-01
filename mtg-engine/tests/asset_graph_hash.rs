//! Integration test for `asset_hash::asset_graph::hash_full_graph`
//! (mtg-620, generalized for the auto-discovering graph-aware renamer mtg-682).
//!
//! Builds a synthetic `web_dir` on a tempdir that REPRODUCES THE REAL TOPOLOGY
//! (so the test would have caught the lobby-redo deploy break): a stable entry
//! `index.html`, a stable manifest loader `asset_manifest.js`, the pure leaves,
//! a `launcher.html` SECOND forward hub, the two game pages that cross-link
//! (`tui_game.html` ⇄ `native_game.html` — the cycle) and import
//! `lobby_launcher.js`, and `lobby_launcher.js` itself naming the game pages
//! via string literals. It then runs the full-graph hasher and asserts the
//! GENERAL invariants:
//!
//!   - `index.html` + `asset_manifest.js` keep their stable names.
//!   - EVERY other discovered asset is renamed to `<stem>.<16-hex>.<ext>`
//!     (auto-discovery: `launcher.html` is hashed WITHOUT being on any
//!     hardcoded list — the bug that started this).
//!   - Module imports (ES `import`, `<script src>`) resolve to hashed names
//!     (the game pages' `import './lobby_launcher.js'` is hashed).
//!   - DAG soft refs (index/launcher/demo → game pages) resolve to hashed
//!     names statically — NOT blanket-redirected to `index.html`.
//!   - The genuine cycle (`tui ⇄ native`, `lobby_launcher.js → games`) is
//!     resolved via the runtime manifest: the surviving logical names appear
//!     in `asset-manifest.json` and the `asset_manifest.js` MANIFEST literal,
//!     and every manifest target exists on disk.
//!
//! This is the unit-level analogue of the JS deploy-tree nav test (which runs
//! the real server + HTTP fetches); it catches rewriter/ordering bugs without
//! a live `mtg server-web`.

use mtg_engine::asset_hash::asset_graph::{self, ENTRY_HTML, HASHED_JS_LEAVES, MANIFEST_JS};
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

/// Minimal stand-in for the real `web/asset_manifest.js`: carries the marker
/// comment + the `export const MANIFEST = {}` literal the hasher rewrites.
const MANIFEST_LOADER: &str = "'use strict';\n\
/* @@ASSET_MANIFEST@@ */\n\
export const MANIFEST = {};\n\
export function resolveAsset(n) { return MANIFEST[n] || n; }\n";

fn build_web(web: &Path) {
    // Fake pkg pair (matches web_pkg::hash_web_assets requirements).
    write(
        &web.join("pkg/mtg_engine.js"),
        "// fake glue\nexport default function init() {}",
    );
    write(&web.join("pkg/mtg_engine_bg.wasm"), "fake wasm bytes");

    // Data leaf.
    write(&web.join("data/sets/index.json"), r#"{"sets":[]}"#);

    // Pure JS leaves.
    write(&web.join("server-config.js"), "window.MTG_WS_URL = 'ws://x';");
    write(&web.join("network.js"), "// network module");
    write(&web.join("bug_report.js"), "// bug report module");
    write(&web.join("help_dialog.js"), "export function installHelpDialog() {}");

    // The STABLE manifest loader.
    write(&web.join(MANIFEST_JS), MANIFEST_LOADER);

    // lobby_launcher.js: imports the stable loader, names the game pages via
    // GAME_PAGE string literals (soft refs → manifest). NOT a pure leaf.
    write(
        &web.join("lobby_launcher.js"),
        "import { resolveAsset } from './asset_manifest.js';\n\
         export const GAME_PAGE = { tui: 'tui_game.html', native: 'native_game.html' };\n\
         export function buildRedirectUrl(o){ return resolveAsset(GAME_PAGE[o.ui]); }\n",
    );

    // Game pages: cross-link (the cycle) via data-asset-href, MODULE-import
    // lobby_launcher.js + the loader + leaves.
    write(
        &web.join("native_game.html"),
        r#"<html><head><script src="server-config.js"></script><script src="bug_report.js"></script></head>
<body><a href="tui_game.html" data-asset-href="tui_game.html">TUI</a> <a href="index.html">lobby</a>
<script type="module">import { buildRedirectUrl } from './lobby_launcher.js'; import { installManifestHrefRewrite } from './asset_manifest.js'; import { installHelpDialog } from './help_dialog.js'; fetch('./data/sets/index.json');</script></body></html>"#,
    );
    write(
        &web.join("tui_game.html"),
        r#"<html><head><script src="server-config.js"></script></head>
<body><a href="native_game.html" data-asset-href="native_game.html">GUI</a> <a href="index.html">lobby</a>
<script type="module">import './network.js'; import './bug_report.js'; import { buildRedirectUrl } from './lobby_launcher.js'; import { installManifestHrefRewrite } from './asset_manifest.js'; import './help_dialog.js'; fetch('/data/sets/index.json');</script></body></html>"#,
    );

    // launcher.html: the SECOND forward hub (the bug). Auto-discovery must hash
    // it. It forward-links to deck_editor.html (a DAG soft edge) and imports
    // lobby_launcher.js.
    write(
        &web.join("launcher.html"),
        r#"<html><body><a href="deck_editor.html">Deck Editor</a> <a href="index.html">Back</a>
<script type="module">import { buildRedirectUrl } from './lobby_launcher.js'; fetch('./data/sets/index.json');</script></body></html>"#,
    );

    // demo.html: forward-links to BOTH game pages (DAG soft edges into the
    // cycle) — must resolve to the hashed names statically.
    write(
        &web.join("demo.html"),
        r#"<html><body><a href="index.html">lobby</a> <a href="tui_game.html">TUI</a> <a href="native_game.html">GUI</a></body></html>"#,
    );

    write(
        &web.join("deck_editor.html"),
        r#"<html><body><a href="index.html">Back to Lobby</a><script>fetch('./data/sets/index.json');</script></body></html>"#,
    );

    // ENTRY: launch buttons + JS redirect with query string + the launcher hub.
    write(
        &web.join("index.html"),
        r#"<html><head><script src="server-config.js"></script></head>
<body>
<a id="launch-native" href="native_game.html">GUI</a>
<a id="launch-tui" href="tui_game.html">TUI</a>
<a id="launch-launcher" href="launcher.html">launcher</a>
<a id="launch-demo" href="demo.html">demo</a>
<script>
  document.getElementById('launch-tui').href = 'tui_game.html' + suffix;
  window.location.href = 'launcher.html?' + qp.toString();
</script>
</body></html>"#,
    );
}

#[test]
fn full_graph_auto_discovers_and_hashes_everything_but_stable() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path();
    build_web(web);

    let res = asset_graph::hash_full_graph(web).expect("hash_full_graph");

    // Every pure JS leaf renamed away.
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

    // AUTO-DISCOVERY: launcher.html hashed without any hardcoded list (the bug).
    let launcher_hashed = res
        .graph_nodes
        .get("launcher.html")
        .expect("launcher.html auto-discovered + hashed (the lobby-redo bug)");
    assert!(is_hashed_name(launcher_hashed));
    assert!(web.join(launcher_hashed).is_file());
    assert!(!web.join("launcher.html").exists());

    // Every non-stable HTML page + graph-JS renamed.
    for page in [
        "native_game.html",
        "tui_game.html",
        "demo.html",
        "deck_editor.html",
        "lobby_launcher.js",
    ] {
        let h = res.graph_nodes.get(page).unwrap_or_else(|| panic!("missing {page}"));
        assert!(is_hashed_name(h), "{page} -> {h} should be hashed");
        assert!(web.join(h).is_file());
        assert!(!web.join(page).exists());
    }

    // STABLE names kept.
    assert!(web.join(ENTRY_HTML).is_file(), "index.html must remain unhashed");
    assert!(
        web.join(MANIFEST_JS).is_file(),
        "asset_manifest.js must remain unhashed (stable loader)"
    );

    // ── ENTRY: every ref hashed; query string preserved; NOT redirected to self.
    let entry_src = fs::read_to_string(web.join(ENTRY_HTML)).unwrap();
    let tui_hashed = res.graph_nodes.get("tui_game.html").unwrap();
    assert!(
        entry_src.contains(&format!("href=\"{tui_hashed}\"")),
        "entry <a href tui> → hashed"
    );
    assert!(
        entry_src.contains(&format!("'{tui_hashed}' + suffix")),
        "entry JS concat → hashed"
    );
    assert!(
        entry_src.contains(&format!("'{launcher_hashed}?'")),
        "entry launcher redirect '?' query preserved + hashed"
    );
    let cfg_hashed = res.js_leaves.get("server-config.js").unwrap();
    assert!(
        entry_src.contains(&format!("<script src=\"{cfg_hashed}\"")),
        "entry <script src> → hashed"
    );

    // ── launcher.html (the second hub): forward-link to deck_editor must be
    //    HASHED (NOT redirected to index.html — the rejected hack), and its
    //    lobby_launcher import hashed.
    let launcher_src = fs::read_to_string(web.join(launcher_hashed)).unwrap();
    let deck_hashed = res.graph_nodes.get("deck_editor.html").unwrap();
    assert!(
        launcher_src.contains(&format!("href=\"{deck_hashed}\"")),
        "launcher forward-link to deck_editor → HASHED (not flattened to index.html)"
    );
    let ll_hashed = res.graph_nodes.get("lobby_launcher.js").unwrap();
    assert!(
        launcher_src.contains(&format!("'./{ll_hashed}'")),
        "launcher module-imports the HASHED lobby_launcher.js"
    );

    // ── demo.html: DAG soft links to BOTH game pages must be HASHED.
    let demo_hashed = res.graph_nodes.get("demo.html").unwrap();
    let demo_src = fs::read_to_string(web.join(demo_hashed)).unwrap();
    let native_hashed = res.graph_nodes.get("native_game.html").unwrap();
    assert!(
        demo_src.contains(&format!("href=\"{tui_hashed}\"")),
        "demo → tui hashed"
    );
    assert!(
        demo_src.contains(&format!("href=\"{native_hashed}\"")),
        "demo → native hashed"
    );
    assert!(
        !demo_src.contains("href=\"index.html\">TUI"),
        "demo NOT flattened to index.html"
    );

    // ── game pages: MODULE import of lobby_launcher.js must be HASHED (the
    //    cycle is broken on the module edge — lobby_launcher hashed first).
    let tui_src = fs::read_to_string(web.join(tui_hashed)).unwrap();
    let native_src = fs::read_to_string(web.join(native_hashed)).unwrap();
    assert!(
        tui_src.contains(&format!("'./{ll_hashed}'")),
        "tui imports HASHED lobby_launcher.js"
    );
    assert!(
        native_src.contains(&format!("'./{ll_hashed}'")),
        "native imports HASHED lobby_launcher.js"
    );
    // The stable loader import must stay LOGICAL (never hashed).
    assert!(
        tui_src.contains("'./asset_manifest.js'"),
        "tui imports the STABLE asset_manifest.js"
    );
    let help_hashed = res.js_leaves.get("help_dialog.js").unwrap();
    assert!(
        tui_src.contains(&format!("'./{help_hashed}'")),
        "tui rewrites help_dialog import → hashed"
    );

    // ── the CYCLE → runtime manifest. Both game pages are manifest-resolved
    //    (their mutual nav cannot all be statically baked). Every manifest
    //    target exists on disk; lobby_launcher.js is NOT in the manifest (its
    //    hashed name was statically baked into the game-page imports).
    assert!(res.manifest.contains_key("tui_game.html"), "tui in manifest");
    assert!(res.manifest.contains_key("native_game.html"), "native in manifest");
    assert!(
        !res.manifest.contains_key("lobby_launcher.js"),
        "lobby_launcher statically resolved, not in manifest"
    );
    for (logical, hashed) in &res.manifest {
        assert!(
            web.join(hashed).is_file(),
            "manifest target {logical} -> {hashed} exists on disk"
        );
    }

    // ── manifest written to BOTH the JSON and the loader literal.
    let manifest_json = fs::read_to_string(web.join("asset-manifest.json")).unwrap();
    assert!(
        manifest_json.contains(tui_hashed),
        "asset-manifest.json maps tui → hashed"
    );
    let loader_src = fs::read_to_string(web.join(MANIFEST_JS)).unwrap();
    assert!(
        loader_src.contains(tui_hashed),
        "asset_manifest.js MANIFEST literal maps tui → hashed"
    );
    assert!(
        loader_src.contains("/* @@ASSET_MANIFEST@@ */"),
        "marker comment preserved"
    );
    assert!(
        loader_src.contains("resolveAsset"),
        "loader's resolveAsset() function preserved (only the literal spliced)"
    );

    // ── lobby_launcher.js (hashed): GAME_PAGE literals stay LOGICAL (manifest
    //    resolves at runtime); the stable loader import stays logical.
    let ll_src = fs::read_to_string(web.join(ll_hashed)).unwrap();
    assert!(
        ll_src.contains("'tui_game.html'"),
        "GAME_PAGE keeps logical tui name (manifest resolves)"
    );
    assert!(
        ll_src.contains("'./asset_manifest.js'"),
        "lobby_launcher imports the STABLE loader"
    );
}
