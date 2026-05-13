---
title: 'Card Compatibility: City of Brass'
status: open
priority: 3
issue_type: task
created_at: 2026-05-13T02:59:25.173784565+00:00
updated_at: 2026-05-13T02:59:25.173784565+00:00
---

# Description

Test all behavioral aspects of City of Brass in MTG Forge-rs.

Card: cardsfolder/c/city_of_brass.txt
Set: ARN (Arabian Nights)

Card text:
  City of Brass — Land
  Whenever City of Brass becomes tapped, it deals 1 damage to you.
  {T}: Add one mana of any color.

Findings (2026-05-12, compat2):

1. [x] Parses as Land (no mana cost)
2. [x] Activated ability AB\$ Mana | Produced\$ Any parses
3. [BROKEN-FIXED] Produced\$ Any was hard-coded to colorless 'C'
   (effect_converter.rs:384). City of Brass effectively was a Colorless
   Source — could never satisfy a single-pip coloured cost. Fixed in
   this commit: Produced\$ Any now sets all five colour pips so the
   mana-production cache derivation classifies the card as
   ManaProductionKind::AnyColor.
4. [x] Mode\$ Taps trigger parses with TriggerEvent::Taps
5. [x] After fix: Tapping for mana produces colored mana AND fires
   the damage trigger.
   - Gameplay evidence:
     '[Tap City of Brass for {R}]'
     'Lightning Bolt (3) deals 3 damage to Player 2 (life: 14)'
     'Life: 19'  (Player 1 took 1 damage from City of Brass tap)

Reproducer:
  cargo build --release
  cat > /tmp/cob.pzl <<P
  [metadata]
  Name:City of Brass tap damage via random
  Goal:Win
  Turns:3
  [state]
  turn=2
  activeplayer=p0
  activephase=MAIN1
  p0life=20
  p0hand=Lightning Bolt
  p0library=Mountain
  p0graveyard=
  p0battlefield=City of Brass
  p1life=20
  p1hand=
  p1library=Mountain
  p1graveyard=
  p1battlefield=
  P
  ./target/release/mtg tui --start-state /tmp/cob.pzl --p1=random --p2=zero --stop-on-choice=10 --seed 42 --verbosity verbose

Unit test: test_card_compat_city_of_brass in mtg-engine/src/game/actions/tests/effects.rs (asserts cache.kind == AnyColor + tap_for_mana drops life to 19).

CARD STATUS: WORKING (after fix in this commit). Both colored mana production and tap-trigger damage to controller verified.
