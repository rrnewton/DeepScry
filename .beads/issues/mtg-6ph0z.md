---
title: Token scripts not loading (Food, Clue tokens fail)
status: closed
priority: 3
issue_type: bug
created_at: 2026-01-06T02:08:06.721723707+00:00
updated_at: 2026-01-06T15:26:59.287884291+00:00
closed_at: 2026-01-06T15:26:59.287884190+00:00
---

# Description

When casting cards that create tokens (like Canyon Crawler creating Food tokens), the game crashes with: Error: Token definition not found. Token scripts exist in tokenscripts folder but aren't loaded by Rust engine. Affected cards: Canyon Crawler, Cunning Maneuver.
