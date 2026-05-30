---
title: 'Network determinism BUG: local-vs-network gamelog divergence (rogue_rogerbrand vs thedeck, random, seed=1)'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-30T19:44:10.485328566+00:00
updated_at: 2026-05-30T19:46:42.110534656+00:00
---

# Description

DUPLICATE of mtg-586 (reopened) — same local-vs-network determinism divergence: random, 01_rogue_rogerbrand vs 02_thedeck_peterschnidrig, seed=1, 641-line gamelog diff, load-sensitive. mtg-586 now carries this exact reproducer + the expedition-only validate-scoping decision. Preserved artifact: experiments/netequiv_desync_rogue_thedeck_s1_20260530/ (gamelog.diff + local/server gamelogs + REPRODUCER.txt). Investigate under mtg-586; related mtg-589 (WASM-shadow rogerbrand family). Closing here to avoid duplicate tracking.
