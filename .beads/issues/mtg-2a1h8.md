---
title: 'Native web GUI: turn-banner log line duplicated/concatenated (repeated ~3x, no newlines)'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-669: related
  mtg-570: related
created_at: 2026-06-03T03:05:49.469904974+00:00
updated_at: 2026-06-03T04:03:16.248658097+00:00
---

# Description

Native web GUI (web/native_game.html, card-style renderer): turn-banner log line rendered duplicated/concatenated ~3x. Observed live on deepscry.net. Related: mtg-570, mtg-669, mtg-432.

ROOT CAUSE (analyzed 2026-06-02, branch fix-log-rendering) — SAME CLASS as mtg-432/mtg-570 (count-only log-staleness). Two mechanisms, both now fixed:

1. PRE-REWIRE RATZILLA FACADE: at the live-observation deploy, native_game.html's network path likely still rendered via the ratzilla TUI (the #ratzilla-terminal facade, before the mtg-669 ratzilla-free rewire). That path uses the fancy-TUI LogWrapCache, whose count-only invalidation duplicated turn banners after a rewind+replay regrew the log (the mtg-432/mtg-570 root). Fixed by the LogWrapCache log_epoch invalidation (commit 54e84c31).

2. CURRENT NATIVE DOM RENDERER: native_game.html now renders the log itself via renderLog() from the shared GuiViewModel. It bailed with 'if (body.children.length === filtered.length) return;' — a COUNT-only staleness check, the JS analog of the Rust cache bug. A networked rewind+replay regrowing the log to the SAME count would skip the re-render, locking in a transiently-duplicated render. Fixed by switching to a per-entry content signature (text+color+bold) so renderLog re-renders on any content change, not just count change (commit b02597f0).

STATUS: both candidate root mechanisms fixed on branch fix-log-rendering; make validate green (validate_5e573daf...log). Cosmetic/UX only (no gameplay impact). Pending LIVE verification on a deployed 2-player native-GUI network game (the only place the original 3x was seen); the network e2e harness (web/test_network_*.js) exercises the renderLog path. Recommend close after a live deploy confirms no duplication, or reopen with a precise residual if it persists.
