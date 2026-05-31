---
title: 'Bug: Black Vise — Count$ValidHand + ETBReplacement ChoosePlayer unsupported (upkeep punisher deals 0)'
status: open
priority: 3
issue_type: task
created_at: 2026-05-31T00:22:18.345143728+00:00
updated_at: 2026-05-31T00:22:18.345143728+00:00
---

# Description

Black Vise (cardsfolder/b/black_vise.txt, mtg-486) now FIRES its upkeep trigger (after the wave14 phase-trigger DealDamageToTriggeredPlayer fix for Karma) but deals 0 damage because two pieces are missing:

Script:
  K:ETBReplacement:Other:ChooseP
  SVar:ChooseP:DB\$ ChoosePlayer | Defined\$ You | Choices\$ Player.Opponent | ChoiceTitle\$ Choose an opponent | AILogic\$ MostCardsInHand
  T:Mode\$ Phase | Phase\$ Upkeep | ValidPlayer\$ Player.Chosen | TriggerZones\$ Battlefield | Execute\$ TrigDamage
  SVar:TrigDamage:DB\$ DealDamage | Defined\$ ChosenPlayer | NumDmg\$ X
  SVar:X:Count\$ValidHand Card.ChosenCtrl/Minus.4

Two engine gaps:

1. **Count\$ValidHand <selector>/Minus.N** — CountExpression only supports
   Count\$Valid (battlefield permanents), Count\$xPaid, Count\$YouDrewThisTurn,
   Count\$YouCastThisTurn, Count\$Compare. It does NOT support counting cards in
   a HAND zone, nor the trailing /Minus.4 arithmetic modifier. Today this
   parses to Fixed(0) so Black Vise always deals 0. Need a CountExpression
   variant for "cards in <player>'s hand" plus the /Minus.N (and /Plus.N)
   post-modifiers (Java applies these via the Count\$ "/Minus.N" suffix).

2. **K:ETBReplacement ChoosePlayer / ValidPlayer\$ Player.Chosen** — the
   "as ~ enters, choose an opponent" replacement is not modeled, so there is
   no per-card "chosen player" stored, and ValidPlayer\$ Player.Chosen /
   Defined\$ ChosenPlayer cannot resolve. For the 2-player case the chosen
   player is always the single opponent, but the general fix needs the
   ChoosePlayer ETB to record a chosen PlayerId on the permanent and the
   phase-trigger ValidPlayer\$ Player.Chosen gate + Defined\$ ChosenPlayer
   resolution to read it.

Until both land Black Vise stays PARTIAL: it parses with the correct
{1} Artifact shape and its trigger fires, but the damage is 0.

The wave14 DealDamageToTriggeredPlayer effect (loader/card.rs phase-trigger
DealDamage branch) ALREADY routes Defined\$ ChosenPlayer to the active player
and evaluates the count against them — so once (1) and (2) land, Black Vise
should work with no further loader changes.

Found during compat-wave14-jeskai (Jeskai Aggro, mtg-561).
