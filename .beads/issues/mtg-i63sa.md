---
title: 'Easy web/WASM game testing infra: mtg_wasm_game.py CLI + DRY web_game_common'
status: closed
priority: 2
issue_type: task
created_at: 2026-05-30T20:09:06.777871674+00:00
updated_at: 2026-06-01T13:34:16.931700852+00:00
---

# Description

## Easy web/WASM game testing infra: mtg_wasm_game.py CLI + DRY web_game_common — DONE

Closed 2026-06-01 gardening: DONE per description ('DONE' section lists all items as complete).

Evidence:
- scripts/mtg_wasm_game.py: EXISTS (confirmed ls)
- agentplay/lib/web_game_common.py: DRY shared infra (find_free_port, derive_controller_seeds, deck_path_to_wasm_name, etc.)
- agentplay/README.md:118: documents mtg_wasm_game.py with examples
- Per-turn screenshots + gamelog working
- WASM page name 404 bug fixed (tui_game.html/native_game.html → WASM_PAGE_FILES map)
- :8080 audit clean

All 5 DONE items in the description are verified implemented.
