---
title: 2005 Championship — broken-card root-cause backlog (B1-B7)
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

UPDATE 2026-06-10_#3180(d7cc1319), agent compat-2005-deepverify (slot05): added B6 (graveyard-targeting ChangeZone self-targets — 4 deck-02 recursion cards, Goryo's Vengeance emits Unknown(0) sentinel) + B7 (Dredge keyword has no draw-replacement logic — Life from the Loam + Nightmare Void). Both found by forcing the deck-02 recursion cards through targeted fixed-input puzzles. Deep-verify also promoted ~10 cards to evidence-backed WORKING (tracker mtg-a4t4t headline ~32%->~49%).

Prioritized fix order: B6 (graveyard-target resolution — unblocks the whole deck-02 recursion engine, 4 cards, clear Unknown(0) sentinel) -> B1 (smallest, wrong life math) -> B7 (Dredge replacement, 2 cards) -> B4 (skip-untap lock) -> B2 (Top library manip) -> B3 (Jitte modal verify) -> B5 (NameCard, unverified).

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

== B6 [BROKEN, HIGH VALUE for deck 02] Graveyard-targeting ChangeZone self-targets / drops its target ==
Cards (ALL in deck 02 karsten_greater_gift, the recursion deck):
  - cardsfolder/r/reclaim.txt        — SP$ ChangeZone | Origin$ Graveyard | Destination$ Library | ValidTgts$ Card.YouCtrl
  - cardsfolder/r/recollect.txt      — SP$ ChangeZone | Origin$ Graveyard | Destination$ Hand    | ValidTgts$ Card.YouCtrl
  - cardsfolder/d/debtors_knell.txt  — T:upkeep -> DB$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | GainControl$ True | ValidTgts$ Creature
  - cardsfolder/g/goryos_vengeance.txt — SP$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | ValidTgts$ Creature.Legendary+YouCtrl | RememberChanged$ True -> DBPump Animate Haste on Remembered
Symptom: a ChangeZone whose Origin is Graveyard and which TARGETS a graveyard card does NOT correctly solicit/bind the chosen card. Observed (puzzles debug/deepverify2005/{reclaim,recollect,debtors,goryo}.pzl, fixed-input forced cast, seed 7, v3):
  - Goryo's Vengeance: 'Goryo's Vengeance (3) moves from Graveyard to Battlefield' (the INSTANT reanimates ITSELF, not the target Kokusho) + 'grants Unknown (0) gains Haste' — the 'Unknown (0)' SENTINEL proves the Remembered target is empty/unresolved.
  - Reclaim: 'Reclaim (3) moves from Graveyard to Library' (moves itself, not the target Kokusho). Even an explicit 'target Kokusho' fixed-input is ignored; no target choice is offered.
  - Recollect: 'Player 1 has no matching Card in graveyard to return' (target predicate Card.YouCtrl matches nothing — even though Reclaim with the IDENTICAL predicate self-matched).
  - Debtors' Knell: upkeep trigger fires ('Debtors' Knell trigger effect') but NO creature is reanimated (Kokusho stays in gy; battlefield unchanged).
Root cause class: target-resolution for ChangeZone with Origin$ Graveyard + ValidTgts. The targeting layer appears to (a) auto-bind the resolving spell's own card from the graveyard, or (b) fail the ValidTgts match entirely, rather than enumerating the legal graveyard cards and binding the chosen one to Remembered/Targeted. SAME class as 1994 mtg-713 B22 (Regrowth self-targeted itself from the graveyard — flagged there as "RE-VERIFY: fixed-input auto-target artifact?"; this pass CONFIRMS it is a genuine engine gap, reproduced with explicit targets and across 4 distinct cards). Generalizes to every graveyard-recursion / reanimation card (Goryo, Debtors' Knell, Reclaim, Recollect, Regrowth, Ink-Eyes, and similar). HIGH VALUE: it cripples deck 02's entire graveyard-recursion engine.
Repro:

