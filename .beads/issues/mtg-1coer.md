---
title: 'Network gate harness: explicit two-deck matchups (don''t rely on implicit default opponent)'
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T20:29:43.139162522+00:00
updated_at: 2026-05-28T20:29:43.139162522+00:00
---

# Description

[user guardrail, 2026-05-28] When the network gate harness gains "single --deck applies to BOTH seats" (mirror match) behavior, make sure NO test relies on the OLD implicit behavior (single --deck set only native P1; browser P2 fell back to a DEFAULT deck). Tests that intend a CROSS-deck matchup must EXPLICITLY provide two decks, not silently collapse to deck-vs-default or deck-vs-itself.

Call-site audit (depth ~2390):
- web/test_network_multideck.js SCENARIOS: 4 scenarios each specify a SINGLE deck + seed:
  monored(13), old_school/01_rogue_rogerbrand(3), old_school/03_robots_jesseisbak(42), counterspells(5).
  Under the OLD behavior these ran X-vs-browser-DEFAULT-deck. With single->both-seats they become MIRROR matches (X-vs-X) -- a silent coverage CHANGE.
- Makefile:286, 816, 822 and .github/workflows/ci.yml:368 call `node test_network_gui_e2e.js` with NO --deck (default-vs-default).
- ci.yml:371 + Makefile:287 call test_network_multideck.js --quick (first 2 scenarios).
- tests/lib/test_helpers.sh already shows the NATIVE path takes two decks (`mtg tui deck1.dck deck2.dck`).

Required:
1. Add an explicit TWO-deck form to test_network_gui_e2e.js (e.g. --deck1/--deck2 or `--deck a,b`). Keep single --deck -> both seats as a convenience.
2. Update test_network_multideck.js scenarios to specify BOTH decks explicitly so each matchup is INTENTIONAL (decide per scenario: deliberate mirror, or a real cross-matchup with a named opponent deck). Do not rely on the implicit default opponent.
3. Make the bare Makefile/ci.yml gate calls explicit about which decks they exercise.

Coordinator: VERIFY this at the desync next-pass agent (aae5c68c) completion review; the agent owns the harness edit but may not cover the multideck-scenario explicitness. Apply/fix if it didn't. Relates to mtg-vk4b7 (the gate harness) + network e2e coverage.
