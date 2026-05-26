---
title: 'Network e2e tests hang 180s: server lobby never exits after game completion'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-26T15:45:22.237454906+00:00
updated_at: 2026-05-26T20:53:43.904768809+00:00
---

# Description

## Description

## Summary

`tests/network_vs_local_equivalence_e2e.sh` and `tests/cycle_ability_network_sync_e2e.sh` (which wraps it) both time out at 180s on `integration` HEAD (`6db45ef1`). Reproduced 2026-05-26_#2286(6db45ef1).

## Root cause

Commit `67f046f0 feat(server): multi-game lobby with system-memory admission gate` (merged via `4e1a7f3d`) replaced single-game server lifecycle with a long-lived lobby. The server process now stays alive after a game completes, accepting new games.

`tests/network_vs_local_equivalence_e2e.sh` lines 240-267 polled `kill -0 $SERVER_PID` and treated server exit as 'network game done'. With the lobby server, `SERVER_PID` never exits, so the loop ran to its 180s timeout even though both clients had cleanly exited with the correct winner.

## RESOLUTION (2026-05-26, branch fix-mtg-ivrqv)

Fixed test-side per the issue's preferred option 1. Modified the wait loop in `network_vs_local_equivalence_e2e.sh` to poll BOTH client PIDs instead of the server PID. When both clients exit (clients hold authoritative end-of-game knowledge from the `GameEnded` message), the test kills the lobby server it spawned and proceeds to gamelog comparison.

Considered but rejected: adding a `--single-game` / `--exit-on-completion` flag to the production server CLI. That would be a production code change to accommodate a test-harness contract; the test harness should adapt to production lifecycle, not vice versa.

### Test deltas

- `network_vs_local_equivalence_e2e.sh` (seed=3, zero/zero): previously 180s timeout → now PASSES in ~20s with identical local/server gamelogs (157 entries each).
- `cycle_ability_network_sync_e2e.sh` (seed=315, random/random): previously 180s timeout → now runs in ~21s and *exposes a separate, previously-masked gamelog desync* (~232 differing lines). Filed as mtg-nufig. NOT in scope of this fix.

### Files changed

- `tests/network_vs_local_equivalence_e2e.sh` (wait-loop replacement + client-PID tracking)

## Related

- mtg-nufig (newly-filed: cycle desync regression exposed by this fix)
- mtg-c232f4 (separate snapshot bincode regression on same HEAD)
- 67f046f0 / 4e1a7f3d (multi-game lobby commit that introduced the lifecycle change)
