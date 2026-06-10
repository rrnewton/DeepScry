---
title: Intermittent robots42 seed-3 failure — AI Lightning Bolt mana-tap infinite loop (network-equiv flake recurrence)
status: open
priority: 2
issue_type: task
created_at: 2026-06-10T02:36:41.491425574+00:00
updated_at: 2026-06-10T02:53:41.922681236+00:00
---

# Description



## UPDATE 2026-06-10 — confirmed INTERMITTENT WITHIN A SINGLE COMMIT (priority raised)

Now observed 3 times across different branches/seeds/test-harnesses, past the 'investigate on 3rd flake' threshold:

1. b3281a33 run 27248463211: `network.robots42` FAIL seed=3 → rerun PASS.
2. ef853204 (web-only: launcher.html/solo_launcher.html) run 27248544560: the SAME deck failed in a DIFFERENT harness — `unit` job's nextest `shell_scripts__robots42_state_sync_e2e` FAILED at seed=42, while the SAME run's `network-equiv` job's `network.robots42` PASSED. Same SHA, same CI run, robots42 run twice: one pass, one fail.

=> This is NON-DETERMINISTIC WITHIN A SINGLE COMMIT'S CI — definitively not load/timing flake, not seed-specific, not caused by any of these web-only branches. It is a real intermittent engine/AI defect in the robots42 (1994 '03 Robots Jesseisbak', mtg-559) path: the AI repeatedly attempts Lightning Bolt, taps a mana rock (Fellwar Stone) for the wrong color, fails to pay {R}, and loops to the 1000-action priority guard.

IMPACT: blocks the follow-up landing wave — ANY branch's CI can randomly hit robots42 and go red regardless of that branch's content. Needs a real fix (deterministic mana-source color selection / AI mana-payment) before the wave can land reliably. Recommend bumping priority and dispatching an engine-side investigation; the web/harness branches are NOT the cause.

NOTE: robots42 runs in TWO validate steps with different seed sets — network.robots42 (network-equiv job; seeds 3/7/19/42) AND shell_scripts__robots42_state_sync_e2e (unit/nextest job). A fix must stabilize both.
