---
title: Flaky network equivalence test (~50% failure rate on HEAD)
status: closed
priority: 2
issue_type: task
labels:
- bug,network,flaky
created_at: 2026-01-26T19:58:32.967954641+00:00
updated_at: 2026-03-09T21:13:14.935614056+00:00
---

# Description

## Flaky network equivalence test (~50% failure rate on HEAD)

**RESOLVED 2026-03-09_#1887**

The flaky network equivalence test has been verified as passing consistently:
- Tested 10 seeds (1-10) with random/random controllers: 10/10 pass
- Tested 10 seeds (1-10) with heuristic/heuristic controllers: 10/10 pass

Previous failures were likely fixed by commits:
- Fix network reveal synchronization 
- Add missing token/copy reveals to prevent FATAL DESYNC (16ba523)
- Other network fixes merged since the issue was filed

Network equivalence tests are now reliable.
