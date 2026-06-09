---
title: 'Network desync: WASM P2 state-hash mismatch at Turn1 EndCombat ac=66 (post opening-hand-fix)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-09T17:29:12.990873341+00:00
updated_at: 2026-06-09T17:29:12.990873341+00:00
---

# Description

## Summary

WASM-vs-WASM network game (random controllers) hits a P2 state-hash mismatch at **Turn 1 "EndCombat", action_count=66, choice_seq=7**. This desync was previously MASKED by the opening-hand ac-0 reveal-drop bug (mtg-212, fixed e0c33784); once that fix let games run past the opening hand, this deeper divergence surfaced.

## Repro

```bash
cargo build --release --features network && make wasm-network
python3 bug_finding/fuzz.py network --client wasm --controllers random --seeds 4
## deck pair from decks/old_school/*.dck,decks/old_school2/*.dck (chain pairing), server seed 1
```
Saved logs: `debug/netarch_fuzz/repro_p2_statehash_ac66/{server,client1,client2}.log` (gitignored).

## Diagnostic (server mismatch box)

```
P2 choice_seq=7  action_count=66
Server hash: 92c4ed0e9693354a  Client hash: 84c90bf66463727b
Turn 1 "EndCombat" active=0
Life: [20, 20]  Hands: [7, 7]  Libs: [53, 53]
Battlefield: 0  Stack: 0  Graveyards: [0, 0]
Hand CardIds(known): P0=[53..59] P1=[113..119]
Library CardIds(sorted): P0=[0..52] P1=[60..112]
```

## Analysis

The structural state shown in the summary is BYTE-IDENTICAL between server and shadow (zones, life, hands, libraries all match). The full `compute_state_hash` nevertheless diverges, so the divergent field is one NOT printed in the summary box — candidates to bisect:
- `TurnStructure` priority/consecutive_passes/priority_player state at the EndCombat step boundary
- `CombatState` residue not cleared identically across the shadow's replay vs the server
- a per-turn counter (`cards_drawn_this_turn`, lands-played) 
- continuous-effects / layer state
- RNG `word_pos` — but the rewind-verifier hash already excludes server-only rng (mtg-559/mtg-610); the FULL network hash may still include it and the shadow's random controller may have advanced its rng differently. NOTE: both sides are `random` controllers; a divergence in how many rng draws each side made by EndCombat would desync.

Recommended next step: add a field-level state-hash diff at the mismatch point (the rewind verifier already has `post_rewind_state_snapshot` JSON diffing — wire an analogous server-vs-client JSON field diff into the `--network-debug` mismatch path) to pinpoint the exact divergent field, rather than bisecting blind. The game reaches EndCombat of turn 1 with an empty board, so the divergence is in turn-structure/priority/rng bookkeeping at a combat-step transition, NOT in card mechanics.

## Status

OPEN. Distinct from mtg-212 (opening-hand reveal, FIXED). Tracked under mtg-429 (network fuzz). This is likely the "known-hard WASM-rewind class" — the shadow advances via rewind+replay and a turn-structure/rng field drifts at the EndCombat boundary that the native oracle does not reproduce.
