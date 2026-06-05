---
title: Web game HANG at turn 24 (eric avatar mirror), no error/banner - mid-cast Rowdy Snowballers + Aang Lesson trigger
status: open
priority: 2
issue_type: bug
created_at: 2026-06-05T13:52:32.751591100+00:00
updated_at: 2026-06-05T13:52:32.751591100+00:00
---

# Description

Web game HANG at turn 24 (eric avatar mirror), no error or banner.

REPORTED (user playtest 2026-06-05, debug TRACE OFF): an eric avatar mirror game reached turn 24 and HUNG - no error on either browser console, no error banner. The game just stopped advancing.

Last log lines before the hang:
  Turn 24 - player2 draws Trusty Boomerang
  player2 casts Rowdy Snowballers (tapping Island + Plains + Plains)
  'Trigger: Aang, the Last Airbender - Whenever you cast a Lesson spell, NICKNAME gains lifelink until end of turn'
...and then nothing.

SUSPECTS: the Aang 'whenever you cast a Lesson spell' trigger firing on the Rowdy Snowballers cast may be the stall point (trigger resolution loop / step re-entry that re-runs instead of resuming). Also note 'NICKNAME' appears unsubstituted in the trigger text - an unresolved name-substitution placeholder, possibly a related symptom.

PRIORITY 2 (game-stopping hang). Related: mtg-242 (WASM network random intermittent hangs), mtg-610 (step/resolution re-entry should resume not re-run). Full turn-23/24 log captured in the playtest session.
