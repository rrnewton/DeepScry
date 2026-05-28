---
title: 'Network desync: All Hallow''s Eve mass-resurrection WASM-shadow sequencing (rogerbrand s3 P2 choice_seq=148)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T20:51:33.988492574+00:00
updated_at: 2026-05-28T20:51:33.988492574+00:00
---

# Description

## Summary

In NETWORK mirror play (both seats = `decks/old_school/01_rogue_rogerbrand.dck`,
server `--seed 3`), the WASM browser client (P2) desyncs from the server during
the **All Hallow's Eve** mass-resurrection upkeep trigger.

Reproducer (deterministic, fails every run):
```
node web/test_network_gui_e2e.js --deck decks/old_school/01_rogue_rogerbrand.dck --seed 3
```

## Precise divergence

- Through action_count=730 (end of Turn 12) server and client agree exactly
  (`gyard=[7,11] bf=11`, identical hashes, `local==server` action count).
- FATAL P2 state-hash mismatch at **choice_seq=148, action_count=741, Turn 13
  "Upkeep"** (server hash != client hash).
- At the SAME action_count=741:
  - SERVER: `Battlefield: 11  Graveyards: [7, 11]` (resurrection NOT yet applied).
  - CLIENT (WASM): `bf=15  gyard=[5,10]`, and `local action_count=747` (the
    client has run **6 actions ahead** and already applied the resurrection).
- i.e. the client front-loads the `ChangeZoneAll | Origin$ Graveyard |
  Destination$ Battlefield | ChangeType$ Creature` resurrection sub-ability
  earlier in the trigger chain than the server does; the eventual SET of moves
  matches but the per-action ORDER/TIMING differs, so the intermediate hash
  checkpoint diverges.

Card script (cardsfolder/.../all_hallows_eve.txt):
`T:Mode$ Phase | Phase$ Upkeep ... Execute$ TrigRemoveCounter`
-> `DBMoveToGraveyard` (ChangeZone Exile->Graveyard, ConditionPresent EQ0 SCREAM)
-> `DBResurrection` (ChangeZoneAll Graveyard->Battlefield Creature, same condition).

## NOT introduced by the mirror-match harness change

Confirmed pre-existing: `git stash` of the harness fix (test_network_gui_e2e.js
deck injection + tui_game.html hook) and re-running the SAME reproducer on the
prior non-mirror code still fails identically at choice_seq=148 action_count=741
(server=353df4d27265a637 client=9c746035ad588a7a). The mirror-match harness fix
(mtg-vk4b7) merely made it deterministic instead of matchup-dependent.

## Class

WASM-client shadow trigger/ChangeZoneAll sequencing (mtg-263 reveal/replay-timing
family). The fix must make the WASM shadow expand the All Hallow's Eve trigger
chain (RemoveCounter -> conditional self exile->graveyard -> conditional
ChangeZoneAll resurrection) into the SAME action sequence, at the SAME action
indices, as the server. Likely in the WASM client trigger-replay path, not the
core ChangeZoneAll executor (which iterates a deterministic Vec of player_zones).

## Related
mtg-vk4b7 (network desync tracker / mirror-match harness fix), mtg-390 (All
Hallow's Eve card compatibility), mtg-464870 (All Hallow's Eve triggers),
mtg-263 (WASM reveal timing), mtg-559 (robots deck — a separate mirror desync at
choice_seq=335 with seed 42, same WASM-shadow class).
