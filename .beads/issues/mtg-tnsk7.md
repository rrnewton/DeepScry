---
title: 'Strict layering: network + player-controller logic must be SHARED across TUI and GUI (no per-UI duplication)'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T20:13:58.072105756+00:00
updated_at: 2026-06-01T03:04:18.336220798+00:00
---

# Description

USER architectural requirement + CONFIRMED ROOT-CAUSE. Rules: engine knows no UI; shared UI logic knows no renderer; networking + player-controller logic is the SAME code for TUI and GUI — only the renderer differs (docs/NETWORK_ARCHITECTURE.md).

ROOT CAUSE (was): web/native_game.html's NETWORK path called launch_network_game (ratzilla TUI) and created a #ratzilla-terminal div, so 'Native GUI' for a network game still rendered the TUI. The native CARD renderer (launch_game_session render loop) was LOCAL-ONLY.

FIX LANDED (branch phase3-native-renderer):
- Rust (mtg-engine/src/wasm/fancy_tui.rs): split launch_network_game into a shared, ratzilla-FREE helper create_and_install_network_session() (does ALL the network/controller setup: late-binding CardID ranges, server RNG state, network-mode controllers, run_until_choice, install_global_session) + an optional attach_ratzilla_renderer(). Two thin exports now sit on top:
    * launch_network_game        = create_and_install_network_session() + attach_ratzilla_renderer()  (tui_game.html, unchanged behavior)
    * launch_network_game_session = create_and_install_network_session() ONLY                          (NEW, ratzilla-free; native_game.html)
  The renderer is the ONLY thing that differs between the two callers — the network/controller code is byte-identical and lives in one place.
- web/native_game.html: network path now calls launch_network_game_session and renders via its OWN native card DOM (show #game-area, applySharedLayout, updateUI driven by tui_get_gui_view_model_json + tui_run_turn), exactly like its local path. The ratzilla-terminal facade (the div-creation block) is DELETED. tui_run_turn() in onMessageProcessed is the same controller advance the TUI page uses.
- DETERMINISM: the native view consumes the SAME GuiViewModel/choice stream the TUI view consumes; no extra/hidden info; the client sends nothing different to the server. attach_ratzilla_renderer() is pure rendering (does not mutate game state), proven by the identical hashes below.

VERIFIED (web/test_network_native_renderer_e2e.js, manual): native #game-area engages (cards), NO #ratzilla-terminal exists, GUI view model reports a 2-player synced opening (turn 1), and the native renderer introduces NO new desync.

DISCOVERED + FILED mtg-uzvu4 (separate, PRE-EXISTING, renderer-independent): the WASM Human controller in network mode desyncs at P2 turn-2 cleanup discard (server=f7ec406da80a882e client=3b7dd10b9f66711d at action_count=45). It reproduces IDENTICALLY through the unmodified ratzilla tui_game.html --human path with the same hash, regardless of deck — proving it is NOT introduced by this renderer work. Only the random-controller network path is in validate; --human is not. mtg-tnsk7 (renderer) is DONE; mtg-uzvu4 (Human-controller engine desync) tracks the remaining full-game sync.
