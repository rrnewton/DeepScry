---
title: Julian Avatar Deck Compatibility
status: open
priority: 1
issue_type: task
created_at: 2026-01-17T23:09:03.931957063+00:00
updated_at: 2026-01-21T20:14:49.106547137+00:00
---

# Description

## Julian Avatar Deck Compatibility Tracking

This issue tracks compatibility testing for the **julian_avatar_deck.dck** deck (UG).

## Engine Implementation Gaps Summary

Key gaps affecting this deck:
- ~~**T:Mode$ Drawn** - "When you draw your second card" triggers~~ **FULLY IMPLEMENTED 2026-01-19** (includes undo support, opponent draw triggers for Underworld Dreams)
- ~~**T:Mode$ Taps** - "Whenever ~ becomes tapped" triggers (Gran-Gran)~~ **FULLY IMPLEMENTED 2026-01-20** (includes DiscardCards effect)
- ~~**T:Mode$ Phase** - Beginning of combat triggers (Avatar Kyoshi)~~ **FIXED 2026-01-21** (SubAbility$ chain to DBUntap now followed)
- **Count$YouDrewThisTurn** - Track cards drawn this turn (Messenger Hawk)
- **Count$Valid** - Count creatures matching filter (Elephant-Mandrill)
- **Exhaust$ True** - Once-per-game activation (Rebellious Captives)
- **Ward:Waterbend<N>** - Ward with Waterbend cost (The Unagi)
- **S:Mode$ RaiseCost** - Additional sacrifice costs (Tectonic Split)
- **K:Affinity:Ally** - Cost reduction (Allies at Last)
- **DB$ EachDamage** - Multiple sources dealing damage (Allies at Last)
- ~~**DB$ Effect with StaticAbilities$** - Grant can't be blocked (Otter-Penguin, Giant Koi)~~ **FIXED 2026-01-21** (GrantCantBeBlocked placeholder resolution in CardDrawn triggers)

---

## Deck Card Verification Checklist

### Lands (17):
- [x] Island (x10) - basic land (works)
- [x] Forest (x7) - basic land (works)

### Creatures (15):

**Avatar Kyoshi, Earthbender (x1)** - 8GGG 6/6 Legendary Human Avatar
- [ ] Conditional hexproof during your turn (S:Mode$ Continuous | Condition$ PlayerTurn)
- [x] Beginning of combat trigger earthbend 8 (T:Mode$ Phase | Phase$ BeginCombat) **FIXED 2026-01-21**
- [x] Untap the earthbended land (SubAbility$ DBUntap) **FIXED 2026-01-21**
- GAP: Conditional static abilities (hexproof on your turn)

**Elephant-Mandrill (x1)** - 2G 3/2 Elephant Monkey
- [x] Reach keyword (VERIFIED 2026-01-19)
- [x] ETB each player creates Food token (VERIFIED 2026-01-19)
- [ ] Beginning of combat pump based on opponent artifacts (Count$Valid Artifact.OppCtrl)
- GAP: Count$Valid for variable pump

**Forecasting Fortune Teller (x2)** - 1U 1/3 Human Advisor Ally
- [x] ETB create Clue token (VERIFIED 2026-01-18)

**Giant Koi (x1)** - 4UU 5/7 Fish
- [x] Islandcycling {2} (VERIFIED 2026-01-19)
- [x] Waterbend 3: Can't be blocked (AB$ Effect with Waterbend cost) **FIXED 2026-01-21** (via GrantCantBeBlocked)
- GAP: Waterbend mana cost parsing (partial - the "can't be blocked" effect works, cost needs verification)

**Gran-Gran (x1)** - U 1/2 Legendary Human Peasant Ally
- [x] Taps trigger: draw then discard (VERIFIED 2026-01-20)
- [ ] Cost reduction static (S:Mode$ ReduceCost based on Lessons in graveyard)
- GAP: ReduceCost static

**Knowledge Seeker (x2)** - 1U 2/1 Fox Spirit
- [x] Vigilance (should work)
- [x] Second card drawn trigger: put +1/+1 counter (VERIFIED 2026-01-19)
- [x] Dies trigger: create Clue token (VERIFIED 2026-01-18)

**Otter-Penguin (x3)** - 1U 2/1 Otter Bird
- [x] Second card drawn trigger: pump +1/+2 (VERIFIED 2026-01-19)
- [x] "Can't be blocked" effect from second draw trigger **FIXED 2026-01-21** (GrantCantBeBlocked placeholder resolved to self)

**Raucous Audience (x1)** - 1R 2/2 Human Rebel
- [ ] Mana ability with conditional (already known GAP: Count$Compare)
- GAP: Count$Compare

**Rebellious Captives (x1)** - 1G 2/2 Human Peasant Ally
- [ ] Exhaust {6}: Put counters + earthbend 2 (Exhaust$ True)
- GAP: Exhaust keyword (once-per-game activation)

