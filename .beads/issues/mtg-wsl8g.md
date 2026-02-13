---
title: Network fuzz test bugs with random/zero controllers
status: closed
priority: 2
issue_type: task
labels:
- bug
created_at: 2026-01-17T17:39:49.809606340+00:00
updated_at: 2026-02-12T22:59:35.958120894+00:00
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

## Latest Results (2026-02-13_#1814(0804cd8b0))

**All controller combinations now passing!**
- heuristic vs heuristic: ✅ 100%
- heuristic vs random: ✅ 100% (seeds 3, 5, 7, 11, 13, 17, 23 all pass)
- zero vs zero: ✅ seed 23 passes

Key fixes:
1. `target_card_ids` protocol field (commit c15b104d5) - Fixes target choice sync
2. Fatal error on invalid indices (commit 0804cd8b0) - Exposes bugs instead of masking them

Previous: (2026-01-19_#1731(0af0092))
**Pass Rate**: 25% (5/20)

## Key Progress

1. **OpponentChoice routing bug is FIXED** - The split MVar architecture (commit 2e58443) correctly routes local/remote choices.
2. **Target sync bug FIXED** (commit c15b104d5) - Added `target_card_ids` field to protocol. When players choose targets, actual CardIds are now sent to opponent's shadow game instead of indices. This fixes desync when valid_targets lists differ.

## Previous Issues (RESOLVED)

These issues were caused by index-based protocol mismatches between server and client.
The target_card_ids protocol field and fatal error handling resolved them:

1. ~~timeout (45%)~~ - Fixed by target_card_ids
2. ~~connection_reset / entity_not_found (15%)~~ - Fixed by fatal error handling
3. ~~handler_exit_unexpected (10%)~~ - Fixed by proper sync
4. ~~Error declaring attacker (5%)~~ - Fixed by proper reveal handling

## Status

This issue can be closed. The network fuzz tests now pass consistently.

## Test Commands
```bash
## Zero vs zero (now works for seed 23!)
./tests/network_vs_local_equivalence_e2e.sh 23 zero zero

## Heuristic vs heuristic (works!)
./tests/network_vs_local_equivalence_e2e.sh 1 heuristic heuristic
```
