---
title: 'Card Compatibility: Eddymurk Crab'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:36:19.240193711+00:00
updated_at: 2026-06-06T08:41:19.497135946+00:00
---

# Description

Test all behavioral aspects of Eddymurk Crab in MTG Forge-rs.

Card: cardsfolder/e/eddymurk_crab.txt
Set: ATLA / 2025 Standard
Deck: 03 Davis Izzet Lessons (2025 WC)

Card text:
  5UU  Creature — Elemental Crab  [5/5]
  Flash
  This spell costs {1} less to cast for each instant and sorcery card in your graveyard.
  Eddymurk Crab enters tapped if it's not your turn.
  When Eddymurk Crab enters, tap up to two target creatures.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 5UU, Creature Elemental Crab, 5/5
2. [x] K:Flash keyword present
3. [unverified] Cost reduction: S:Mode$ ReduceCost | Amount$ X | SVar:X:Count$ValidGraveyard Instant.YouOwn,Sorcery.YouOwn — affected by Count$ValidGraveyard bug (mtg-cedrg)
4. [unverified] ETBReplacement: enters tapped if not your turn (K:ETBReplacement:Other:LandTapped with ConditionPlayerTurn$ False)
5. [unverified] ETB tap trigger: "tap up to two target creatures"

Note: Too expensive to cast in basic test (5UU minus graveyard discount), never casts in zero-vs-zero game without graveyard setup.

CARD STATUS: PARTIAL — parses correctly; cost reduction depends on Count$ValidGraveyard (broken, mtg-cedrg); ETB abilities unverified

## Update (2026-06-06):
mtg-cedrg (Count$ValidGraveyard) is now FIXED. Eddymurk Crab's cost reduction (depends on graveyard count) should now work correctly. Needs re-testing.

CARD STATUS: PARTIAL — cost reduction mechanism should now work; ETB abilities still unverified
