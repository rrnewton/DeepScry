//! Integration test for `asset_hash::asset_graph::hash_full_graph`
//! (mtg-620 → mtg-682 → CAS-hardened mtg-704).
//!
//! Builds a synthetic `web_dir` on a tempdir that REPRODUCES THE REAL PURE-DAG
//! TOPOLOGY after the CAS rework: a stable entry `index.html` (the SOLE mutable
//! file, carrying the `__MTG_RELEASE_TOKEN__` sentinel), the pure leaves
//! (including the now-leaf `lobby_launcher.js`), a `launcher.html` forward hub,
//! the two game pages (NO tui⇄native switch link, NO asset_manifest import — they
//! just import the leaf `lobby_launcher.js`), `demo.html` (forward into the game
//! pages), and `deck_editor.html` whose "Back to Launcher" is the stable
//! `index.html?goto=launcher` dispatcher back-edge. It runs the full-graph hasher
//! and asserts the NEW invariants:
//!
//!   - ONLY `index.html` keeps its stable name; the old stable-named
//!     `asset_manifest.js` loader + `asset-manifest.json` are NEVER written.
//!   - EVERY other discovered asset is renamed `<stem>.<16-hex>.<ext>`
//!     (auto-discovery: `launcher.html` is hashed without any hardcoded list).
//!   - Every FORWARD edge (index/launcher/demo → pages, launcher → deck_editor,
//!     game pages → leaf import) is STATICALLY rewritten to the hashed name —
//!     there is no runtime manifest indirection left.
//!   - The full `logical → hashed` manifest is content-hashed → the release
//!     token; `asset-manifest.<token>.json` exists and the token is baked into
//!     `index.html` (placeholder consumed).
//!
//! This is the unit-level analogue of the JS deploy-tree nav test (which runs
//! the real server + HTTP fetches); it catches rewriter/ordering bugs without
//! a live `mtg server-web`.

use mtg_engine::asset_hash::asset_graph::{self, ENTRY_HTML, HASHED_JS_LEAVES, RELEASE_TOKEN_PLACEHOLDER};
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

