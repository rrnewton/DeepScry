---
title: 'Card Compatibility: Torch the Tower'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:36:07.737676006+00:00
updated_at: 2026-06-06T04:36:07.737676006+00:00
---

# Description

Test all behavioral aspects of Torch the Tower in MTG Forge-rs.

Card: cardsfolder/t/torch_the_tower.txt
Set: WOE (Wilds of Eldraine)
Deck: 01 Manfield Izzet Lessons (sideboard)

Card text:
  R  Instant
  Bargain (You may sacrifice an artifact, enchantment, or token as you cast this spell.)
  Torch the Tower deals 2 damage to target creature or planeswalker. If this spell was bargained, instead it deals 3 damage to that permanent and you scry 1.
  If a permanent dealt damage by Torch the Tower would die this turn, exile it instead.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost R, Instant, K:Bargain
2. [unverified] Bargain cost (sacrifice artifact/enchantment/token)
3. [unverified] 2 vs 3 damage based on bargain (Count$Bargain.3.2)
4. [unverified] Scry 1 if bargained
5. [unverified] Exile-if-dies replacement effect (RememberDamaged$ True, ReplaceDyingDefined$ Remembered)

CARD STATUS: PARTIAL — parses with Bargain keyword; bargain cost and conditional damage unverified
