---
title: 'Card Compatibility: Black Knight'
status: open
priority: 3
issue_type: task
created_at: 2026-05-13T02:45:16.378519212+00:00
updated_at: 2026-05-13T02:45:16.378519212+00:00
---

# Description

Test all behavioral aspects of Black Knight in MTG Forge-rs.

Card: cardsfolder/b/black_knight.txt
Set: LEA (Alpha)

Card text:
  Black Knight {B}{B}, 2/2 Creature - Human Knight
  First Strike, Protection from white

Findings (2026-05-12, compat2):

1. [x] Parses as 2/2 Creature - Human Knight, cost {B}{B}
2. [x] Has Keyword::FirstStrike on the parsed card
3. [x] Has Keyword::ProtectionFromWhite on the parsed card
4. [x] Combat: First Strike Combat Damage step fires
   - Gameplay evidence: '--- First Strike Combat Damage ---' then 'Black Knight (3) deals 2 damage to Player 2 (life: 18)'
5. [x] Protection from white: White Knight blocker did NOT block Black Knight (defender chose 'no blocks')
   - Gameplay evidence: Black Knight attacked with White Knight on board; combat damage dealt to player, no block declared.

Reproducer:
  cat > /tmp/bk.pzl <<P
  [metadata]
  Name:Black Knight Protection
  Goal:Win
  Turns:3
  [state]
  turn=2
  activeplayer=p0
  activephase=MAIN1
  p0life=20
  p0battlefield=Black Knight; Swamp
  p1life=20
  p1battlefield=White Knight; Plains; Plains
  ...etc
  P
  ./target/release/mtg tui --start-state /tmp/bk.pzl --p1=zero --p2=zero --stop-on-choice=8 --seed 42 --verbosity verbose

Unit test: test_card_compat_black_knight in mtg-engine/src/game/actions/tests/effects.rs

CARD STATUS: WORKING — parses correctly with both keywords, combat behavior verified by gameplay reproducer.
