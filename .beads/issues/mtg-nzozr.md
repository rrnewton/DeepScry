---
title: 2005 Championship — broken-card root-cause backlog (B1-B5)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-a4t4t: parent-child
created_at: 2026-06-11T04:21:29.707048424+00:00
updated_at: 2026-06-11T04:21:29.707048424+00:00
---

# Description

2005 World Championship compat survey — shared root-cause backlog. Compiled by agent compat-2005-survey (slot05) from a 200-game tourney (seed 7) + 4 verbosity-3 mirror games + static ApiType scan of all 57 union cards vs effect_converter.rs. Tracker: mtg-a4t4t (umbrella mtg-684). STAMP 2026-06-10_#3175(c6dbd34f).

SURVEY-ONLY pass — NO engine/web edits made. All fixes below are DEFERRED until the in-flight engine refactor lands. Each item lists file pointers + a runnable repro.

Prioritized fix order: B1 (smallest, wrong life math) -> B4 (skip-untap lock) -> B2 (Top library manip) -> B3 (Jitte modal verify) -> B5 (NameCard, unverified).

== B1 [BROKEN, HIGH VALUE] Honden of Cleansing Fire — '/Times.N' count multiplier dropped ==
Card: cardsfolder/h/honden_of_cleansing_fire.txt (4x in deck03 03_asahara_enduring_ideal).
Script: SVar:X:Count$Valid Shrine.YouCtrl/Times.2 ; TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ X.
Symptom: the '/Times.2' multiplier suffix on a Count$Valid expression is an unrecognized filter token. Runtime: '[WARN count] Unknown filter type in count expression: Shrine.YouCtrl/Times.2' (3930x across tourney).
EMPIRICAL: puzzle debug/survey2005/honden_gainlife.pzl (P0 controls Honden of Cleansing Fire + Honden of Seeing Winds = 2 Shrines) -> upkeep logged 'Player 1 gains 6 life' (WRONG; correct = 2 life x 2 Shrines = 4). The multiplier handling is broken, not merely dropped — output is non-zero but incorrect.
Root cause class: count-expression parser gap (the '/Times.N' post-filter multiplier). Search the Count$ filter parser (grep 'Times' / 'count expression' under mtg-engine/src). Generalizes to every Shrine cycle + any '/Times.N' card.
Repro:

```sh
cat > /tmp/honden.pzl <<'P'
[metadata]
Name=Honden Gain Life Test
Goal=Survive
Turns=2
[state]
turn=3
activeplayer=p0
activephase=UPKEEP
p0life=20
p0battlefield=Plains;Plains;Plains;Plains;Honden of Cleansing Fire;Honden of Seeing Winds
p0library=Plains;Plains;Plains
p1life=20
p1battlefield=Plains
p1library=Plains;Plains
P
./target/release/mtg tui --start-state /tmp/honden.pzl --p1 heuristic --p2 heuristic --seed 7 -v 3 2>&1 | grep -iE 'gains [0-9]+ life|Unknown filter'
```
Expected (BUG today): 'gains 6 life' + 'Unknown filter type ... Shrine.YouCtrl/Times.2'. After fix: 'gains 4 life', no warn.


== B2 [PARTIAL] Sensei's Divining Top — RearrangeTopOfLibrary unimplemented ==
Card: cardsfolder/s/senseis_divining_top.txt (4x: decks 02, 03).
Script: A:AB$ RearrangeTopOfLibrary | Cost$ 1 | NumCards$ 3  (look at top 3, reorder) ; A:AB$ Draw | Cost$ T ... SubAbility$ DBChangeZone (draw + put Top on library).
Symptom: '[WARN actions] Unimplemented effect RearrangeTopOfLibrary resolved as no-op' (15651x — AI spams the {1} ability). Falls through effect_converter.rs catch-all (line ~1849, '_ => Effect::Unimplemented'). RearrangeTopOfLibrary is a recognized API-name string but has NO converter arm and NO engine effect.
WORKING half: the {T} draw-then-put-on-library ability resolved cleanly in debug/survey2005/v3_02.log (Top cast + draw-loop functions). So the artifact is castable and one of two abilities works.
Root cause class: missing converter arm + missing engine effect for library-reorder (look at top N, controller reorders). Affects Sensei's Top, Soothsaying, Index, Brainstorm-style reorders.
Repro: deck02/03 tourney shows the warn; a clean puzzle would place Top on battlefield + activate {1}.

