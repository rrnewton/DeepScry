---
title: WASM client desyncs after combat damage when a creature dies (separate from Seismic Sense)
status: open
priority: 2
issue_type: task
created_at: 2026-05-14T15:07:48.235624408+00:00
updated_at: 2026-05-14T15:07:48.235624408+00:00
---

# Description

## Summary

When a player runs as a WASM client (`--client mixed`), state hash mismatches occur in the middle of the CombatDamage step on turns where a creature dies in combat. This is **distinct from** the Seismic Sense desync (mtg-c54e90) — these failures don't involve any hidden-zone effect; they're purely the combat damage/state-based-action sequence.

## Reproducers (mixed native↔WASM, --quick fuzz)

```
seed=2 p1=wasm/heuristic p2=native/heuristic   FAIL P1 hash mismatch  (turn 13, Master Piandao game)
seed=5 p1=native/heuristic p2=wasm/heuristic   FAIL P2 hash mismatch  (turn 13, Raucous Audience game)
seed=?? Fire Sages turn 24 case (network_fuzz_851efjqc)
```

Run: `python3 bug_finding/network_fuzz_test.py --quick --client mixed --parallel 2` (5/10 fail).

## Server log excerpt (Raucous Audience, turn 13, /tmp/qa-fail-wasm-raucous)

```
[GAMELOG Turn13 CD] Earth Kingdom Soldier (31) deals 4 damage to The Boulder, Ready to Rumble (72)
[GAMELOG Turn13 CD] Ostrich-Horse (68) deals 4 damage to Earth Kingdom Soldier (31)
[GAMELOG Turn13 CD] The Boulder, Ready to Rumble (72) deals 4 damage to Earth Kingdom Soldier (31)
[GAMELOG Turn13 CD] Ostrich-Horse (34) deals 4 damage to Ryan (life: 12)
[GAMELOG Turn13 CD] Earth Kingdom Soldier (31) dies from combat damage
[GAMELOG Turn13 CD] The Boulder, Ready to Rumble (72) dies from combat damage
NETWORK SYNC MISMATCH DETECTED - P2 choice_seq=158
```

## Server log excerpt (Fire Sages, turn 24)

```
[GAMELOG Turn24 DB] Ryan declares Fire Sages (22) as blocker for Forest (79)
[GAMELOG Turn24 CD] Forest (79) deals 3 damage to Fire Sages (22)
[GAMELOG Turn24 CD] Fire Sages (22) deals 2 damage to Forest (79)
[GAMELOG Turn24 CD] Fire Sages (22) dies from combat damage
NETWORK SYNC MISMATCH DETECTED - P2 choice_seq=289
```

The critical detail is that the server sees a creature death and applies state-based actions; the WASM client either applies them differently or in a different order. Forest (79) is *not* a creature in real MTG — that line might indicate a parser bug exposing a creature-from-land. But even setting that aside, two independent runs (Master Piandao, Raucous Audience, Fire Sages) all desync the moment combat damage assignment+SBA ticks in a turn where a creature dies.

## Why it's WASM-specific

Pure native↔native runs in the same fuzz suite never desync at combat damage — they only desync at Seismic Sense (mtg-c54e90). Mixed native↔WASM runs add this new failure mode.

Possible cause: WASM client's combat damage / state-based-action path differs (ordering of triggers, dies-leaves-battlefield triggers, last-known-info snapshots, etc.). See `mtg-engine/src/wasm/network_client.rs` and the SBA loop in the client controller.

## Flakiness note

One mixed run also showed FLAKY behavior (3/3 pass on rerun of the originally-failed seed=1 wasm/native). This suggests a timing / message-ordering race in WasmNetworkClient as well — not all WASM desyncs are deterministic.

## Test data

- /tmp/qa-fail-wasm-firesages
- /tmp/qa-fail-wasm-piandao
- /tmp/qa-fail-wasm-raucous

## Discovered by

`bug_finding/network_fuzz_test.py --quick --client mixed --parallel 2` on `qa-fuzz-testing` @ fe820468, 2026-05-14.
