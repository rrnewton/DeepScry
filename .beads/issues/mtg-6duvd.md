---
title: 'Desync tooling P2: auto binary-search-on-action_count first-diverging-ACTION finder'
status: open
priority: 3
issue_type: task
created_at: 2026-06-05T17:42:17.604311411+00:00
updated_at: 2026-06-05T17:42:17.604311411+00:00
---

# Description

Audit P2. Given a reproducing seed/deck/controller, replay both server-model and shadow-model and binary-search action_count to return the EXACT first action where the undo-logs/hashes diverge — the missing 'first-diverging-action' tool (today done by hand-diffing undo dumps, 0% reuse). Pairs with P1. Effort M. Related: mtg-o99ow.
