---
title: Network fuzz test bugs with random/zero controllers
status: open
priority: 2
issue_type: task
labels:
- bug
created_at: 2026-01-17T17:39:49.809606340+00:00
updated_at: 2026-01-20T10:11:08.382519689+00:00
---

# Description

## Network Fuzz Test Bugs

## CRITICAL PRINCIPLE: Desync is ALWAYS a Fatal Error

The random/zero controller failures in fuzz testing are **real bugs** that must be fixed properly.
Any desynchronization between server and client is an immediate, fatal error - we do NOT paper
over desync with recovery hacks.

The `spell_ability` field in `ChoiceResponse` is for **validation/early detection only**. If the
index-based selection and ability-based selection don't match, we crash immediately with a clear
error message. We do NOT use the extra data to "recover" from inconsistent state.

## Latest Results (2026-01-19_#1731(0af0092))

**Pass Rate**: 25% (5/20)
- heuristic vs heuristic: **100% (5/5)** ✅ FIXED!
- heuristic vs random: 0%
- heuristic vs zero: 0%
- random vs heuristic: 0%

## Key Progress

**OpponentChoice routing bug is FIXED** - The split MVar architecture (commit 2e58443) correctly routes local/remote choices.

## Current Issues (random/zero controllers only)

### 1. timeout (45% - 9 occurrences)
Random/zero controllers cause game to hang waiting for invalid choices.

### 2. connection_reset / entity_not_found (15% - 3 occurrences)
"Entity not found: 0" when certain cards resolve with random controller.

### 3. handler_exit_unexpected (10% - 2 occurrences)
Handler exits unexpectedly when game state diverges due to invalid choices.

### 4. Error declaring attacker (5% - 1 occurrence)
"Creature must be on battlefield to attack" - random controller tries to attack with non-existent creature.

## Root Cause Analysis

The failures with random/zero controllers stem from:
1. **Index bounds issues** - Server logs show "Invalid choice index N (max M), clamping to 0"
2. **Game state divergence** - Clamped choices cause client/server to have different game states
3. **Cascading failures** - Once states diverge, all subsequent choices have mismatched indices

**THE FIX IS NOT RECOVERY HACKS** - The fix is to identify WHY the client and server have
different views of available choices, and fix the root cause. Until we understand and fix
the real bug, we crash on desync.

## Why heuristic vs heuristic works

Heuristic controllers make deterministic, sensible choices based on game state evaluation. Both clients see the same game state and make the same choices, maintaining synchronization.

## Test Commands
```bash
## Heuristic vs heuristic (works!)
./tests/network_vs_local_equivalence_e2e.sh 1 heuristic heuristic

## Random controller failures (detect desync)
./tests/network_vs_local_equivalence_e2e.sh 7 heuristic random
```
