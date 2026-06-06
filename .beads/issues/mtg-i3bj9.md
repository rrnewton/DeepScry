---
title: 'Card Compatibility: Artist''s Talent'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:32:18.803924927+00:00
updated_at: 2026-06-06T04:32:18.803924927+00:00
---

# Description

Test all behavioral aspects of Artist's Talent in MTG Forge-rs.

Card: cardsfolder/a/artists_talent.txt
Set: ATLA / 2025 Standard
Deck: 01 Manfield, 02 Shibata Izzet Lessons (2025 WC)

Card text:
  1R  Enchantment — Class
  Level 1: Whenever you cast a noncreature spell, you may discard a card. If you do, draw a card.
  {2}{R}: Level 2 — Noncreature spells you cost cost {1} less to cast.
  {2}{R}: Level 3 — If a source you control would deal noncombat damage to an opponent or a permanent an opponent controls, it deals that much damage plus 2 instead.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 1R, Enchantment Class
2. [unverified] Level 1 SpellCast trigger with optional discard/draw
3. [unverified] Level 2 cost reduction static ability
4. [unverified] Level 3 +2 noncombat damage replacement effect
5. [NOTE] Class upgrade mechanics (K:Class:2:2 R, K:Class:3:2 R) support is partially implemented

Reproducer:


CARD STATUS: PARTIAL — Class mechanics partially supported; level 1 trigger unverified in game log
