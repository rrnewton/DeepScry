---
title: 'Perf: web random/random games are slow even at full speed (near-instant controller)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-06T00:19:33.358631818+00:00
updated_at: 2026-06-06T00:19:33.358631818+00:00
---

# Description

User (2026-06-06, on deployed site): even with the near-instant random controller and 'full speed' auto-run, web random/random games are 'still pretty slow.' Investigate + profile where the per-action wall-clock goes. CANDIDATES to measure: (1) console-log spam — a log::info!/console.log PER ACTION captured by the bug_report.js shim in the hot loop (see mtg-q2hts; gate it and re-measure — likely a big chunk); (2) per-action rewind/replay cost on the WASM shadow (unwind_state_sync_to + reconstruct passes run frequently?); (3) full GuiViewModel JSON serialize + parse every tick (mtg-6vzht render-skip helps render but not the serialize — see mtg-pio60 slim-serialize, mtg-v039x O(1) serialize-skip); (4) per-tick DOM/render work; (5) WS round-trip latency per choice (auto-pass submits + waits for ack every priority). METHOD: add timing around the per-action loop (native + wasm), profile a full-speed random/random game, attribute the time. Then optimize the dominant cost. Likely synergizes with mtg-q2hts (log gating), mtg-pio60/mtg-v039x (serialize). NOT urgent (user said 'later') but real.