fn build_web(web: &Path) {
    // Fake pkg pair (matches web_pkg::hash_web_assets requirements).
    write(
        &web.join("pkg/mtg_engine.js"),
        "// fake glue\nexport default function init() {}",
    );
    write(&web.join("pkg/mtg_engine_bg.wasm"), "fake wasm bytes");

    // Data leaf.
    write(&web.join("data/sets/index.json"), r#"{"sets":[]}"#);

    // Pure JS leaves — INCLUDING the now-leaf lobby_launcher.js (it references
    // NO html page after the mtg-704 leaf-ification, so it is a pure leaf).
    write(&web.join("server-config.js"), "window.MTG_WS_URL = 'ws://x';");
    write(&web.join("network.js"), "// network module");
    write(&web.join("bug_report.js"), "// bug report module");
    write(&web.join("help_dialog.js"), "export function installHelpDialog() {}");
    write(
        &web.join("lobby_launcher.js"),
        "export const STICKY_PARAM_KEYS = ['name', 'release', 'ws'];\n\
         export function buildRedirectQuery(o){ return new URLSearchParams(o); }\n",
    );

    // Game pages: NO tui⇄native switch link, NO asset_manifest import. Only a
    // MODULE-import of the leaf lobby_launcher.js + the other leaves; back-edge
    // is the stable index.html.
    write(
        &web.join("native_game.html"),
        r#"<html><head><script src="server-config.js"></script><script src="bug_report.js"></script></head>
<body><a href="index.html">lobby</a>
<script type="module">import { buildRedirectQuery } from './lobby_launcher.js'; import { installHelpDialog } from './help_dialog.js'; fetch('./data/sets/index.json');</script></body></html>"#,
    );
    write(
        &web.join("tui_game.html"),
        r#"<html><head><script src="server-config.js"></script></head>
<body><a href="index.html">lobby</a>
<script type="module">import './network.js'; import './bug_report.js'; import { buildRedirectQuery } from './lobby_launcher.js'; import './help_dialog.js'; fetch('/data/sets/index.json');</script></body></html>"#,
    );

    // launcher.html: forward hub. Owns the game-page filenames (forward DAG
    // edge) + forward-links deck_editor; imports the leaf.
    write(
        &web.join("launcher.html"),
        r#"<html><body><a href="deck_editor.html">Deck Editor</a> <a href="index.html">Back</a>
<script type="module">import { buildRedirectQuery } from './lobby_launcher.js';
const P = { tui: 'tui_game.html', native: 'native_game.html' };
fetch('./data/sets/index.json');</script></body></html>"#,
    );

    // demo.html: forward-links to BOTH game pages (DAG soft edges) — must
    // resolve to the hashed names statically.
    write(
        &web.join("demo.html"),
        r#"<html><body><a href="index.html">lobby</a> <a href="tui_game.html">TUI</a> <a href="native_game.html">GUI</a></body></html>"#,
    );

    // deck_editor.html: back-edge to launcher routed through the stable
    // dispatcher (NOT a direct launcher link → no cycle).
    write(
        &web.join("deck_editor.html"),
        r#"<html><body><a href="index.html?goto=launcher">Back to Launcher</a><a href="index.html">Back to Lobby</a><script>fetch('./data/sets/index.json');</script></body></html>"#,
    );

    // ENTRY: launch buttons + JS redirect + the release-token sentinel.
    write(
        &web.join("index.html"),
        r#"<html><head><script src="server-config.js"></script>
<script>var MTG_RELEASE_TOKEN = '__MTG_RELEASE_TOKEN__';</script></head>
<body>
<a id="launch-native" href="native_game.html">GUI</a>
<a id="launch-tui" href="tui_game.html">TUI</a>
<a id="launch-launcher" href="launcher.html">launcher</a>
<a id="launch-demo" href="demo.html">demo</a>
<a id="launch-deck-editor" href="deck_editor.html">edit</a>
<script>
  document.getElementById('launch-tui').href = 'tui_game.html' + suffix;
  window.location.href = 'launcher.html?' + qp.toString();
</script>
</body></html>"#,
    );
}

#[test]
fn pure_dag_auto_discovers_hashes_and_bakes_token() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path();
    build_web(web);

    let res = asset_graph::hash_full_graph(web).expect("hash_full_graph (pure DAG)");

    // Every pure JS leaf renamed away — INCLUDING lobby_launcher.js.
    for leaf in HASHED_JS_LEAVES {
        let hashed = res
            .js_leaves
            .get(*leaf)
            .unwrap_or_else(|| panic!("missing leaf {leaf}"));
        assert!(is_hashed_name(hashed), "{leaf} -> {hashed} should be hashed");
        assert!(web.join(hashed).is_file(), "{hashed} exists on disk");
        assert!(!web.join(leaf).exists(), "{leaf} should be renamed away");
    }
    assert!(
        res.js_leaves.contains_key("lobby_launcher.js"),
        "lobby_launcher.js is now a PURE LEAF (hashed up front, not a graph node)"
    );

    // Data index renamed.
    let (orig, hashed) = &res.data_index;
    assert_eq!(orig, "data/sets/index.json");
    assert!(is_hashed_name(hashed.split('/').next_back().unwrap()));
    assert!(web.join(hashed).is_file());

    // AUTO-DISCOVERY: launcher.html hashed without any hardcoded list.
    let launcher_hashed = res
        .graph_nodes
        .get("launcher.html")
        .expect("launcher.html auto-discovered + hashed");
    assert!(is_hashed_name(launcher_hashed));
    assert!(web.join(launcher_hashed).is_file());
    assert!(!web.join("launcher.html").exists());

    // Every non-stable HTML page renamed.
    for page in ["native_game.html", "tui_game.html", "demo.html", "deck_editor.html"] {
        let h = res.graph_nodes.get(page).unwrap_or_else(|| panic!("missing {page}"));
        assert!(is_hashed_name(h), "{page} -> {h} should be hashed");
        assert!(web.join(h).is_file());
        assert!(!web.join(page).exists());
    }

    // ONLY index.html stays unhashed; the old stable loader/manifest are GONE.
    assert!(web.join(ENTRY_HTML).is_file(), "index.html must remain unhashed");
    assert!(
        !web.join("asset_manifest.js").exists(),
        "the stable runtime loader must NOT exist (deleted cache vuln)"
    );
    assert!(
        !web.join("asset-manifest.json").exists(),
        "the stable-named manifest must NOT exist (only the content-hashed one)"
    );

    // The content-hashed immutable manifest exists and the token is self-consistent.
    assert_eq!(res.manifest_file, format!("asset-manifest.{}.json", res.release_token));
    let manifest_path = web.join(&res.manifest_file);
    assert!(manifest_path.is_file(), "asset-manifest.<token>.json written");
    assert_eq!(
        mtg_engine::asset_hash::asset_hash_hex(&fs::read(&manifest_path).unwrap()),
        res.release_token,
        "token == blake3(manifest bytes)"
    );

    // ── ENTRY: every forward ref hashed; query string preserved; token baked.
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
    assert!(
        !entry_src.contains(RELEASE_TOKEN_PLACEHOLDER),
        "release-token placeholder consumed"
    );
    assert!(entry_src.contains(&res.release_token), "real token baked into entry");

    // ── launcher.html forward edges hashed (deck_editor + game-page literals +
    //    the leaf import).
    let launcher_src = fs::read_to_string(web.join(launcher_hashed)).unwrap();
    let deck_hashed = res.graph_nodes.get("deck_editor.html").unwrap();
    let native_hashed = res.graph_nodes.get("native_game.html").unwrap();
    let ll_hashed = res.js_leaves.get("lobby_launcher.js").unwrap();
    assert!(
        launcher_src.contains(&format!("href=\"{deck_hashed}\"")),
        "launcher forward-link to deck_editor → HASHED"
    );
    assert!(
        launcher_src.contains(&format!("'{tui_hashed}'")) && launcher_src.contains(&format!("'{native_hashed}'")),
        "launcher owns the HASHED game-page filenames (forward DAG edge)"
    );
    assert!(
        launcher_src.contains(&format!("'./{ll_hashed}'")),
        "launcher module-imports the HASHED leaf lobby_launcher.js"
    );

    // ── demo.html: DAG soft links to BOTH game pages hashed; not flattened.
    let demo_hashed = res.graph_nodes.get("demo.html").unwrap();
    let demo_src = fs::read_to_string(web.join(demo_hashed)).unwrap();
    assert!(
        demo_src.contains(&format!("href=\"{tui_hashed}\"")),
        "demo → tui hashed"
    );
    assert!(
        demo_src.contains(&format!("href=\"{native_hashed}\"")),
        "demo → native hashed"
    );

    // ── game pages: import the HASHED leaf; NO switch link; NO asset_manifest.
    let tui_src = fs::read_to_string(web.join(tui_hashed)).unwrap();
    let native_src = fs::read_to_string(web.join(native_hashed)).unwrap();
    assert!(tui_src.contains(&format!("'./{ll_hashed}'")), "tui imports HASHED leaf");
    assert!(
        native_src.contains(&format!("'./{ll_hashed}'")),
        "native imports HASHED leaf"
    );
    assert!(!tui_src.contains("asset_manifest"), "tui has no asset_manifest import");
    assert!(
        !tui_src.contains("native_game") && !native_src.contains("tui_game"),
        "game pages have NO cross-renderer switch link (cycle removed)"
    );
    let help_hashed = res.js_leaves.get("help_dialog.js").unwrap();
    assert!(
        tui_src.contains(&format!("'./{help_hashed}'")),
        "tui rewrites help_dialog → hashed"
    );

    // ── deck_editor back-edge is the stable dispatcher URL (NOT a direct/hashed
    //    launcher link — that would be the cycle).
    let deck_src = fs::read_to_string(web.join(deck_hashed)).unwrap();
    assert!(
        deck_src.contains("index.html?goto=launcher"),
        "deck_editor back-edge → stable index.html?goto=launcher dispatcher"
    );
    assert!(
        !deck_src.contains(launcher_hashed.as_str()) && !deck_src.contains("\"launcher.html\""),
        "deck_editor does NOT link launcher directly (no cycle)"
    );

    // ── the FULL manifest maps every asset class to its hashed name, each on disk.
    for logical in [
        "launcher.html",
        "native_game.html",
        "tui_game.html",
        "deck_editor.html",
        "lobby_launcher.js",
        "data/sets/index.json",
    ] {
        let h = res
            .manifest
            .get(logical)
            .unwrap_or_else(|| panic!("manifest maps {logical}"));
        assert!(web.join(h).is_file(), "manifest target {logical} -> {h} exists on disk");
    }
    assert!(
        res.manifest.contains_key("pkg/mtg_engine.js"),
        "manifest pins pkg js (Merkle root)"
    );
    assert!(
        res.manifest.contains_key("pkg/mtg_engine_bg.wasm"),
        "manifest pins wasm (Merkle root)"
    );
}
