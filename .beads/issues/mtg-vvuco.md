---
title: 'Flaky network desync: white_weenie seed=7 at choice_seq=214'
status: open
priority: 3
issue_type: task
created_at: 2026-03-31T16:29:25.915613697+00:00
updated_at: 2026-03-31T16:29:25.915613697+00:00
---

# Description

Flaky network desync with white_weenie.dck seed=7. Native P2 client state hash mismatches server at choice_seq=214, action_count=1003. Intermittent (~20% failure rate). Not WASM-specific. HashMap/HashSet migration completed but still fails. Reproduction: cd web && node test_network_gui_e2e.js --deck decks/white_weenie.dck --seed 7 (run 5+ times).
