---
title: Add bug-report button to native_game.html (native GUI), not just Web TUI
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T18:50:44.169062268+00:00
updated_at: 2026-05-28T18:50:44.169062268+00:00
---

# Description

The bug-report button exists only in web/tui_game.html (Web TUI), NOT in web/native_game.html (native GUI). Add the same bug-report button + dialog to native_game.html so users on the native GUI (which is becoming the PRIMARY surface, mtg-574) can file bug reports too. Reuse the shared dialog/flow; apply the same WS-precheck + clear-error UX (sibling issue). Related: mtg-tan84 (bug-report pipeline), mtg-572/mtg-574 (native GUI primary).