**Teo, Spirited Glider (x1)** - 3U 1/4 Legendary Human Pilot Ally
- [ ] Flying (should work)
- [ ] Flying attackers trigger: draw/discard (T:Mode$ AttackersDeclared)
- [ ] Conditional counter on discard nonland (ImmediateTrigger, RememberDiscarded)
- GAP: AttackersDeclared trigger, ImmediateTrigger, RememberDiscarded

**The Unagi of Kyoshi Island (x1)** - 3UU 5/5 Legendary Serpent
- [x] Flash (VERIFIED 2026-01-19)
- [ ] Ward—Waterbend {4} (K:Ward:Waterbend<4>)
- [x] Opponent draws second card trigger (T:Mode$ Drawn | ValidPlayer$ Opponent) - works with 2026-01-19 Drawn trigger implementation
- GAP: Ward:Waterbend

**Turtle-Duck (x1)** - 1U 0/4 Turtle Bird
- [x] AB$ Animate ability (VERIFIED in Gabriel deck - mtg-5hvly)

### Spells/Other (8):

**Abandon Attachments (x1)** - 1UR Instant Lesson
- [ ] "You may discard. If you do, draw 2" (UnlessCost$ Discard | UnlessSwitched$ True)
- GAP: UnlessCost/UnlessSwitched optional cost

**Allies at Last (x2)** - 2G Instant
- [ ] Affinity for Allies (K:Affinity:Ally)
- [ ] Multi-target creature damage (DB$ EachDamage | ValidTgts$ Creature.OppCtrl)
- GAP: Affinity, EachDamage

**Ember Island Production (x1)** - 3UU Sorcery
- [x] SP$ Charm with CopyPermanent modes (VERIFIED 2026-01-18)

**Messenger Hawk (x1)** - 2UB 1/2 Bird Scout
- [x] Flying (VERIFIED 2026-01-19)
- [x] ETB create Clue token (VERIFIED 2026-01-19)
- [ ] Static pump based on cards drawn this turn (Count$YouDrewThisTurn)
- GAP: Count$YouDrewThisTurn

**Meteor Sword (x1)** - 7 Artifact Equipment
- [x] ETB destroy target permanent (VERIFIED 2026-01-18)
- [x] Equipped creature gets +3/+3 (VERIFIED 2026-01-18)
- [x] Equip {3} (VERIFIED 2026-01-18)

**Pillar Launch (x1)** - 1R Instant
- [x] Pump + Untap SubAbility (VERIFIED in Gabriel deck)

**Tectonic Split (x1)** - 4GG Enchantment
- [ ] Additional cost: sacrifice half lands (S:Mode$ RaiseCost | Cost$ Sac<X/Land>)
- [ ] Hexproof (should work)
- [ ] Lands gain triple mana ability (S:Mode$ Continuous | AddAbility$)
- GAP: RaiseCost, AddAbility for lands

---

## Verified Cards Summary (18/40 +3 fixed)

Working cards:
1. **Island** - basic land
2. **Forest** - basic land
3. **Turtle-Duck** - AB$ Animate
4. **Pillar Launch** - Pump + SubAbility
5. **Forecasting Fortune Teller** - ETB Clue token
6. **Ember Island Production** - SP$ Charm + CopyPermanent
7. **Meteor Sword** - Equipment with ETB destroy, equip, +3/+3 bonus
8. **Knowledge Seeker** - Vigilance + Dies trigger + Second draw trigger (FULLY WORKING 2026-01-19)
9. **Messenger Hawk** - Flying + ETB Clue token (partial - Count$YouDrewThisTurn pump needs work)
10. **Giant Koi** - Islandcycling works + "can't be blocked" effect **FIXED 2026-01-21** (partial - Ward:Waterbend needs work)
11. **Elephant-Mandrill** - Reach + ETB Food for ALL players (partial - combat pump needs Count$Valid)
12. **The Unagi** - Flash + Opponent Drawn trigger works (partial - Ward:Waterbend needs work)
13. **Otter-Penguin** - Second draw trigger for +1/+2 pump + "can't be blocked" **FULLY WORKING 2026-01-21**
14. **Gran-Gran** - Taps trigger for draw/discard works (VERIFIED 2026-01-20) (partial - ReduceCost static needs work)
15. **Avatar Kyoshi** - BeginCombat trigger with Earthbend + Untap **FIXED 2026-01-21** (partial - conditional hexproof needs work)

## Recent Fixes (2026-01-21)

1. **Otter-Penguin can't be blocked effect**: Fixed GrantCantBeBlocked placeholder resolution in CardDrawn trigger execution - target now correctly resolves to self
2. **Avatar Kyoshi BeginCombat trigger**: Fixed SubAbility$ chain to follow DBUntap after Earthbend effect

## Testing Protocol

1. ~~Test "likely to work" cards first with puzzles~~ DONE for 5 cards
2. Identify and implement missing mechanics
3. Run full deck vs deck games
4. Verify web GUI compatibility

## Related Issues
- mtg-5hvly: Gabriel Avatar Deck Compatibility
- mtg-0iad2: Ryan Avatar Deck Compatibility
