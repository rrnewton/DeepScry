---
title: 'Native web GUI: turn-banner log line duplicated/concatenated (repeated ~3x, no newlines)'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-570: related
  mtg-669: related
created_at: 2026-06-03T03:05:49.469904974+00:00
updated_at: 2026-06-03T03:05:49.469904974+00:00
---

# Description

In the native web GUI (web/native_game.html, the card-style renderer), the turn banner log line renders duplicated and concatenated multiple times with no separating newlines. Observed live on deepscry.net (2026-06-03): '>>> Turn 1 - eric_avatar_draft 20 (player2 20) <<<<' repeated ~3x in a row on one line. Likely a log-append/re-render bug in the native GUI log panel (turn banner emitted/appended multiple times, or missing dedup/newline). Cosmetic/UX, not gameplay-affecting. Repro: play a game in the native web GUI, watch the turn-1 banner. Related: mtg-570 (dup/garbled log lines in TUI render, same class different renderer), mtg-669 (shared TUI/GUI log code, no per-UI dup).
