---
title: 'BUG: Per-source image toggles (local/scryfall/gatherer) are read but never consulted'
status: open
priority: 3
issue_type: task
labels:
- bug
- game-html
created_at: 2026-05-15T17:07:47.295557642+00:00
updated_at: 2026-05-15T17:07:47.295557642+00:00
---

# Description

The per-source toggle checkboxes (#img-src-local, #img-src-scryfall, #img-src-gatherer) in native_game.html are read into the settings object but never actually consulted when building the fallback list. Only the master "Show Card Images" checkbox gates rendering.

_Imported from tg task `fix-image-source-toggles` (status was BACKLOG); priority preserved._
