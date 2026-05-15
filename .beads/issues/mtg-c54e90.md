---
title: 'Network desync: Seismic Sense triggers FATAL P2 state hash mismatch every time it resolves'
status: open
priority: 2
issue_type: task
created_at: 2026-05-14T14:27:33.959366580+00:00
updated_at: 2026-05-14T14:27:33.959366580+00:00
---

# Description

## Summary

Fuzz testing the network code (`bug_finding/network_fuzz_test.py`) on integration branch (tip fe820468) found that **every** state-hash desync we observed in 45 native↔native runs occurred immediately after a Seismic Sense resolution. 13/45 runs (29%) failed with a FATAL P2 state hash mismatch, and in 100% of those the last-resolved spell logged before the divergence is Seismic Sense — across many seeds and controller combinations.

## Reproducers

```bash
./tests/network_vs_local_equivalence_e2e.sh 2 heuristic heuristic
./tests/network_vs_local_equivalence_e2e.sh 7 heuristic random
./tests/network_vs_local_equivalence_e2e.sh 7 random random
./tests/network_vs_local_equivalence_e2e.sh 5 heuristic zero
```

(Decks: `decks/booster_draft/avatar/{ryan,gabriel}_avatar_draft.dck`, both contain 1 Seismic Sense.)

## Server log excerpt (network_fuzz_4g_edohl, seed=2 heuristic+heuristic, Turn 8)

```
[GAMELOG Turn8 M1] Gabriel casts Seismic Sense (69) (putting on stack)
[GAMELOG Turn8 M1] Tap Forest for {G}
[GAMELOG Turn8 M1] Seismic Sense (69) resolves
[GAMELOG Turn8 M1] Gabriel looks at the top 1 card of their library
[GAMELOG Turn8 M1] Gabriel puts Ostrich-Horse into Hand
[GAMELOG Turn8 M1] Seismic Sense (69) digs 1 card(s) from opponent's library to hand
NETWORK SYNC MISMATCH DETECTED - P2 choice_seq=84
Server hash: a37e19ca97a4d125  Client hash: a4653dfa26c28f5c
SERVER STATE: Hands: [4, 5]  Libs: [30, 25]  Hand CardIds: [65, 73, 77, 78, 79]
CLIENT STATE: Hands: [4, 4]  Libs: [30, 26]  Hand CardIds: [73, 77, 78, 79]
DIFFERENCES: Server has card 65 in P2 hand and lib_size 25; client has lib_size 26 and 4 hand cards.
```

The server applied a hidden zone change (top of own library → hand) but the client did NOT replay the same change. Server's library shrank by 1 and hand grew by 1; client's library still has the card and hand is one short.

## Card data

`forge-java/forge-gui/res/cardsfolder/.../seismicsense.txt` — Seismic Sense draws/digs effect that mixes 'look at top of own library', 'put into hand', and (per gamelog text) 'digs N cards from opponent's library to hand'. The two-zone (own library + opponent library) movement appears to confuse the controller's hidden-zone replay path.

## Why this matters

Per CLAUDE.md / docs/NETWORK_ARCHITECTURE.md desync is **always fatal** — the server/client model requires controllers to be information-independent. Single-card support for Seismic Sense in network mode is currently broken in a way that crashes the game whenever this card resolves.

## Possible relationship

Same family of bug as recently fixed Bazaar/Loot ChoicePoint(Discard) issue (commit cb67465c). Look at how the server records and replays the 'put on top of library / put into hand' decisions made by the active player when resolving Seismic Sense — the client likely never reaches the same ChoicePoint and so misses the zone update.

## Test data

Failure logs preserved at:
- /tmp/qa-fail-seed2-heuristic-heuristic
- /tmp/qa-fail-seed7-h-r
- /tmp/qa-fail-coord-exit (seed=7 random+random)

## Discovered by

`bug_finding/network_fuzz_test.py --configs 45 --parallel 3` on branch `qa-fuzz-testing` at `fe820468` 2026-05-14.
