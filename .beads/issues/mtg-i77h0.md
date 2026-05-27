---
title: Network E2E test logs ERROR on clean shutdown after PASS
status: open
priority: 4
issue_type: bug
created_at: 2026-05-27T13:42:40.205262067+00:00
updated_at: 2026-05-27T13:42:40.205262067+00:00
---

# Description

After a network E2E test reports '=== TEST PASSED ===' and proceeds to close the browser, the server emits ERROR-level log lines for the clean WebSocket disconnect. These dilute 'grep ERROR' triage workflows.

## Example (from CI run 26482076564, Network E2E job, AFTER the assert passed)
```
=== TEST PASSED ===
[ERROR mtg_forge_rs::network::server] Game 1: P2 handler exited unexpectedly: Ok(Ok(()))
[ERROR mtg_forge_rs::network::server] Handler P0: Fatal error: Opponent disconnected
[ERROR mtg_forge_rs::network::server] Game 1: Error - P2 connection terminated unexpectedly
[ERROR mtg_forge_rs::network::client] WsReaderShared: Fatal error: Opponent disconnected
```

The test itself reports PASS — these are post-assertion shutdown artifacts.

## Recommended fix
Detect that the game has already reached terminal state when an unexpected disconnect arrives, and demote the log level to INFO/DEBUG in that case. Treat 'Ok(Ok(()))' from a handler as a normal close, not 'unexpectedly'.

## Discovery
Found during integration-branch triage 2026-05-27_#2297(b5cbdc85). See ai_docs/integration_triage_20260527.md (F4). Cosmetic / non-blocking.

## Related
- mtg-99og6 (CI status policy)