```sh
cat > /tmp/goryo.pzl <<'P'
[metadata]
Name=Goryos Vengeance reanimate legendary
Goal=Win
Turns=2
[state]
turn=3
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Goryo's Vengeance
p0battlefield=Swamp;Swamp
p0graveyard=Kokusho, the Evening Star
p0library=Swamp;Swamp
p1life=20
p1battlefield=Plains
p1library=Plains;Plains
P
./target/release/mtg tui --start-state /tmp/goryo.pzl --p1 fixed --p1-fixed-inputs "cast Goryo's Vengeance;*;*" --p2 zero --seed 7 -v 3 2>&1 | grep -iE "Goryo|Kokusho|Unknown|Battlefield"
```
Expected (BUG today): 'Goryo's Vengeance (3) moves from Graveyard to Battlefield' + 'grants Unknown (0) gains Haste' (self-reanimates, sentinel). After fix: 'Kokusho ... moves from Graveyard to Battlefield' + Kokusho gains Haste.

== B7 [BROKEN] Dredge keyword has no draw-replacement firing logic ==
Cards: cardsfolder/l/life_from_the_loam.txt (Dredge 3, deck 02), cardsfolder/n/nightmare_void.txt (Dredge 2, deck 02).
Symptom: Dredge IS parsed (mtg-engine/src/core/keyword_set.rs: KeywordArgs::Dredge { amount }; loader/card.rs:1125 'Dredge' arm) but there is NO logic anywhere in mtg-engine/src/game/ that REPLACES a draw with "mill N, return this card from graveyard to hand" (grep for a draw-replacement-by-dredge site = zero hits). So a card with Dredge in the graveyard never offers the dredge option; the player just draws normally.
EMPIRICAL: puzzle debug/deepverify2005/dredge.pzl (Life from the Loam in graveyard, draw step) -> 'Player 1 draws Mountain' (normal draw), NO dredge prompt, Loam stays in graveyard.
Impact: Life from the Loam + Nightmare Void function ONLY as their one-shot spell (the spell halves WORK — Loam returns a land, Nightmare Void discards); their recursion engine (the entire reason they're in a recursion deck) is silently dead. Both deck 02.
Root cause class: missing Dredge replacement effect — a "if you would draw, you may instead mill N and return this from gy to hand" replacement applied at the draw event for any card with Dredge in the graveyard. Generalizes to all Dredge cards.
Repro:

```sh
cat > /tmp/dredge.pzl <<'P'
[metadata]
Name=Dredge Loam
Goal=Win
Turns=2
[state]
turn=3
activeplayer=p0
activephase=UPKEEP
p0life=20
p0battlefield=Forest;Forest
p0graveyard=Life from the Loam
p0library=Forest;Island;Swamp;Plains;Mountain
p1life=20
p1battlefield=Plains
p1library=Plains;Plains
P
./target/release/mtg tui --start-state /tmp/dredge.pzl --p1 fixed --p1-fixed-inputs "*;*;*;*" --p2 zero --seed 7 -v 3 2>&1 | grep -iE "dredge|mill|Life from the Loam|draws"
```
Expected (BUG today): 'Player 1 draws Mountain' only — no dredge offered. After fix: a dredge prompt -> mill 3 -> 'returns Life from the Loam from graveyard to hand'.

== Cross-links ==
Tracker mtg-a4t4t. Umbrella mtg-684. Sibling backlogs: mtg-713 (1994), mtg-902 (2020), mtg-v59ll (2025). Cross-year rollup epic mtg-b4aat. B6 is the SAME class as 1994 B22 (Regrowth) — now CONFIRMED a genuine engine gap (fold into the rollup epic's long-tail / a graveyard-target-resolution family). Survey artifacts (gitignored): debug/survey2005/{tourney.log, v3_0[1-4].log, honden_gainlife.pzl} + debug/deepverify2005/{v3_*.log (24 games), *.pzl (~14 puzzles), dredge.pzl, goryo.pzl}.
