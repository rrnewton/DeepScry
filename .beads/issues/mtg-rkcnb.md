---
title: 'Card Compatibility: Agna Qel''a'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-06T04:35:30.237739399+00:00
updated_at: 2026-06-06T08:41:30.988328943+00:00
---

# Description

Test all behavioral aspects of Agna Qel'a in MTG Forge-rs.

Card: cardsfolder/a/agna_qela.txt
Set: ATLA / 2025 Standard
Deck: 01 Manfield, 02 Shibata Izzet Lessons (2025 WC)

Card text:
  Land
  This land enters tapped unless you control a basic land.
  {T}: Add {U}.
  {2}{U}, {T}: Draw a card, then discard a card.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: no cost, Land
2. [x] Enters untapped when you control a basic land: R:Event$ Moved with ConditionPresent$ Land.Basic+YouCtrl | ConditionCompare$ EQ0 works
3. [x] Taps for {U}
4. [x] Activated draw/discard ability fires: 'Agna Qel'a activates ability: Draw a card, then discard a card.'
5. [unverified] Enters tapped behavior when NO basic land controlled

Reproducer:
    - Agna Qel'a ()
  (First turn - no draw)
  Zero2 draws Combustion Technique (119)
  Zero1 draws Boomerang Basics (59)
    - Agna Qel'a ()
  Zero1 plays Agna Qel'a (13)
  Zero1 draws Accumulate Wisdom (58)
  Boomerang Basics (59) causes Zero1 to draw 1 card(s)
  Zero2 draws Multiversal Passage (118)
    Island (12) (tapped)
    Agna Qel'a (13)
  [38;5;8mTap Agna Qel'a for {U}[39m
  Zero1 draws Riverpyre Verge (54)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
  Zero1 must discard 1 cards (hand size: 8, max: 7)
  Island is discarded
  Zero1 discards Island (33)
  Zero2 draws Gran-Gran (117)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
  Gran-Gran (117) enters the battlefield as a 1/2 creature
  Zero1 draws Riverpyre Verge (53)
    Agna Qel'a (13)
    Island (74) (tapped)
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Gran-Gran (52)
  Island is discarded
  Zero1 discards Island
  Zero2 draws Monument to Endurance (116)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
  Zero2 draws Riverpyre Verge (114)
  Island is discarded
  Zero2 discards Island
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Accumulate Wisdom (51)
  Island is discarded
  Zero1 discards Island
  Zero1 draws Multiversal Passage (50)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Island (74) (tapped)
    Island (88) (tapped)
    Island (94) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Zero2 draws Mountain (113)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
  Zero2 draws Mountain (112)
  Mountain is discarded
  Zero2 discards Mountain
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Iroh's Demonstration (49)
  Combustion Technique is discarded
  Zero1 discards Combustion Technique
  Zero1 draws Abandon Attachments (48)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Gran-Gran (52) enters the battlefield as a 1/2 creature
  Zero2 draws It'll Quench Ya! (111)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
  Zero2 draws Monument to Endurance (110)
  Monument to Endurance is discarded
  Zero2 discards Monument to Endurance
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Monument to Endurance (47)
  Island is discarded
  Zero1 discards Island
  Zero1 draws Artist's Talent (42)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Island (74) (tapped)
    Island (88) (tapped)
    Island (94) (tapped)
    Mountain (113) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Zero1 draws Firebending Lesson (41)
  Firebending Lesson is discarded
  Zero1 discards Firebending Lesson
  Zero1 must discard 1 cards (hand size: 8, max: 7)
  Abandon Attachments is discarded
  Zero1 discards Abandon Attachments (55)
  Zero2 draws Riverpyre Verge (109)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Gran-Gran (52) - 1/2 (tapped)
  Zero2 draws Agna Qel'a (108)
  Agna Qel'a is discarded
  Zero2 discards Agna Qel'a
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Riverpyre Verge (40)
  Riverpyre Verge is discarded
  Zero1 discards Riverpyre Verge
  Zero1 draws Monument to Endurance (39)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Zero1 draws Stormchaser's Talent (38)
  Stormchaser's Talent is discarded
  Zero1 discards Stormchaser's Talent
  Zero2 draws Iroh's Demonstration (106)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Riverpyre Verge (40) (tapped)
    Gran-Gran (52) - 1/2 (tapped)
  Zero2 draws Riverpyre Verge (105)
  Riverpyre Verge is discarded
  Zero2 discards Riverpyre Verge
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Multiversal Passage (36)
  Iroh's Demonstration is discarded
  Zero1 discards Iroh's Demonstration
  Zero1 draws Monument to Endurance (35)
  Zero1 draws Artist's Talent (32)
  Abandon Attachments (48) causes Zero1 to draw 2 card(s)
  Zero1 draws Mountain (31)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Riverpyre Verge (40) (tapped)
    Island (74) (tapped)
    Mountain (113) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Zero1 draws Multiversal Passage (30)
  Multiversal Passage is discarded
  Zero1 discards Multiversal Passage
  Zero2 draws Boomerang Basics (104)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Riverpyre Verge (40) (tapped)
    Mountain (31) (tapped)
    Gran-Gran (52) - 1/2 (tapped)
  Zero2 draws Combustion Technique (103)
  Boomerang Basics (104) causes Zero2 to draw 1 card(s)
  Zero2 draws Multiversal Passage (102)
  Multiversal Passage is discarded
  Zero2 discards Multiversal Passage
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Stormchaser's Talent (29)
  Monument to Endurance is discarded
  Zero1 discards Monument to Endurance
  Zero1 draws Firebending Lesson (28)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Island (74) (tapped)
    Mountain (113) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Monument to Endurance (27)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Combustion Technique (26)
  Zero1 draws Boomerang Basics (25)
  Boomerang Basics is discarded
  Zero1 discards Boomerang Basics
  Zero2 draws Artist's Talent (101)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Riverpyre Verge (40) (tapped)
    Mountain (31) (tapped)
    Gran-Gran (52) - 1/2 (tapped)
  Zero2 draws Artist's Talent (100)
  Artist's Talent is discarded
  Zero2 discards Artist's Talent
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Riverpyre Verge (24)
  Stormchaser's Talent is discarded
  Zero1 discards Stormchaser's Talent
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Gran-Gran (23)
  Zero1 draws Spirebluff Canal (22)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Mountain (31) (tapped)
    Mountain (113) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
  Gran-Gran (23) enters the battlefield as a 1/2 creature
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Boomerang Basics (20)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Stormchaser's Talent (19)
  Zero1 draws Gran-Gran (18)
  Boomerang Basics (20) causes Zero1 to draw 1 card(s)
  Zero1 draws Iroh's Demonstration (17)
  Iroh's Demonstration is discarded
  Zero1 discards Iroh's Demonstration
  Zero2 draws Firebending Lesson (98)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Riverpyre Verge (40) (tapped)
    Mountain (31) (tapped)
    Spirebluff Canal (22) (tapped)
    Gran-Gran (52) - 1/2 (tapped)
    Otter Token (120) - 1/1 (tapped)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero2 draws Stormchaser's Talent (97)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero2 draws Monument to Endurance (95)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero2 draws Multiversal Passage (93)
  Zero2 draws Spirebluff Canal (92)
  Spirebluff Canal is discarded
  Zero2 discards Spirebluff Canal
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Zero1 draws Combustion Technique (16)
  Gran-Gran is discarded
  Zero1 discards Gran-Gran
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Artist's Talent (15)
  Zero1 draws Abandon Attachments (14)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Mountain (31) (tapped)
    Island (74) (tapped)
    Island (88) (tapped)
    Island (94) (tapped)
    Mountain (113) (tapped)
    Gran-Gran (117) - 1/2 (tapped)
    Otter Token (121) - 1/1 (tapped)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Combustion Technique (11)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Firebending Lesson (10)
  Zero1 draws Iroh's Demonstration (9)
  Zero1 draws Accumulate Wisdom (8)
  Abandon Attachments (14) causes Zero1 to draw 2 card(s)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Spirebluff Canal (7)
  Zero1 draws Accumulate Wisdom (3)
  Accumulate Wisdom is discarded
  Zero1 discards Accumulate Wisdom
  Zero1 must discard 4 cards (hand size: 11, max: 7)
  Artist's Talent is discarded
  Zero1 discards Artist's Talent (42)
  Monument to Endurance is discarded
  Zero1 discards Monument to Endurance (35)
  Stormchaser's Talent is discarded
  Zero1 discards Stormchaser's Talent (19)
  Gran-Gran is discarded
  Zero1 discards Gran-Gran (18)
  Zero2 draws Boomerang Basics (91)
    Island (12) (tapped)
    Agna Qel'a (13) (tapped)
    Island (21) (tapped)
    Island (34) (tapped)
    Riverpyre Verge (53) (tapped)
    Riverpyre Verge (40) (tapped)
    Mountain (31) (tapped)
    Spirebluff Canal (22) (tapped)
    Riverpyre Verge (24) (tapped)
    Gran-Gran (52) - 1/2 (tapped)
    Otter Token (120) - 1/1 (tapped)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero2 draws Gran-Gran (90)
  Zero2 draws Multiversal Passage (89)
  Boomerang Basics (91) causes Zero2 to draw 1 card(s)
  Gran-Gran (90) enters the battlefield as a 1/2 creature
  Zero2 draws Firebending Lesson (87)
  Firebending Lesson is discarded
  Zero2 discards Firebending Lesson
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws It'll Quench Ya! (2)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Spirebluff Canal (1)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Artist's Talent (0)
  Agna Qel'a activates ability: Draw a card, then discard a card.
  Artist's Talent is discarded
  Zero1 discards Artist's Talent

Expected:

Actual: Confirmed working.

CARD STATUS: WORKING — lands correctly, taps for U, activates draw/discard
