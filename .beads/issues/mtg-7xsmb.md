---
title: 'Web/WASM game testing infra: DRY consolidation + real networked WASM CLI'
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T14:50:05.623026314+00:00
updated_at: 2026-06-02T14:50:05.623026314+00:00
---

# Description

Made the web/WASM game testing tools first-class and DRY (under mtg-5 testing/methodology; relates mtg-198 web/WASM network, mtg-183 networking e2e).

DRY consolidation (agentplay/lib/web_game_common.py is now the single home for web-game plumbing):
- Added wait_for_tcp(), spawn_http_server() (static file server rooted at web/), build_mtg_server_cmd(), build_mtg_connect_cmd().
- agentplay/lib/wasm_process.py: _start_http_server now calls the shared spawn_http_server (removed its inline Popen + readiness loop; dropped now-unused socket import).
- scripts/mtg_tui_networked.py: server argv + client argv now built via the shared build_mtg_server_cmd/build_mtg_connect_cmd (removed the inline server_cmd list and the inline build_client_cmd body). Behavior verified identical (native random-vs-random networked game still completes, both clients exit 0).
- All three backends (native networked, WASM local, WASM networked, agentplay WASM driver) share ONE free-port picker, ONE http.server spawn, ONE seed-derivation, ONE server/connect argv builder.

Real networked WASM CLI (was a stub):
- scripts/mtg_wasm_game.py --networked previously printed a NOTE and fell through to the local path. It now drives WasmPlaywrightProcess.run_network_ui(): spawns a native mtg server + a native AI peer (mtg connect), then boots the headless browser tab as the second network client via the proven ?mode=network&ws=&server_pass=&name=&deck=&controller= auto-match contract (same one web/test_network_*_e2e.js use). Captures per-turn + progress + final screenshots and the gamelog, same run-dir layout as the local path.
- Drop-in mtg-tui arg surface preserved via web_game_common.add_common_mtg_tui_args; --networked toggles native-server-backed vs in-tab local WASM. screenshots/gamelog/snapshot land in the run dir by default (debug/wasm_game_<ts>/ or --out-dir).

Testing modes confirmed working on web games with screenshots: random + heuristic local AI-vs-AI (scripts/mtg_wasm_game.py), random networked WASM (--networked, ran to natural game over), and agent-directed via agentplay/agent_game.py --driver=wasm (mock LLM = engine-driven, captures game-over screenshot; per-decision screenshots fire on real Python ChoicePoints).

KNOWN LIMITATION (honest): networked-WASM screenshots are SPARSE compared to local. The WASM network client is a passive shadow — its tui_get_gui_view_model_json() turn_number advances only at sync points, not every turn, and the network AI races through turns. Mitigated with a progress-on-log-growth fallback (progress_NNNN.png every +12 log lines) plus turn_NNNN + final. A networked 10-14 turn game yields ~2-3 frames vs local's per-turn frames. The FINAL screenshot reliably renders the full board + game-over banner. Future work: hook the WASM network client to emit a per-turn render event the harness can await, or screenshot on every choice_seq increment seen in the browser console stream.

Gate: agentplay pytest stays green (79 passed, 3 skipped). make validate cited in branch.
