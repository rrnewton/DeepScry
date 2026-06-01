---
title: 'TRACK: Lobby+server-protocol redesign to deployed prototype (AFK build 2026-05-31)'
status: open
priority: 1
issue_type: task
created_at: 2026-06-01T00:34:25.997406631+00:00
updated_at: 2026-06-01T00:34:25.997406631+00:00
---

# Description

AFK AUTONOMOUS BUILD authorized by user 2026-05-31. Drive the lobby/launcher/game redesign to a COMPLETE DEPLOYED PROTOTYPE without blocking for questions. Design doc: ai_docs/LOBBY_LAUNCHER_GAME_ARCHITECTURE_20260531.md (decisions LOCKED).

DECISIONS (locked): 3 thin pages (lobby+launcher+waiting-room as the light no-WASM page; native_game + tui_game as thin single-renderer game pages) + deck_editor.html as a 4th page LATER. Prefetch wasm+selected-deck bins during waiting-room idle (content-addressed→cache hit). Renderer chosen in launcher→navigate to matching page (kills native-renders-TUI bug). NO live mid-game renderer switch. Max DRY shared layer. ONE deck picker (waiting room). Server owns: unique-name registration, waiting-games+heartbeat/eviction, per-player deck+ready, reconnect tokens (design).

SEQUENCING (coordinator drives via worktrees+agents, sequential merges, validate-gated, clean validate-launch cmd w/o 'validate.sh' substring):
- PHASE 1 (mtg-protocol): worktree lobby-server-protocol. Server protocol+state: ServerMessage/ClientMessage for Register(unique name), waiting-game registration on CreateGame + heartbeat/eviction on WS drop (mtg-dw9j3), per-player deck+ready, reconnect token issue/redeem (design+implement core). PLUS trusted-bug-report-password infallibility fix (validate_trusted_bug_report_password Result<bool>→bool: ALWAYS store, only flag trusted; empty config or wrong pw ⇒ untrusted, NEVER reject upload) (server.rs:2654). PLUS server/server-web --help rewrite, purge retired-python mentions (mtg-57hso). PLUS deploy: gitignored trusted-password file + deploy-cloud.sh warns-but-proceeds if unset; dedup --cardsfolder in unit (mtg-obrx2). No mtg-rules-review needed (no gameplay rules change). NO deploy yet.
- PHASE 2 (mtg-web): worktree off integration AFTER phase1 merges. 3-page web rewrite + shared ES modules (wasm_boot.js, card_images.js, net_game_driver.js renderer-agnostic, help_dialog.js, bug_report.js w/ advanced trusted-pw field, extend lobby_launcher.js). lobby.html=lobby+launcher+waiting-room w/ real server registration+heartbeat list+one deck picker+prefetch+deck-editor button(stub). native_game/tui_game→thin shells importing shared layer. content-address cards.bin + kill fixed-name data-bin refs (mtg-33fmb).
- PHASE 3 (mtg-tnsk7): native network renderer (real native card render for NETWORK games, not the TUI facade). RISKY (net determinism — desync fatal). Attempt; if not safely landable AFK, ship phases 1-2 + honest doc, leave tnsk7 open.
- THEN: promote main + deploy + verify live + report for user review.

Sub-issues: mtg-khy7x(UX storyboard, this design), mtg-1vwpd(param contract), mtg-tnsk7(layering/native renderer), mtg-dw9j3(heartbeat), mtg-33fmb(cards.bin), mtg-57hso(help), mtg-obrx2(deploy unit+pw file). Live baseline before build: main=9d125ae2; integration=50612dba(rogue, undeployed).
