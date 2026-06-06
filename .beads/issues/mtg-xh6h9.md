---
title: 'Falling Star: EntityNotFound(u32::MAX) — TapPermanent reuse_previous with no previous target'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-06T01:35:43.024095937+00:00
updated_at: 2026-06-06T01:35:43.024095937+00:00
---

# Description

Found by the new consolidated fuzz expedition driver (bug_finding/fuzz.py expedition, mtg-813) sweeping the old_school2 corpus with random seeds in `determinism` mode.

INVARIANT: native same-seed determinism / no fatal game errors.

SYMPTOM (deterministic — both runs of the same seed crash identically):
  [GAMELOG TurnNN M1] Falling Star (10) resolves
  [WARN  resolve_effect] TapPermanent has reuse_previous but no previous target
  Error: EntityNotFound(4294967295)        # 4294967295 = u32::MAX sentinel (no target)
The process exits 1.

Falling Star's effect taps each creature it deals damage to (a TapPermanent sub-effect that uses `reuse_previous` to reference the just-damaged creatures). When there is no previous target to reuse (e.g. Falling Star dealt no damage / hit only players, or the previous-target list was empty), the TapPermanent resolves against the u32::MAX sentinel entity id and the engine errors with EntityNotFound instead of being a no-op.

REPRODUCER (deterministic):
  python3 bug_finding/fuzz.py expedition --modes determinism --decks 'decks/old_school2/ur_burn.dck,decks/old_school2/white_weenie_classic.dck' --controllers random --duration 60
  (the driver prints the exact failing seed + single-seed repro per finding; multiple old_school2 pairs reproduce it, e.g. ur_burn vs white_weenie_classic.)
  Equivalent single game:
  ./target/release/mtg tui <deckA> <deckB> --p1 random --p2 random --seed <SEED> --tag-gamelogs

LIKELY FIX AREA: the TapPermanent `reuse_previous` path in resolve_effect (the WARN is emitted right before the error) should treat "no previous target" as a no-op (tap zero permanents) rather than tapping the sentinel id. Needs an MTG rules review (Falling Star, CR 701.x tap; missing target → do nothing).

This is one of the concrete targets the mtg-813 1-hour-expedition prize must drive to zero. Filed from the consolidation worktree (consolidate-fuzz-infra-mtg-813).
