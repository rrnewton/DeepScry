---
title: 'BUG: Turn headers missing in fancy.html log (Turn 11 skipped, P1 actions appear during P2''s turn)'
status: open
priority: 0
issue_type: task
labels:
- bug
- gameplay
- log
- rewind-replay
created_at: 2026-05-15T17:06:22.910701346+00:00
updated_at: 2026-05-15T17:06:22.910701346+00:00
---

# Description

GAMEPLAY BUG: Turn headers are missing from the fancy.html game log, making it look like P1 acts during P2's turn.

_Imported from tg task `bug-missing-turn-headers` (status was OPEN); priority preserved._

## Notes (imported from tg)

Initial investigation: Native CLI run with seed=42 eric_avatar_draft vs gabriel_avatar_draft AI-vs-AI shows Turn 11 header CORRECTLY emitted (verified with --tag-gamelogs). So the bug is specific to fancy.html WASM rendering path with rewind/replay — most likely in the human-controller code path in mtg-engine/src/wasm/fancy_tui.rs::run_until_choice / rewind_to_turn_start. Previous similar fix was f0af0604 (turn separator removal on rewind). Suspecting an off-by-one or a NEW interaction.
DETAILED INVESTIGATION (2026-05-10):
