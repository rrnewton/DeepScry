---
title: 'Bug: global ETBTapped replacement — model nonBasic/Snow/nonPhyrexian/conditional qualifiers (1994 B12 follow-up)'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-03T17:24:57.927643516+00:00
updated_at: 2026-06-03T17:24:57.927643516+00:00
---

# Description

Follow-up to mtg-713 B12 / mtg-5xn5n (Kismet). The structured global ETB-tapped replacement (CardDefinition::etb_tapped_global) only installs predicates whose ValidCard$ qualifiers are controller restrictions we model (OppCtrl/YouCtrl/Any), with ActiveZones Battlefield and no IsPresent/ValidCause. Predicates with qualifiers TargetRestriction::parse silently DROPS are deliberately REFUSED (left no-op) to avoid over-matching (e.g. tapping basic lands / your own creatures).

Cards still no-op (the gated set), with the missing construct:
  - nonBasic land qualifier: Thalia Heretic Cathar, Archon of Emeria, Zhao the Moon Slayer
  - Snow subtype qualifier:  Reidane, God of the Worthy
  - nonPhyrexian qualifier:  Phyrexian Censor
  - cmcNotChosenEvenOdd:     Ashling's Prerogative (chosen even/odd quality)
  - IsPresent$ Card.Self+tapped + a paired ETBUntapped replacement: Archelos, Lagoon Mystic
  - ValidCause$ (played-by-opponent): Uphill Battle
  - ActiveZones$ Command + ValidCard$ Creature: The Doctor's Childhood Barn (background)
  - EnchantedPlayerCtrl:     Radiant Grace // Radiant Restraints

TODO: extend TargetRestriction (or the ETB matcher) to model nonBasic, Snow (and generic subtype on a dotted qualifier), nonPhyrexian, and the conditional (IsPresent$) / cause (ValidCause$) gating; then drop the gate in loader/card.rs::classify_etb_tapped_replacement for those forms. Add per-card e2e coverage.
