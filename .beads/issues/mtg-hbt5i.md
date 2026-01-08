---
title: 'Shadow state desync: triggered abilities with hidden info'
status: open
priority: 2
issue_type: task
created_at: 2026-01-08T15:59:50.778794868+00:00
updated_at: 2026-01-08T16:58:44.964087382+00:00
---

# Description

## Summary

Network games experience intermittent desync with action_count mismatches or timeouts during complex gameplay.

## Previous (INCORRECT) Analysis

Previously diagnosed as "shadow state can't track opponent's triggered abilities with hidden information" - but this was WRONG. Mill results are NOT hidden - graveyard is a public zone. All zone changes should trigger CardRevealed messages.

## Current Understanding

The test `test_run_game_with_random_controllers` is flaky:
- Sometimes passes (in ~10-20 seconds)
- Sometimes times out (at Turn 10+)
- Sometimes fails with action_count mismatch (e.g., client=1785 expected=1790)

## Actual Root Cause: Unknown

The desync occurs during complex card interactions (Balance + Su-Chi death triggers, etc.) but the actual root cause is not yet identified. Possible causes:

1. **Reveal message ordering** - CardRevealed messages may arrive out of order relative to choices
2. **Trigger execution order** - Server and client may resolve triggers in different order
3. **Race conditions** - Messages crossing in the WebSocket handling
4. **Action logging differences** - Same logic producing different undo_log entries

## Investigation Needed

1. Enable network_debug mode and capture state hash comparisons
2. Add detailed logging around reveal processing
3. Identify which specific game action causes the 5-action gap
4. Compare server and client undo_log entries at divergence point

## Reproducer

```bash
## Run multiple times - flaky test
cargo test --features network --test network_e2e test_run_game_with_random_controllers -- --ignored
```

## Related

- mtg-1jtoy: Network desync from reveal ordering
- mtg-to96y: Main networking tracking issue
