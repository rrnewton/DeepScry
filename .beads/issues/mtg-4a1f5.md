---
title: 'Live smoke: allow_local_img_load=true ''Local'' label missing on tui_game + native_game'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-31T17:20:38.819784214+00:00
updated_at: 2026-05-31T17:20:51.915185747+00:00
---

# Description

Live smoke (web/smoke_test_live.js vs https://deepscry.net @17dfdef0) flagged [major]: with ?allow_local_img_load=true the 'Local' image-source label is MISSING on BOTH tui_game.html and native_game.html (localLabel=false even with override). Tasks #29/#35 implemented + made sticky this gate; this is either a real regression in the deployed build or a stale smoke selector (UI label changed). Investigate: (1) does the override still work in a manual browser session? (2) is the smoke checking a stale selector? Low priority (image loading is off by default and gated; does not affect playability). Found alongside the cache-skew fix deploy.
