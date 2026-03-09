---
title: Network equivalence test seed 4 heuristic/random times out
status: closed
priority: 3
issue_type: task
labels:
- bug
created_at: 2026-02-11T17:57:00.851746907+00:00
updated_at: 2026-03-09T21:13:26.815613964+00:00
---

# Description

## Network equivalence test seed 4 heuristic/random times out

**RESOLVED 2026-03-09_#1887**

The timeout issue for seed 4 with heuristic/random controllers is now fixed:
- All 10 seeds (1-10) now pass with heuristic/heuristic controllers
- Seed 4 specifically completes in ~9 seconds instead of timing out

The underlying network sync and reveal issues have been fixed in recent commits. Closing as resolved.
