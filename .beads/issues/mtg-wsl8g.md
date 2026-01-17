---
title: Network fuzz test bugs with random/zero controllers
status: open
priority: 2
issue_type: task
labels:
- bug
created_at: 2026-01-17T17:39:49.809606340+00:00
updated_at: 2026-01-17T17:39:49.809606340+00:00
---

# Description

## Network Fuzz Test Bugs - Random/Zero Controllers

## Summary
Network fuzz testing with 50 configurations revealed multiple bugs when using random or zero controllers:
- **Pass rate**: heuristic vs heuristic = 100%, all other combinations = 0%
- **Total failures**: 45/50 (90%)

## Error Categories

### 1. entity_not_found / connection_reset (11 occurrences combined) - FIXED
**Root cause**: Seismic Sense (and similar Dig effects) crashed when accessing library cards that don't exist in the client's shadow state.

**Fix applied**: Effect::Dig implementation now uses `unwrap_or_else` to provide fallback card names when card data is missing in network mode.

**Reproducer**: `./tests/network_vs_local_equivalence_e2e.sh 2 heuristic random`

### 2. timeout (33 occurrences)
**Root cause**: Games hang indefinitely, likely after an error causes desync.

**Reproducer**: `./tests/network_vs_local_equivalence_e2e.sh 1 heuristic random`

### 3. creature_not_on_battlefield (1 occurrence)
**Root cause**: Random controller tries to attack with a creature that died or was never on battlefield.

**Reproducer**: `./tests/network_vs_local_equivalence_e2e.sh 5 heuristic random`

## Remaining Issues
- **timeout (33 occurrences)**: Root cause unknown, games hang
- **creature_not_on_battlefield (1 occurrence)**: Random controller validation issue

## Fixed in this commit
- `mtg-engine/src/game/actions/mod.rs` - Effect::Dig now handles missing card data gracefully

## Test Results Date
2026-01-17 (commit depth ~240)
