---
title: 'TODO: Find and fix our leaky tests'
status: open
priority: 2
issue_type: task
labels:
- human
created_at: 2025-10-27T13:51:48+00:00
updated_at: 2026-06-01T13:23:57.944878374+00:00
---

# Description

## TODO: Find and fix our leaky tests

GARDENING (2026-06-01): possibly-stale, needs human/code re-check — the original report was vague ('some tests are leaking memory') with no specific test identified and no reproduction steps. The investigation concluded 'needs more information'. No leak has been identified or confirmed. If memory issues surface concretely in CI or profiling, file a new focused issue. Until then, this is low-confidence.

## Investigation Status (from original filing)

**Needs more information:**
- No specific test identified as leaking
- No reproduction steps provided
- Thread-local storage is used in card.rs:PARSING_FILE_CONTEXT and wasm/ modules

## Next Steps (if revived)

1. Need specific report details: which test(s), how much memory, how observed?
2. Consider adding memory tracking to test harness
3. Run tests under heaptrack/dhat with nextest
4. Check if any tests create large game states repeatedly without cleanup
