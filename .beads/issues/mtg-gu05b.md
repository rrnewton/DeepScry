---
title: Swap trivial decks (counterspells/simple_bolt) out of rewind+multideck test matrices for complex decks
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T18:46:49.513861400+00:00
updated_at: 2026-06-02T18:46:49.513861400+00:00
---

# Description

USER 2026-06-02 (deferred until netarch mtg-610 merges green): the network multideck gate (web/test_network_multideck.js SCENARIOS) and the new whole_game_rewind_replay_e2e.rs matrix lean on TRIVIAL decks (counterspells.dck, simple_bolt.dck) that are too simple to stress the undo log / desync paths. Once netarch lands on integration (robots42 4/4 green), REPLACE counterspells (and de-emphasize simple_bolt) with a more complex deck that exercises real interaction depth — combat tricks + activated abilities + multi-step resolutions (candidate: an old-school deck like rogerbrand or a championship deck). Apply to BOTH matrices. Coordinator to propose the replacement deck(s) to the user before locking in. Relates mtg-610 (netarch), mtg-559.
