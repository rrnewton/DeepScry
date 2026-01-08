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

## Debugging Work Done (2026-01-08)

### Added reveal validation (actions.rs)
- `validate_cards_revealed()` function checks all hand cards are revealed
- Gated by `network_debug` flag - only runs when debugging is enabled
- Skips validation for opponent's cards (hidden info architecture: mtg-qtqcr)
- Immediate panic if card not revealed - follows linear transfer of control model
- No retries/waiting - missing reveals indicate protocol bug, not timing issue

### Added GameLoop support for network mode (mod.rs)
- `local_player_id` field to identify which player we are
- `with_reveal_validation(player_id, enabled)` builder method - `enabled` should be `network_debug`
- Updated client.rs to gate validation on `network_debug` flag

### Key findings:
1. Test times out intermittently regardless of validation
2. Hidden info architecture confirmed working - opponent's cards show as "Unknown"
3. Attempted "delayed OpponentMadeChoice" approach (send with reveals from next ChoiceRequest) but caused deadlocks
4. The server's `reveal_pusher` is never configured - reveals only bundled with ChoiceRequest
5. Timeout appears to happen around Library of Alexandria activations (drawing cards)

### Theories ruled out:
- NOT caused by validation itself (times out with validation disabled too)
- NOT caused by timing - linear model means all reveals should be processed before needed

## Investigation Needed

1. Enable network_debug mode and capture state hash comparisons
2. Add detailed logging around reveal processing
3. Identify which specific game action causes the 5-action gap
4. Compare server and client undo_log entries at divergence point
5. Consider implementing `reveal_pusher` on server to send reveals immediately after effects

## Reproducer

```bash
## Run multiple times - flaky test
cargo test --features network --test network_e2e test_run_game_with_random_controllers -- --ignored
```

## Related

- mtg-1jtoy: Network desync from reveal ordering
- mtg-to96y: Main networking tracking issue
