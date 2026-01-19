---
title: Julian Avatar Deck Compatibility
status: open
priority: 1
issue_type: task
created_at: 2026-01-17T23:09:03.931957063+00:00
updated_at: 2026-01-18T05:31:41.640025448+00:00
---

# Description

## Julian Avatar Deck Compatibility Tracking

This issue tracks compatibility testing for the **julian_avatar_deck.dck** deck (UG).

## Engine Implementation Gaps Summary

Key gaps affecting this deck:
- **T:Mode$ Drawn** - "When you draw your second card" triggers (multiple cards)
- **T:Mode$ Taps** - "Whenever ~ becomes tapped" triggers (Gran-Gran)
- **T:Mode$ Phase** - Beginning of combat triggers (Avatar Kyoshi)
- **Count$YouDrewThisTurn** - Track cards drawn this turn (Messenger Hawk)
- **Count$Valid** - Count creatures matching filter (Elephant-Mandrill)
- **Exhaust$ True** - Once-per-game activation (Rebellious Captives)
- **Ward:Waterbend<N>** - Ward with Waterbend cost (The Unagi)
- **S:Mode$ RaiseCost** - Additional sacrifice costs (Tectonic Split)
- **K:Affinity:Ally** - Cost reduction (Allies at Last)
- **DB$ EachDamage** - Multiple sources dealing damage (Allies at Last)

---

## Deck Card Verification Checklist

### Lands (17):
- [x] Island (x10) - basic land (works)
- [x] Forest (x7) - basic land (works)

### Creatures (15):

**Avatar Kyoshi, Earthbender (x1)** - 8GGG 6/6 Legendary Human Avatar
- [ ] Conditional hexproof during your turn (S:Mode$ Continuous | Condition$ PlayerTurn)
- [ ] Beginning of combat trigger earthbend 8 (T:Mode$ Phase | Phase$ BeginCombat)
- [ ] Untap the earthbended land (SubAbility$ DBUntap)
- GAP: Phase triggers, conditional static abilities

**Elephant-Mandrill (x1)** - 2G 3/2 Elephant Monkey
- [x] Reach keyword (VERIFIED 2026-01-19)
- [x] ETB each player creates Food token (VERIFIED 2026-01-19: "Created Food Token under Player 1's control", "Created Food Token under Player 2's control" - TokenOwner$ Player now implemented!)
- [ ] Beginning of combat pump based on opponent artifacts (Count$Valid Artifact.OppCtrl)
- GAP: Count$Valid for variable pump

**Forecasting Fortune Teller (x2)** - 1U 1/3 Human Advisor Ally
- [x] ETB create Clue token (VERIFIED 2026-01-18: "Created Clue Token under Player 1's control")

**Giant Koi (x1)** - 4UU 5/7 Fish
- [x] Islandcycling {2} (VERIFIED 2026-01-19: "Player 1 uses Islandcycling on Giant Koi")
- [ ] Waterbend 3: Can't be blocked (AB$ Effect with Waterbend cost)
- GAP: AB$ Effect with StaticAbilities$ Unblockable

**Gran-Gran (x1)** - U 1/2 Legendary Human Peasant Ally
- [ ] Taps trigger: draw then discard (T:Mode$ Taps)
- [ ] Cost reduction static (S:Mode$ ReduceCost based on Lessons in graveyard)
- GAP: Taps trigger mode, ReduceCost static

**Knowledge Seeker (x2)** - 1U 2/1 Fox Spirit
- [x] Vigilance (should work)
- [ ] Second card drawn trigger: put +1/+1 counter (T:Mode$ Drawn | Number$ 2)
- [x] Dies trigger: create Clue token (VERIFIED 2026-01-18: "Created Clue Token under Player 1's control" - fixed death trigger bug for state-based lethal damage)
- GAP: T:Mode$ Drawn trigger

**Otter-Penguin (x3)** - 1U 2/1 Otter Bird
- [ ] Second card drawn trigger: pump +1/+2 and unblockable (T:Mode$ Drawn)
- GAP: T:Mode$ Drawn, DB$ Effect unblockable

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
- [ ] Flash (should work)
- [ ] Ward—Waterbend {4} (K:Ward:Waterbend<4>)
- [ ] Opponent draws second card trigger (T:Mode$ Drawn | ValidPlayer$ Opponent)
- GAP: Ward:Waterbend, Drawn trigger

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
- [x] SP$ Charm with CopyPermanent modes (VERIFIED 2026-01-18: "Created a token copy of Grizzly Bears (as 4/4 Hero copy)")

**Messenger Hawk (x1)** - 2UB 1/2 Bird Scout
- [x] Flying (VERIFIED 2026-01-19)
- [x] ETB create Clue token (VERIFIED 2026-01-19: "Created Clue Token under Player 1's control", Clue even activates to draw)
- [ ] Static pump based on cards drawn this turn (Count$YouDrewThisTurn)
- GAP: Count$YouDrewThisTurn

**Meteor Sword (x1)** - 7 Artifact Equipment
- [x] ETB destroy target permanent (VERIFIED 2026-01-18: destroyed Canyon Crawler)
- [x] Equipped creature gets +3/+3 (VERIFIED 2026-01-18: Grizzly Bears 2/2 → 5/5)
- [x] Equip {3} (VERIFIED 2026-01-18: equip ability works)

**Pillar Launch (x1)** - 1R Instant
- [x] Pump + Untap SubAbility (VERIFIED in Gabriel deck - SubAbility$ DBUntap with Defined$ Targeted fixed)

**Tectonic Split (x1)** - 4GG Enchantment
- [ ] Additional cost: sacrifice half lands (S:Mode$ RaiseCost | Cost$ Sac<X/Land>)
- [ ] Hexproof (should work)
- [ ] Lands gain triple mana ability (S:Mode$ Continuous | AddAbility$)
- GAP: RaiseCost, AddAbility for lands

---

## Verified Cards Summary (11/40)

Working cards:
1. **Island** - basic land
2. **Forest** - basic land
3. **Turtle-Duck** - AB$ Animate
4. **Pillar Launch** - Pump + SubAbility
5. **Forecasting Fortune Teller** - ETB Clue token
6. **Ember Island Production** - SP$ Charm + CopyPermanent
7. **Meteor Sword** - Equipment with ETB destroy, equip, +3/+3 bonus
8. **Knowledge Seeker** - Dies trigger creates Clue token (partial - Drawn trigger still needs work)
9. **Messenger Hawk** - Flying + ETB Clue token (partial - Count$YouDrewThisTurn pump needs work)
10. **Giant Koi** - Islandcycling works (partial - Waterbend unblockable ability needs work)
11. **Elephant-Mandrill** - Reach + ETB Food token for controller (partial - TokenOwner$ Player only affects controller, combat pump needs Count$Valid)

## Testing Protocol

1. ~~Test "likely to work" cards first with puzzles~~ DONE for 5 cards
2. Identify and implement missing mechanics
3. Run full deck vs deck games
4. Verify web GUI compatibility

## Related Issues
- mtg-5hvly: Gabriel Avatar Deck Compatibility
- mtg-0iad2: Ryan Avatar Deck Compatibility
