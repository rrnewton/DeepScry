---
title: 'Web game: start PAUSED, don''t auto-run; spacebar/''Run 1 turn''/''Auto run'' advance; ?auto_run=true override'
status: open
priority: 3
issue_type: feature
created_at: 2026-06-05T13:52:32.666251510+00:00
updated_at: 2026-06-05T13:52:32.666251510+00:00
---

# Description

Web random/random (and any auto-runnable) game should start PAUSED, not auto-run.

REPORTED (user playtest 2026-06-05): loading a random/random game in the web interface, it runs automatically as soon as both game screens load (after both players press Ready), IGNORING the 'Run 1 turn' / 'Auto run' control in the upper-right.

DESIRED:
- Start in a PAUSED state.
- Advance by: spacebar, OR the 'Run 1 turn' button, OR enabling 'Auto run'.
- Optional ?auto_run=true query param to auto-enable auto-run for testing.

FUTURE: enables a benchmark - how fast can a random/random browser game run to completion (near-instant Random controller). Note: full-speed random/random is currently still slow (see the log-spam + perf issue).

Files: the network game page auto-run control + game boot flow (web/native_game.html, web/tui_game.html, web/game_boot_params.js). Related: mtg-567 (auto-run button spams turn message with human controller).
