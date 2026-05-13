---
title: 'Card Compatibility: Strip Mine'
status: open
priority: 3
issue_type: task
created_at: 2026-05-13T02:59:42.367982628+00:00
updated_at: 2026-05-13T02:59:42.367982628+00:00
---

# Description

Test all behavioral aspects of Strip Mine in MTG Forge-rs.

Card: cardsfolder/s/strip_mine.txt
Set: ATQ (Antiquities)

Card text:
  Strip Mine — Land
  {T}: Add {C}.
  {T}, Sacrifice Strip Mine: Destroy target land.

Findings (2026-05-12, compat2):

1. [x] Parses as Land (no mana cost)
2. [x] Mana ability {T}: Add {C} parses (Produced\$ C)
3. [x] Sacrifice activated ability parses with cost 'T Sac<1/CARDNAME>'
   - Cost is composite (NOT pure Cost::Tap or Cost::Mana — verified in
     test_card_compat_strip_mine).
4. [x] Activating destroys the target land AND sacrifices Strip Mine.
   - Gameplay evidence:
     'Strip Mine activates ability: Destroy target land.'
     '-> targeting Mountain (4)'
     'Strip Mine (3) goes to graveyard'
     'Mountain (4) goes to graveyard'

Note: With the Zero/Random controllers the auto-target picks the FIRST
valid land — which is often the controller's own land. This is a
controller-policy issue, not a card-correctness issue. The card
specifies AILogic\$ LandForLand for the smart-AI heuristic; that is a
follow-up for the heuristic AI to honour.

Reproducer:
  cargo build --release
  cat > /tmp/strip.pzl <<P
  [metadata]
  Name:Strip Mine destroys land
  Goal:Win
  Turns:3
  [state]
  turn=2
  activeplayer=p0
  activephase=MAIN1
  p0life=20
  p0battlefield=Strip Mine; Mountain
  p1life=20
  p1battlefield=Mountain; Mountain
  ...
  P
  ./target/release/mtg tui --start-state /tmp/strip.pzl --p1=zero --p2=zero --stop-on-choice=8 --seed 42 --verbosity verbose

Unit test: test_card_compat_strip_mine in mtg-engine/src/game/actions/tests/effects.rs

CARD STATUS: WORKING. Mana, sacrifice cost, and target-land destruction all verified.
