---
title: 'Bug: Analyze the Pollen SubAbility condition not gating conditional ChangeZone search'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:31:18.859687838+00:00
updated_at: 2026-06-06T04:31:18.859687838+00:00
---

# Description

When Analyze the Pollen is cast WITHOUT collect-evidence (no cards exiled from graveyard), both the basic-land search and the creature/land search fire. The SubAbility$ DBChangeZone unconditionally chains even though DBChangeZone carries ConditionDefined$ Collected | ConditionPresent$ Card, which should gate execution on the evidence having been collected.

Root cause class: SubAbility condition evaluation gap — the engine follows SubAbility chains without evaluating ConditionDefined/ConditionPresent gates on the sub-chain. The main ability's ConditionCompare$ EQ0 (fire if no Collected tokens) is checked, but the SubAbility's Condition* parameters are ignored.

Cards affected: Analyze the Pollen (mtg-2c2sl), likely all cards that use conditional SubAbility chains where the sub-ability has ConditionDefined$ <token> | ConditionPresent$ Card as a guard.

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw 'Analyze the Pollen;Breeding Pool;Stomping Ground;Stomping Ground;Mountain' --p2-draw 'Island;Island;Island;Island;Island;Island;Island' --seed 42 --verbosity 3 decks/championship/2025/04_henry_temur_otters.dck decks/championship/2025/01_manfield_izzet_lessons.dck
```

Expected: Only ONE search (basic land) fires when no evidence collected.
Actual: TWO searches fire (basic land + creature/land).
