---
title: 'Network e2e tests hang 180s: server lobby never exits after game completion'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-26T15:45:22.237454906+00:00
updated_at: 2026-05-26T15:45:22.237454906+00:00
---

# Description

## Summary

`tests/network_vs_local_equivalence_e2e.sh` and `tests/cycle_ability_network_sync_e2e.sh` (which wraps it) both time out at 180s on `integration` HEAD (`6db45ef1`). Reproduced 2026-05-26_#2286(6db45ef1).

## Root cause

Commit `67f046f0 feat(server): multi-game lobby with system-memory admission gate` (merged via `4e1a7f3d`) replaced single-game server lifecycle with a long-lived lobby. The server process now stays alive after a game completes, accepting new games.

`tests/network_vs_local_equivalence_e2e.sh` lines 240-267 polls `kill -0 $SERVER_PID` and treats server exit as 'network game done'. With the lobby server, `SERVER_PID` never exits, so the loop runs to its 180s timeout even though both clients have cleanly exited with the correct winner.

## Evidence

From `/tmp/network_vs_local_e2e_981411/network/server.log` (seed=3, zero/zero):

```
[INFO  mtg_forge_rs::network::server] Game 1: Completed, winner = Some(1), action_count = 1485
[INFO  mtg_forge_rs::network::server] Coordinator: Received GameEnded, winner=Some(1)
[INFO  mtg_forge_rs::network::server] Handler P1: Sending GameEnded winner=Some(1)
[INFO  mtg_forge_rs::network::server] Handler P0: Sending GameEnded winner=Some(1)
[INFO  mtg_forge_rs::network::server] Game 1 (default): completed
```

After this, server keeps listening; test times out 180s later.

Clients exit cleanly with correct outcome:
```
[INFO  mtg_forge_rs::network::client] Client GameLoop finished: winner=Some(1), action_count=1490
```

Same symptom for cycle test (seed=315 random/random) — server log ends with `Game 1 (default): completed` and stays alive.

## Fix options

1. **Test-side**: change wait loop to poll for BOTH client PIDs exiting instead of SERVER_PID. Server is now meant to outlive a game.
2. **Server-side**: add `--exit-on-game-completion` / `--single-game` flag for test harnesses; default to lobby mode in production.
3. Hybrid: clients exiting all connections triggers server shutdown when test flag set.

Option 1 is the minimal fix and matches reality (clients have authoritative end-of-game knowledge).

## Reproducer

```sh
git checkout 6db45ef1
git submodule update --init forge-java
cargo build --release --features network
bash tests/network_vs_local_equivalence_e2e.sh
## -> waits 180s then prints 'Error: Network game timed out after 180s'
```

## NOT a desync

Per CLAUDE.md 'Desync is ALWAYS Fatal' — this is NOT a desync. Both clients reached identical winner=Some(1) with the same action_count window. Game completed correctly; only the test harness misinterprets server lifecycle.

## Related

- mtg-c232f4 (separate snapshot bincode regression, also failing on same commit but unrelated)
- 67f046f0 (multi-game lobby commit, introduced the lifecycle change)
- 4e1a7f3d (merge of server-lobby into integration)
