---
title: 'Web network gamelog: first turn banner ''>>> Turn N <<<<'' duplicated (rewind preserves it, replay re-logs it)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-06T00:25:52.346116552+00:00
updated_at: 2026-06-06T00:25:52.346116552+00:00
---

# Description

User-reported REPEATEDLY (frustrated 2026-06-06, never previously filed): in the web NETWORK game GUI the first gamelog line is printed twice:
  >>> Turn 1 - player1 20 (playrrr2 20) <<<<
  >>> Turn 1 - player1 20 (playrrr2 20) <<<<
  player1 casts Mox Pearl (56) ...
Only the first turn banner doubles; the rest of the log is fine.

NOT a web bug: web/native_game.html renderLog() (line 2638) REPLACES the whole log from state.logs (no append-doubling) with a content-signature skip. So the duplication is in the log DATA the WASM shadow produces, and it is NETWORK-mode specific (local games don't rewind).

ROOT CAUSE (traced): on a turn change, GameState (state.rs:3160-3166) logs turn_separator("") + turn_separator(turn_msg) [the banner], THEN logs the ChangeTurn undo-action with prior_log_size = log_count() captured AFTER the banner (state.rs:3167, comment 'so rewind preserves it'). The shadow's rewind truncates the log buffer to prior_log_size (undo.rs:1714-1719) — which INCLUDES the banner, so the banner is preserved. Then the forward REPLAY re-executes the turn-change code, which re-logs turn_separator(banner) → the banner now appears TWICE. turn_separator() (logger.rs:761) does NOT dedup consecutive identical lines.

WHY ONLY TURN 1: the early rewind/replay cycle covers the turn-1 ChangeTurn; later banners may not be in a rewound span in a typical game (confirm: it may actually affect any rewound turn-change).

FIX OPTIONS (pick the principled one after a network repro):
(a) capture prior_log_size BEFORE the banner so rewind truncates the banner away and the replay re-adds it exactly once (verify a rewind WITHOUT a following replay doesn't then lose the banner);
(b) make the banner logging idempotent on replay — skip re-logging a turn_separator identical to the last buffer entry (a dedup; safe but treats the symptom);
(c) tag the banner as a non-replayed log side-effect so replay doesn't re-emit it.
Prefer (a) or (c) over the (b) band-aid. VERIFY with a web network random/random game (the deployed repro) that the first line appears exactly once AND no banner is lost on rewind. Part of the netarch rewind/replay cleanup (same machinery as reconstruct-after-rewind, mtg-ho2r8). Related: mtg-412 (gamelog perspective), mtg-ho2r8.
