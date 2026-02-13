---
title: 'Network reveal sync: shadow game missing cards from hand'
status: open
priority: 2
issue_type: task
created_at: 2026-02-13T14:11:07.064662244+00:00
updated_at: 2026-02-13T14:11:07.064662244+00:00
---

# Description

## Summary

The client's shadow game sometimes doesn't have revealed cards that should be in the player's hand, causing ability computation mismatches.

## Observed Behavior

Error logs show:
```
ABILITY SYNC BUG - server has 3 abilities, local has 2
  Server abilities: ["CastSpell { card_id: 23 }", "CastSpell { card_id: 25 }", "CastSpell { card_id: 32 }"]
  Local abilities: ["CastSpell { card_id: 25 }", "CastSpell { card_id: 32 }"]
```

The server knows about CardId 23 (Rough Rhino Cavalry) in Ryan's hand, but the client's shadow game doesn't.

## Root Cause

The CardRevealed message for card 23 either:
1. Was never sent
2. Was sent but not processed before ability computation
3. Was processed but not applied correctly to shadow game state

## Impact

- Heuristic makes decisions based on incomplete information
- Causes M1/M2 timing divergence (same creature cast in different phases)
- ~20% of heuristic vs heuristic seeds fail equivalence test

## Workaround Attempted

Previously: Use server's ability list instead of local. 
Problem: This violates information independence and causes DIFFERENT divergence because heuristic evaluates different options.

Current: Always use local abilities, log sync bugs as errors.
Result: Better (80% pass rate), but underlying bug remains.

## Reproduction

```bash
./tests/network_vs_local_equivalence_e2e.sh 5 heuristic heuristic
## Check logs: grep "ABILITY SYNC BUG" /tmp/network_vs_local_e2e_*/network/client1.log
```

## Related

- mtg-secqu: Network architecture compliance tracking
- The card in question (Rough Rhino Cavalry) appears to have been in opening hand but reveal may have been lost
