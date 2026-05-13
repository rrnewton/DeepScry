---
title: 'Card Compatibility: Triskelion'
status: closed
priority: 2
issue_type: task
created_at: 2026-05-13T02:20:41.577217174+00:00
updated_at: 2026-05-13T02:44:07.901977138+00:00
closed_at: 2026-05-13T02:44:07.901977038+00:00
---

# Description

FIXED 2026-05-12 (compat1).

Set: ATQ (mtg-07ff20)
Deck: rogue_rogerbrand (mtg-526f25)
Card script: cardsfolder/t/triskelion.txt

Root cause: Triskelion's activated ability has cost 'SubCounter<1/P1P1>' (remove a +1/+1 counter), but the cost parser at mtg-engine/src/core/costs.rs only recognized 'SubCounter<N/LOYALTY>' (planeswalker minus abilities). Other counter types fell through to the catch-all and returned None, which caused parse_activated_abilities() to skip the ability entirely. Result: Triskelion appeared on the battlefield as 4/4 but the activated ability was never offered.

Fix:
1. mtg-engine/src/core/costs.rs: added Cost::SubCounter { amount, counter_type } variant + parser. SubCounter<N/LOYALTY> still routes to Cost::SubLoyalty for backwards compatibility (planeswalker once-per-turn rule and 0-loyalty-dies still apply); SubCounter<N/X> for any other CounterType produces Cost::SubCounter.
2. mtg-engine/src/core/costs.rs: added Cost::get_sub_counter_requirement() helper for affordability checks.
3. mtg-engine/src/game/game_loop/actions.rs: ability-availability filter now consults get_sub_counter_requirement() and disables the ability if source card lacks the required counters.
4. mtg-engine/src/game/actions/mod.rs: Cost::SubCounter payment removes the counters via card.remove_counter() with proper logging. Unlike SubLoyalty, no once-per-turn restriction and no 0-counter-death.
5. Added 2 unit tests + 1 e2e shell test.

Behavioral aspects verified:
1. [x] Castable for {6} as Artifact Creature Construct
2. [x] Base P/T 1/1, ETBs with 3 +1/+1 counters → 4/4
3. [x] SubCounter<1/P1P1> cost validates: cannot activate when counters = 0
4. [x] Activation deals 1 damage to ANY target (ValidTgts$ Any)
5. [x] After activations, Triskelion's counters and effective P/T decrease
6. [x] Self-targeting works (suicidal use)
7. [unverified] Multiple-target split-activation (would need scripted test)
8. [x] Targeting respects shroud/hexproof (inherited from generic targeting)
9. [x] Game log shows counter removal AND damage dealt
10. [unverified] AILogic$ Triskelion behavior
11. [verified by absence of bug] No summoning-sickness gate (cost has no Tap)

Reproducer:
  ./target/release/mtg tui --start-state test_puzzles/triskelion_pings.pzl --p1=fixed --p2=zero --p1-fixed-inputs='activate Triskelion' --stop-on-choice=4 --json --seed 42 --verbosity 3

Expected output (excerpt):
    [1] activate Triskelion
    Triskelion activates ability: It deals 1 damage to any target.
      -> targeting Triskelion (3)   <-- (fixed controller picks first; heuristic AI picks better)
    Triskelion loses 1 P1P1 counter(s) (now 2)
    Triskelion (3) takes 1 damage (total: 1)

Regression test: tests/triskelion_subcounter_cost_e2e.sh

CARD STATUS: WORKING. SubCounter cost is generic enough to also enable cards like Lightning Coils, Pentavus, Walking Ballista (P1P1 ping abilities) and any Vehicle/non-loyalty counter-removal abilities.
