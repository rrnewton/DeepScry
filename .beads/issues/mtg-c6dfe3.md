---
title: 'Card Compatibility: Serra Angel'
status: open
priority: 3
issue_type: task
created_at: 2026-05-13T02:45:29.437370320+00:00
updated_at: 2026-05-13T02:45:29.437370320+00:00
---

# Description

Test all behavioral aspects of Serra Angel in MTG Forge-rs.

Card: cardsfolder/s/serra_angel.txt
Set: LEA (Alpha)

Card text:
  Serra Angel {3}{W}{W}, 4/4 Creature - Angel
  Flying, Vigilance

Findings (2026-05-12, compat2):

1. [x] Parses as 4/4 Creature - Angel, cost {3}{W}{W}
2. [x] Has Keyword::Flying on the parsed card
3. [x] Has Keyword::Vigilance on the parsed card
4. [x] Vigilance: Serra Angel does NOT tap when attacking
   - Gameplay evidence: After 'Serra Angel (8) deals 4 damage to Player 1', the next turn's battlefield display shows 'Serra Angel (8) - 4/4' (no '(tapped)' marker — contrast Black Knight which displays as 'Black Knight (3) - 2/2 (tapped)' after attacking).

Reproducer:
  cat > /tmp/sa.pzl <<P
  [metadata]
  Name:Serra Angel Vigilance
  Goal:Win
  Turns:3
  [state]
  turn=2
  activeplayer=p1
  activephase=MAIN1
  p0life=20
  p0battlefield=Black Knight; Swamp
  p1life=20
  p1battlefield=Serra Angel; Plains
  ...
  P
  ./target/release/mtg tui --start-state /tmp/sa.pzl --p1=zero --p2=zero --stop-on-choice=8 --seed 42 --verbosity verbose

Unit test: test_card_compat_serra_angel in mtg-engine/src/game/actions/tests/effects.rs

Existing combat tests already cover Serra Angel via load_test_card (test_vigilance_creature_stays_untapped in combat.rs); this test pins the parser-level shape so production card loading produces the same Card struct.

CARD STATUS: WORKING — parses correctly with both keywords, vigilance behavior verified by gameplay reproducer.
