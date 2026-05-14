---
title: 'Network desync: Cycle ability (Mountaincycling) not visible to client; ABILITY SYNC BUG → FATAL DESYNC on Cycle vs CastSpell'
status: open
priority: 2
issue_type: task
created_at: 2026-05-14T15:19:22.526513712+00:00
updated_at: 2026-05-14T15:19:22.526513712+00:00
---

# Description

## Summary

The client's local shadow game does not enumerate Cycle (typecycling) abilities that the server enumerates, leading to:

1. `ABILITY SYNC BUG - server has 3 abilities, local has 2` warnings every priority pass
2. eventually a `FATAL DESYNC: Choice mismatch - index 1 selected Cycle { card_id: 31 ... search_type: Some(Subtype("Mountain")) }, but client expected CastSpell { card_id: 36 }` because the indexes diverge.

The client never offered the Cycle option to its controller (its action list was `[CastSpell { card_id: 36 }, CastSpell { card_id: 39 }]`), so when the server received "index 1" it believed CastSpell {36} was a Cycle command.

## Reproducer

```bash
cd mtg-forge-rs
./tests/network_vs_local_equivalence_e2e.sh 315 random random
```

Logs preserved at /tmp/qa-fail-cycle-sync.

## Server vs Client mismatch (from client1.log)

```
Server abilities: [
  Cycle { card_id: 31, cost: ManaCost { generic: 2 }, search_type: Some(Subtype("Mountain")) },
  CastSpell { card_id: 36 },
  CastSpell { card_id: 39 }
]
Local  abilities: [
  CastSpell { card_id: 36 },
  CastSpell { card_id: 39 }
]
```

The Cycle ability for card 31 (which has Mountaincycling per the deck contents) is not present in the local shadow's available-actions list.

## Likely root cause

Per CLAUDE.md, controllers must be information-independent and produce the same action list on server and client. Cycling is an activated ability that pays mana to discard the card and search for a basic land of the matching type — this should be enumerable on the client just from the public state (card in hand, mana available). Some part of the client's ability-enumeration path is omitting Cycle{Subtype} abilities.

Files to look at:
- `mtg-engine/src/network/local_controller.rs` (the place generating the warning)
- ability enumeration in the engine — whatever distinguishes server's enumeration from local's

## Discovered by

`bug_finding/network_fuzz_test.py --configs 30 --controller random` 2/30 occurrences with seed=315 random+random.
