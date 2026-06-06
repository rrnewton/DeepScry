---
title: 'Card Compatibility: Stormchaser''s Talent'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:30:40.593964812+00:00
updated_at: 2026-06-06T04:30:40.593964812+00:00
---

# Description

Test all behavioral aspects of Stormchaser's Talent in MTG Forge-rs.

Card: cardsfolder/s/stormchasers_talent.txt
Set: ATLA (Avatar: The Last Airbender)
Deck: 01 Manfield Izzet Lessons, 02 Shibata Izzet Lessons (2025 WC)

Card text:
  U  Enchantment — Class
  Level 1: When this enters, create a 1/1 blue and red Otter creature token with prowess.
  {3}{U}: Level 2 — When this Class becomes level 2, return target instant or sorcery card from your graveyard to your hand.
  {5}{U}: Level 3 — Whenever you cast an instant or sorcery spell, create a 1/1 blue and red Otter creature token with prowess.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost U, Enchantment Class
2. [x] Level 1 ETB trigger fires: 'Created Otter Token under Zero1's control'
3. [BROKEN] Otter token has Prowess but PumpCreature trigger fizzles (mtg-b0igv): '[WARN pump] PumpCreature fizzled: unresolved target 0' whenever a noncreature spell is cast
4. [unverified] Level 2 class upgrade and InstantSorcery return trigger
5. [unverified] Level 3 class upgrade and SpellCast Otter token trigger

Reproducer:
    - Stormchaser's Talent (U)
  Zero1 casts Stormchaser's Talent (19) (putting on stack)
  [38;5;10mStormchaser's Talent (19) resolves[39m
  Created Otter Token under Zero1's control
  Zero1 declares Otter Token (120) (1/1) as attacker
  [38;5;9m[1mOtter Token (120) deals 1 damage to Zero2 (life: 19)[0m
    Otter Token (120) - 1/1 (tapped)
    Stormchaser's Talent (19)
    Otter Token (120) - 1/1
    Stormchaser's Talent (19)
  Trigger: Otter Token - [noncreature] Prowess (+1/+1 until end of turn)
[WARN  pump] PumpCreature fizzled: unresolved target 0
  Zero1 declares Otter Token (120) (1/1) as attacker
  [38;5;9m[1mOtter Token (120) deals 1 damage to Zero2 (life: 18)[0m
    Otter Token (120) - 1/1 (tapped)
    Stormchaser's Talent (19)
  Trigger: Otter Token - [noncreature] Prowess (+1/+1 until end of turn)
[WARN  pump] PumpCreature fizzled: unresolved target 0
    Otter Token (120) - 1/1
    Stormchaser's Talent (19)
  Zero1 declares Otter Token (120) (1/1) as attacker
  [38;5;9m[1mOtter Token (120) deals 1 damage to Zero2 (life: 17)[0m
    Otter Token (120) - 1/1 (tapped)
    Stormchaser's Talent (19)
    Otter Token (120) - 1/1
    Stormchaser's Talent (19)
  Zero1 declares Otter Token (120) (1/1) as attacker
  Zero2 declares Gran-Gran (117) as blocker for Otter Token (120)
  Otter Token (120) deals 1 damage to Gran-Gran (117)
  Gran-Gran (117) deals 1 damage to Otter Token (120)
  Otter Token (120) goes to graveyard
  Otter Token (120) dies from combat damage
    Stormchaser's Talent (19)
    Stormchaser's Talent (19)
    Stormchaser's Talent (19)
    Stormchaser's Talent (19)
    Stormchaser's Talent (19)
  Zero1 draws Stormchaser's Talent (46)
    - Stormchaser's Talent (U)
    Stormchaser's Talent (19)
  Zero1 casts Stormchaser's Talent (46) (putting on stack)
  [38;5;10mStormchaser's Talent (46) resolves[39m
  Created Otter Token under Zero1's control
  Zero1 declares Otter Token (121) (1/1) as attacker
  [38;5;9m[1mOtter Token (121) deals 1 damage to Zero2 (life: 14)[0m
    Otter Token (121) - 1/1 (tapped)
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
    Otter Token (121) - 1/1
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  Trigger: Otter Token - [noncreature] Prowess (+1/+1 until end of turn)
[WARN  pump] PumpCreature fizzled: unresolved target 0
  Zero1 declares Otter Token (121) (1/1) as attacker
  [38;5;9m[1mOtter Token (121) deals 1 damage to Zero2 (life: 12)[0m
    Otter Token (121) - 1/1 (tapped)
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  [38;5;9m[1mOtter Token (121) takes 1 damage (total: 1)[0m
  Otter Token (121) goes to graveyard
  Otter Token (121) dies from lethal damage
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  Zero1 draws Stormchaser's Talent (38)
  Stormchaser's Talent is discarded
  Zero1 discards Stormchaser's Talent
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Zero1 draws Stormchaser's Talent (29)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
    - Stormchaser's Talent (U)
    Stormchaser's Talent (19)
    Stormchaser's Talent (46)
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  Trigger: Artist's Talent - Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.

Expected log:


CARD STATUS: PARTIAL — ETB Otter token creation works; Prowess on Otter is broken (mtg-b0igv); Class level upgrades unverified
