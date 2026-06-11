---
title: 'Bug: ApiType::ChooseType unsupported -> basic-land-type land produces no mana (Multiversal Passage)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-11T04:05:11.812532486+00:00
updated_at: 2026-06-11T04:05:11.812532486+00:00
---

# Description

Root-cause bug found in the 2025 World Championship survey (mtg-881, branch claude/compat-2025-survey).

STAMP: 2026-06-10_#3175(c6dbd34f)

== Symptom ==
Multiversal Passage (a 4-of mana-base land in decks 01, 02, 03) is a DEAD land: across all 4 verbosity-3 mirror games it (a) never prompted the "choose a basic land type" ETB choice, (b) never offered the "pay 2 life or enter tapped" choice, and (c) never produced ANY mana. The AI plays it and then leaves it idle forever because it cannot tap it for mana.

== Card ==
cardsfolder/m/multiversal_passage.txt (Multiversal Passage, Land, no mana cost)
  R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplaceWith$ DBChooseBasic | ReplacementResult$ Updated | Description$ As this land enters, choose a basic land type. Then you may pay 2 life. If you don't, it enters tapped.
  SVar:DBChooseBasic:DB$ ChooseType | ETB$ True | Type$ Basic Land | SubAbility$ DBTap
  SVar:DBTap:DB$ Tap | ETB$ True | Defined$ Self | UnlessCost$ PayLife<2> | UnlessPayer$ You
  S:Mode$ Continuous | Affected$ Card.Self | AddType$ ChosenType | RemoveLandTypes$ True | Description$ This land is the chosen type.

== Root cause ==
ApiType::ChooseType has NO converter arm in mtg-engine/src/loader/effect_converter.rs (grep: zero hits). The whole ETB chain is silently dropped: no basic-land-type chosen -> the S:Mode$ Continuous "AddType$ ChosenType | RemoveLandTypes$ True" static has no ChosenType to apply -> the land has NO intrinsic mana ability and NO basic-land subtype -> it produces no mana. The conditional-tapped Tap sub-ability (UnlessCost$ PayLife<2>) also never fires (the card entered untapped/silently). This is a SILENT PARSER/CONVERTER DROP, not a runtime crash, so it never surfaced in tourney warnings.

== Empirical evidence ==
debug/compat2025/v3_01_manfield.log (gitignored, branch claude/compat-2025-survey):
  line 490: AI-Heuristic1 plays Multiversal Passage (46)
  (no "chooses <type>" line, no "pay 2 life" line, enters untapped)
  Grep across all 4 v3_*.log for "chosen type"/"choose a basic"/"Multiversal ... produces|tap ... for {" => ZERO hits. It never makes mana.

== Reproducer ==
```sh
./target/release/mtg tui decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck --p1 heuristic --p2 heuristic --seed 7 --verbosity 3 2>&1 | grep -iE "Multiversal|chooses.*type|choose a basic"
```
Expected stdout (after fix): a "choose a basic land type" prompt + the land tapping for the chosen color later. Current: only "plays Multiversal Passage", never any mana.

== Fix shape ==
Add converter + engine support for ApiType::ChooseType (ETB$ True, Type$ Basic Land) plus the AddType$ ChosenType / RemoveLandTypes$ True static, so the player/AI chooses a basic land type on ETB and the land gains that subtype's intrinsic mana ability. The choice must be deterministic + rewind-safe + identical server/client (no hidden info). Generalizes to other "choose a basic land type / creature type" cards. Game-logic change -> requires MTG rules review. DO NOT START until the mtg-245 refactor lands.

== Affected cards ==
Multiversal Passage (this survey, decks 01/02/03). Any card using DB$ ChooseType + AddType$ ChosenType (basic-land-type-choosing lands, creature-type choosers).

Rolls up under mtg-881 / 2025 backlog. CARD STATUS for Multiversal Passage (mtg-847): BROKEN.
