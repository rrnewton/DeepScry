---
title: 'Card Compatibility: Chaos Orb'
status: open
priority: 2
issue_type: task
created_at: 2026-05-13T01:35:05.721026174+00:00
updated_at: 2026-05-13T02:17:30.712629430+00:00
---

# Description

Card Compatibility: Chaos Orb (LEA / mtg-3c7c63) — partial → mostly working.

Updated 2026-05-12 (compat1) after fixing mtg-4c1696.

Behavioral aspects verified:
1. [x] Card loads (parser produces non-empty effect list for activated ability)
2. [x] Card enters battlefield as Artifact
3. [x] Activation cost {1}, {T} pays
4. [x] FlipOntoBattlefield API resolves
5. [x] DestroyAll / FlipOntoBattlefield restriction NOW destroys an opponent permanent (was: self) — see mtg-4c1696
6. [x] Self-destroy: Chaos Orb still destroys itself via the Defined$ Self subability chain ('Chaos Orb (3) goes to graveyard')
7. [unverified] DBCleanup ClearRemembered semantics
8. [partial] requires_nontoken filter is honored (tokens excluded)
9. [x] Chaos Orb still destroys itself even when no opponent permanents exist (subability chain runs unconditionally)
10. [partial] Game log shows clear targeting + destroy sequence
11. [unverified] Heuristic AI evaluation quality

Reproducer: ./target/release/mtg tui --start-state test_puzzles/chaos_orb_destroys_target.pzl --p1=fixed --p2=zero --p1-fixed-inputs='activate Chaos Orb' --stop-on-choice=4 --json --seed 42 --verbosity 3
Expected output:
  Chaos Orb activates ability: ...
    -> targeting Mountain (16)
  Mountain (16) goes to graveyard
  Chaos Orb (3) goes to graveyard

Regression test: tests/chaos_orb_targets_opponent_e2e.sh

CARD STATUS: WORKING (digital approximation). The original card text is 'destroy all nontoken permanents it touches' (random per physical flip). Java Forge approximates with RNG over nearby permanents; mtg-forge-rs picks ONE opponent permanent per activation. Acceptable for play; future improvement could randomize over all-permanent set.
