---
title: 'Bug: no damage-INCREASE replacement layer (DB$ ReplaceEffect VarName$ DamageAmount) — Artist''s Talent L3, Torbran'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-11T04:05:44.345585741+00:00
updated_at: 2026-06-11T04:05:44.345585741+00:00
---

# Description

Shared root-cause bug: the engine has NO damage-INCREASE replacement layer. Surfaced independently by the 2020 championship survey (Torbran, mtg-902 B1) and the 2025 championship survey (Artist's Talent Level 3, mtg-843 / mtg-881).

STAMP: 2026-06-10_#3175(c6dbd34f)

== Root cause ==
The DB$ ReplaceEffect | VarName$ DamageAmount | VarValue$ X construct (used to add +N to outgoing damage from a filtered source) has NO support:
  - ApiType::ReplaceEffect does not exist in the parser (mtg-engine/src/loader/ability_parser.rs — grep: zero hits for ReplaceEffect).
  - effect_converter.rs has no arm for it (zero hits for VarName / DamageAmount).
core/prevention.rs implements damage-PREVENTION replacements and combat.rs has a source-filtered prevention-shield path, but nothing applies a +N INCREASE to outgoing damage from a filtered source. The R: line that wires it up is dropped, and even if it resolved the DB$ ReplaceEffect sub-ability would fall through to Effect::Unimplemented.

== Card scripts hitting this ==
1. cardsfolder/a/artists_talent.txt — Artist's Talent (2025 decks 01,02). Level 3:
     K:Class:3:2 R:AddReplacementEffect$ DoubleDamage
     SVar:DoubleDamage:Event$ DamageDone | ValidSource$ Card.YouCtrl,Emblem.YouCtrl | ValidTarget$ Permanent.OppCtrl,Opponent | IsCombat$ False | ReplaceWith$ DmgPlus2
     SVar:DmgPlus2:DB$ ReplaceEffect | VarName$ DamageAmount | VarValue$ X
     SVar:X:ReplaceCount$DamageAmount/Plus.2
   "If a source you control would deal noncombat damage to an opponent or a permanent an opponent controls, it deals that much damage plus 2 instead." -> SILENTLY DROPPED. L1 (discard-draw) and L2 (cost reducer) WORK; L3 does nothing. => Artist's Talent is PARTIAL.
2. cardsfolder/t/torbran_thane_of_red_fell.txt — Torbran (2020 decks). "+2 to red-source damage to opponents." Same DmgPlus2/ReplaceCount construct. (mtg-902 B1.)

== Fix shape ==
Add a damage-MODIFICATION replacement category applied at the SINGLE damage-application chokepoint (both combat and ability/non-combat damage) so it cannot be bypassed. Deterministic + rewind-safe + identical server/client (no transient skip-fields). Filter comes from ValidSource / ValidTarget / IsCombat on the R: line. Generalizes to Torbran, Fiery Emancipation (x3), Gratuitous Violence (x2), City on Fire, etc. Game-logic change -> requires MTG rules review. DO NOT START until the mtg-245 execute_effect refactor lands (it touches the effect path that would collide). COORDINATE with any agent editing combat.rs / effect_converter.rs.

== Affected (this survey) ==
Artist's Talent (mtg-843) -> PARTIAL. Also blocks Torbran (mtg-902 B1).
