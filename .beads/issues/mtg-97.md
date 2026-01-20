---
title: 'TODO: Find and fix our leaky tests'
status: open
priority: 2
issue_type: task
labels:
- human
created_at: 2025-10-27T13:51:48+00:00
updated_at: 2026-01-20T14:12:23.427319686+00:00
---

# Description

## Issue

We had a report that some of our tests are leaking memory. Investigate and fix.

## Investigation Status

**Needs more information:**
- No specific test identified as leaking
- No reproduction steps provided
- No memory profiling tools (valgrind) available in this environment
- Thread-local storage is used in:
  - `card.rs:PARSING_FILE_CONTEXT` - for parsing warnings (cleared after parsing)
  - `wasm/` modules - for WASM state (not relevant to native tests)

## Next Steps

1. Need specific report details: which test(s), how much memory, how observed?
2. Consider adding memory tracking to test harness
3. Check if any tests create large game states repeatedly without cleanup
4. May need to run tests under valgrind/heaptrack in a different environment

Marking as needs-info until more details are provided.
