---
title: 'Shadow state desync: triggered abilities with hidden info'
status: open
priority: 2
issue_type: task
created_at: 2026-01-08T15:59:50.778794868+00:00
updated_at: 2026-01-08T16:00:12.503782863+00:00
---

# Description

## Summary

Client shadow games cannot properly track opponent's triggered abilities that involve:
1. Hidden information (e.g., milling cards from library)
2. Player choices based on hidden information

This causes progressive shadow state desync, eventually leading to invalid choice indices.

## Root Cause

When a triggered ability like Ostrich-Horse's ETB fires:
1. Server mills cards (hidden from opponent)
2. Server asks player for choice (put land in hand or counter on creature)
3. Server applies the chosen effect

The OPPONENT's client cannot simulate this because:
- Mill results are hidden (opponent can't know what was milled)
- Choice depends on hidden info (opponent can't know what lands were available)
- The resulting game state differs based on unobservable choices

## FIXME Already Documented

In mtg-engine/src/network/client.rs:328-331:
```rust
// FIXME-UNFINISHED: Should replay the choice on our shadow state to keep
// it in sync with the server. Currently the client shadow state diverges
// from server state after opponent choices.
```

## Evidence

From seed 1 network test:
1. Turn 16: P2 casts Ostrich-Horse (68)
2. ETB trigger should mill 3 cards and optionally return land to hand
3. Trigger execution not logged - not processed on client shadow
4. P2 casts Raucous Audience (62) - card NOT in hand listing
5. Shortly after: Invalid choice index 4 (max 2), clamping to 0

## Reproducer

```bash
./debug/test_network_seeds.sh
## Seeds 1 and 2 hang (timeout), seed 3 completes
## Check /tmp/mtg_seed_test_1/server.log for Invalid choice index
```

## Related

- mtg-1jtoy: Network desync from reveal ordering (different root cause)
- mtg-to96y: Main networking tracking issue
