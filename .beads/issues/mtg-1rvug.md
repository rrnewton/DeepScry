---
title: Hash downstream game-page HTML so index.html is the sole mutable pointer
status: open
priority: 4
issue_type: task
created_at: 2026-05-28T17:00:52.960502811+00:00
updated_at: 2026-05-28T17:00:52.960502811+00:00
---

# Description

Follow-up to mtg-571. The user's stated ideal is that ONLY index.html is mutable. Today tui_game.html / native_game.html / demo.html are themselves fixed-name short-TTL files linked from index.html.

GOAL: content-hash the game-page HTML (tui_game.<hash>.html etc.) via trunk multi-target / multiple HTML entry points and rewrite index.html's launcher links to the hashed names, so index.html (+ deploy-generated server-config.js) is the ONLY mutable, short-TTL HTML. Depends on the full trunk rel=rust migration follow-up.