== B3 [PARTIAL / VERIFY] Umezawa's Jitte — Charm modal reaches execute_effect fallback ==
Card: cardsfolder/u/umezawas_jitte.txt (3x: decks 01, 04).
Script: A:AB$ Charm | Cost$ SubCounter<1/CHARGE> | Choices$ JittePump,JitteCurse,JitteLife.
Symptom: '[WARN actions] ModalChoice effect reached execute_effect - should have been resolved during casting. 3 modes available.' (832x). The Charm modal is expected to be resolved at cast/activation time; reaching execute_effect means the mode selection took a fallback path.
Charge-counter trigger (combat damage -> +2 CHARGE) parse is fine. Unknown whether the chosen mode (+2/+2 | -1/-1 | gain 2) actually APPLIES or is silently dropped on the fallback — NOT confirmed because Jitte was never equipped+activated in survey seeds.
Root cause class: ModalChoice resolution-timing for activated-ability Charm with a SubCounter cost. Needs a targeted equipped-creature puzzle (creature + Jitte attached + >=1 CHARGE counter) to assert the pump/curse/lifegain actually lands. File a per-card issue if mode is dropped.

== B4 [PARTIAL] Yosei, the Morning Star — SkipPhase (skip-untap) unimplemented ==
Card: cardsfolder/y/yosei_the_morning_star.txt (2x decks 01/04, 4x SB deck02).
Script: T:Mode$ ChangesZone ... dies ... Execute$ TrigSkipPhase ; SVar:TrigSkipPhase:DB$ SkipPhase | ValidTgts$ Player | Step$ Untap | IsCurse$ True | SubAbility$ TrigTap ; SVar:TrigTap:DB$ Tap | TargetMax$ 5 ...
Symptom: '[WARN actions] Unimplemented effect SkipPhase resolved as no-op' (84x). CONFIRMED in debug/survey2005/v3_02.log: 'Trigger: Yosei ... skips their next untap step. Tap up to five ...' then 'SkipPhase resolved as no-op' then 'Yosei goes to graveyard'. The skip-untap LOCK half is silently dead.
WORKING: Flying keyword, 5/5 body, and the chained TrigTap (DB$ Tap) sub-ability are IMPL. So the tap-up-to-5 half works; only the untap-skip is missing.
Root cause class: missing SkipPhase/SkipStep effect (skip a named step for target player next turn). Affects Yosei + Kokusho-cycle siblings + any 'skips next untap/draw step' card. Sibling to 1994 Stasis skip-untap work (mtg-711) but that was a replacement on the controller; this is a targeted next-turn step-skip curse.
Repro: deck01/02 tourney OR the v3_02.log Yosei-dies snippet; a Wrath-of-God + Yosei puzzle reproduces deterministically.

== B5 [UNVERIFIED] Pithing Needle + Cranial Extraction — NameCard (ApiType::Unknown) ==
Cards: cardsfolder/p/pithing_needle.txt (3x decks 01/04), cardsfolder/c/cranial_extraction.txt (SB deck02).
Scripts: Pithing Needle ETBReplacement -> DB$ NameCard | AILogic$ PithingNeedle, plus S:Mode$ CantBeActivated | ValidCard$ Card.NamedCard. Cranial Extraction: A:SP$ NameCard ... -> ChangeZoneAll graveyard/hand/library exile of Card.NamedCard.
Symptom: NameCard is NOT in the ApiType enum (ability_parser.rs) — parses as ApiType::Unknown('NameCard') -> converter catch-all -> Effect::Unimplemented. BUT neither card was deployed by the heuristic AI in any survey game, so there is NO game-log evidence of impact (no NameCard warn appeared because the effect never resolved).
Risk: if NameCard no-ops, Pithing Needle never picks a name (its CantBeActivated static keys off Card.NamedCard, so the whole lock is inert) and Cranial Extraction exiles nothing. Both are likely BROKEN but UNCONFIRMED.
Root cause class: missing ApiType::NameCard + the 'choose a card name' choice infra + Card.NamedCard predicate wiring. Needs targeted puzzles (force Pithing Needle ETB + try to activate a named source; force Cranial Extraction cast at an opponent with a known graveyard/hand/library). File per-card issues once confirmed.

== Cross-links ==
Tracker mtg-a4t4t. Umbrella mtg-684. Sibling backlogs: mtg-713 (1994), mtg-902 (2020), mtg-v59ll (2025). Survey artifacts (gitignored): debug/survey2005/{tourney.log, v3_0[1-4].log, honden_gainlife.pzl, unique_cards.txt, resolved.tsv}.
