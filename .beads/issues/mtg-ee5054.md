---
title: Replay verifier missing from native_game.html (gui_view_model)
status: open
priority: 3
issue_type: task
created_at: 2026-05-12T20:50:27.105818739+00:00
updated_at: 2026-05-12T20:50:27.105818739+00:00
---

# Description

The WASM rewind/replay verifier (replay_verifier.rs, fancy_tui.rs::tui_set_verify_rewind_replay) is wired ONLY to tui_game.html. The native_game.html backend (mtg-engine/src/wasm/gui_view_model.rs) has NO equivalent verification, so any rewind-induced state divergence in the GUI mode silently corrupts state.

## Background

Discovered while enabling the verifier in Playwright e2e tests (commit on native-web-gui branch, 2026-05-12). For tui_game.html, every rewind captures pre-rewind state hash + log tail and verifies it matches post-replay; any divergence surfaces as 'REWIND/REPLAY FATAL'. For native_game.html: nothing.

Per CLAUDE.md (NETWORK_ARCHITECTURE.md): 'desync is ALWAYS fatal'. The verifier is the early-detection mechanism — native_game.html should have it too.

## Action

1. Decide: does native_game.html actually exercise rewind/replay? If yes, port the verifier wiring from fancy_tui.rs to gui_view_model.rs (extract common bits — replay_verifier.rs is already backend-neutral).
2. Add a parallel JS toggle (window.gui_set_verify_rewind_replay or reuse the same name).
3. Update test_game_gui*.js Playwright tests to enable it and check for REWIND/REPLAY FATAL.

## References

- mtg-zl49k: Web GUI undo/rewind architecture bugs (this would be a debug aid for that)
- mtg-engine/src/wasm/replay_verifier.rs: backend-neutral verifier
- mtg-engine/src/wasm/fancy_tui.rs:1502 set_verify_rewind_replay
- mtg-engine/src/wasm/gui_view_model.rs: NO verifier wiring
- web/test_network_utils.js::enableReplayVerifier (added in this commit)
