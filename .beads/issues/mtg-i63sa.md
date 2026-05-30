---
title: 'Easy web/WASM game testing infra: mtg_wasm_game.py CLI + DRY web_game_common'
status: open
priority: 2
issue_type: task
created_at: 2026-05-30T20:09:06.777871674+00:00
updated_at: 2026-05-30T20:09:06.777871674+00:00
---

# Description

Make all testing tools usable against web/WASM games as easily as native, capturing screenshots + gamelogs. Branch web-ui-agentplay.

DONE:
1. DRY shared infra: NEW agentplay/lib/web_game_common.py centralizes the pieces previously duplicated across scripts/mtg_tui_networked.py and agentplay/lib/wasm_process.py — find_free_port(), derive_controller_seeds() (the mtg tui --seed → per-controller P1/P2 salt formula), deck_path_to_wasm_name(), and the common mtg tui arg surface (add_common_mtg_tui_args / parse_common_mtg_tui_args → MtgTuiArgs: decks, --p1/--p2, --seed, --max-turns). mtg_tui_networked.py and wasm_process.py now import these instead of carrying their own copies.

2. NEW CLI scripts/mtg_wasm_game.py: drop-in for mtg tui that runs the game in headless WASM (Playwright/Chromium) via the page's OWN launcher UI, so screenshots show the rendered game. Same essential flags (deck(s), --p1/--p2, --seed, --max-turns) plus --page {fancy,game}, --out-dir, --headed, --networked. Artifacts: game.log, snapshot.json, wasm_transcript.log, screenshots/turn_NNNN.png + final.png.

3. Per-turn screenshots + gamelog: new WasmPlaywrightProcess.run_autoplay_ui(max_turns) drives the launcher (deck/controller selectors + Launch + per-turn Space/Run-1-Turn), screenshots each new turn, reads the live game's view model for the gamelog. Works for both fancy (tui_game.html) and game (native_game.html) pages.

4. FIXED historical WASM flakiness: wasm_process.py navigated to fancy.html/game.html, but those were renamed to tui_game.html/native_game.html — the tab 404'd (engine still ran via the absolute-path bridge import, but every screenshot captured the 404 page). Added WASM_PAGE_FILES map. The gated test_drivers_run_to_completion_wasm (AGENTPLAY_TEST_WASM=1) now passes and screenshots are real.

5. :8080 audit: no production lobby/landing/game page hardcodes :8080. server-config.js derives a port-less same-origin wss:// URL from window.location; the only literal localhost:8080 is the degenerate file:// fallback. index.html uses MTG_WS_URL or ?ws= override. Test/harness ports (test_*.js, wasm_ai_harness.html, smoke_test_live.js) are legitimate.

Evidence: random-vs-random WASM game ran end-to-end:
  scripts/mtg_wasm_game.py --p1 random --p2 random --seed 42 --max-turns 25 decks/old_school2/the_deck_classic.dck
→ game.log (163 lines), 26 per-turn screenshots in debug/. mtg_tui_networked.py networked random-vs-random also runs to completion after the refactor (P1 wins).

Refactor verified behavior-preserving: prompt/shape parity tests + gated WASM driver test pass.
