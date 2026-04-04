---
title: 'Server: File bug reports as GitHub issues via gh CLI'
status: open
priority: 0
issue_type: task
created_at: 2026-04-04T02:16:36.510135052+00:00
updated_at: 2026-04-04T02:16:36.510135052+00:00
---

# Description

Files: mtg-forge-rs/mtg-engine/src/server.rs (or new module for gh integration)

Action: After storing the bug report locally, use the gh CLI to file a GitHub issue:
1. Inline the user's description text in the issue body/description
2. Attach the log files (game_logs.txt, console_logs.txt) to the issue
3. Use appropriate labels/tags for the issue
4. Capture the URL of the created issue
5. Return the issue URL to the client via WebSocket response
6. Use with-proxy to prefix gh commands if they have trouble accessing the internet

Why: Bug reports need to be tracked as GitHub issues for visibility and follow-up.

Verify:
- gh issue create command is invoked with correct arguments
- Issue URL is returned to the client
- cargo build succeeds
- Test with a mock or real gh invocation
