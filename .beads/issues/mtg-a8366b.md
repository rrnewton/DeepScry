---
title: 'Bug: Waterbend cost-availability check counted tapped lands'
status: closed
priority: 3
issue_type: task
created_at: 2026-05-11T01:06:30.412423198+00:00
updated_at: 2026-05-11T01:06:36.308990715+00:00
---

# Description

## Bug fixed: 2026-05-10_#2169(7baf01da)

**Symptom (bug-vinebender-triple-activation):** In native_game.html with the heuristic AI on both sides (eric_avatar_draft vs gabriel_avatar_draft, seed 42), the Foggy Swamp Vinebender appeared to activate its 'Waterbend 5: put a +1/+1 counter on this creature' ability multiple times after entering the battlefield, without paying the cost, and the +1/+1 counters never appeared on the creature.

**Root cause:** GameLoop::push_activatable_abilities (mtg-engine/src/game/game_loop/actions.rs around line 957) computes whether a Waterbend ability is currently affordable by counting the player's mana sources via mana_engine.all_sources(). The pre-fix filter was |s| s.card_id != card_id — it counted EVERY mana source on the battlefield, including ones that were already tapped or summoning-sick. Once the AI tapped its lands to pay for the first Waterbend activation, the same lands were still being counted as available payment capacity, so the ability re-appeared in the available-actions list.

**Fix:** Filter out tapped + summoning-sick sources in the affordability check.

**Regression tests:** mtg-engine/tests/vinebender_waterbend_test.rs (6 tests, all passing).

**Related:** mtg-6n8rl (Avatar set mechanics tracking)
