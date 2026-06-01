---
title: Unify native_game.html and tui_game.html on ONE common query-parameter dispatch interface
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T20:13:58.070866123+00:00
updated_at: 2026-06-01T03:04:18.337836855+00:00
---

# Description

DRY: unify native_game.html + tui_game.html shared layer; only the renderer should differ. PARAM/DISPATCH contract already done via lobby_launcher.js (Phase 2); this issue's DRY half continues here.

DONE (branch phase3-native-renderer):
- Extracted web/help_dialog.js — the shared help modal (installHelpDialog({getHelpText})), previously duplicated (~40 lines) in BOTH game pages with subtle drift (native had overlay/dialog onclick + null guards; tui did not). Both pages now import it; the page only supplies the help TEXT via a closure over its tui_get_help_text WASM import. Net: both pages thinner, single source of truth for the modal.
- Wired help_dialog.js into the content-addressing pipeline so it ships hashed+immutable: added to HASHED_JS_LEAVES in mtg-engine/src/asset_hash.rs, to JS_LEAVES in web/test_web_server_smoke.js, and to the asset_graph_hash.rs test fixture (+ a new assertion that the import is rewritten to the hashed name).

REMAINING (deferred, documented): the larger pure-module extractions (wasm_boot.js: init()/manifest resolution/load_set/load_tokens/prefetch; card_images.js: local->scryfall->gatherer cascade + allow_local_img_load gate; net_game_driver.js as a JS-level renderer-agnostic driver). The RENDERER-AGNOSTIC network/controller split — the substantive part of net_game_driver — was instead landed at the WASM layer (see mtg-tnsk7: create_and_install_network_session + launch_network_game_session), which is where the actual layering bug lived; the two game pages now share that one network/controller path and differ only in renderer. The remaining JS boot/image extractions are mechanical DRY cleanups (loadSetFiles/ensureTokensLoaded/loadCardsForDecks/resolveLocalImageAllowed thread page-scoped state and were deferred to keep this change green and low-risk per the AFK incremental fallback).
