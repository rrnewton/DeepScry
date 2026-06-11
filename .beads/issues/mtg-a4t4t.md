---
title: 'TRACK: 2005 World Championship decks — full deck compatibility'
status: open
priority: 1
issue_type: task
depends_on:
  mtg-684: parent-child
created_at: 2026-06-11T04:20:42.989713563+00:00
updated_at: 2026-06-11T04:20:42.989713563+00:00
---

# Description

TRACK: full play-tested gameplay compatibility for all 2005 Magic World Championship decks (Yokohama, Japan; Standard / Kamigawa-Ravnica era). Sibling of mtg-709 (1994), mtg-881 (2025), mtg-901 (2020); rolls up under the championship-collections umbrella mtg-684.

User goal: each 2005 World Championship deck plays COMPLETE games with NO engine errors, and every card's abilities/keywords/effects classified WORKING (or PARTIAL/BROKEN then FIXED) per the targeted_compatibility + compatibility_tracking SKILLs.

== Scope ==
decks/championship/2005/ (4 Top-4 decks, incl sideboards):
- 01_mori_ghazi_glare      (1st, World Champion, Katsuhiro Mori JP — Selesnya Ghazi-Glare aggro-combo)
- 02_karsten_greater_gift  (2nd, Frank Karsten NL — BGW Greater Gifts / Gifts Ungiven ramp-reanimator; NOTE deck file reconstructed, ~56 cards vs 60 expected)
- 03_asahara_enduring_ideal (3rd-4th, Akira Asahara JP — Enduring Ideal enchantment combo; NOTE deck file APPROXIMATE reconstruction)
- 04_kaji_ghazi_glare      (3rd-4th, Tomohiro Kaji JP — Selesnya Ghazi-Glare, same archetype as Mori; identical 60-card main, empty sideboard)

2005-STANDARD pool (Kamigawa legends/Shrines/Spirits, Ravnica guildlands/shocklands, Umezawa's Jitte) — distinct from 1994/Vintage and 2020/2025 modern pools.

== Baseline survey (2026-06-10_#3175(c6dbd34f), agent compat-2005-survey slot05) ==
- 57 unique cards across all 4 decks (union, incl sideboards). ALL 57 resolve to a single cardsfolder file — ZERO missing cards. (Decks 01/04 LF, decks 02/03 CRLF — strip CR when re-extracting the union.)
- 'mtg tourney' all 4 decks x 200 games (seed 7): ALL completed, ZERO panics/crashes. Win-rates sane. debug/survey2005/tourney.log.
- 4 per-deck heuristic mirror games at verbosity 3 (debug/survey2005/v3_0[1-4].log): all play to a real win/loss, no panics.

KEY NUANCE (same as 2020/2025 surveys): zero panics != every ability works. Heuristic AI exercises only a subset of abilities; several gaps are SILENT at runtime (dropped at converter, debug-level only). Runtime WARN lines + static ApiType scan surfaced the real gaps.

== Runtime WARN census (200-game tourney) ==
- 15651x  Unimplemented effect 'RearrangeTopOfLibrary' no-op  -> Sensei's Divining Top look-at-top-3 (B2)
-  3930x  Unknown filter type: Shrine.YouCtrl/Times.2          -> Honden of Cleansing Fire gain-life (B1)
-   832x  ModalChoice reached execute_effect (3 modes)         -> Umezawa's Jitte Charm fallback (B3)
-    84x  Unimplemented effect 'SkipPhase' no-op               -> Yosei skip-untap death-trigger half (B4)
(NO panics, NO Unknown(*) sentinels, NO illegal-action drops.)

== ApiType / construct survey (static scan of all 57 scripts vs effect_converter.rs) ==
IMPL converter arms: Mana, Tap, TapAll, Untap, Pump, PumpAll, Token, Draw, Discard, GainLife, LoseLife, SetLife, DealDamage, DamageAll, Destroy, DestroyAll, Sacrifice, Regenerate, ChangeZone, ChangeZoneAll, Cleanup, Effect, PutCounter, Charm, Animate, ChooseSource.
GAP (catch-all -> Effect::Unimplemented no-op):
  * RearrangeTopOfLibrary (Sensei's Divining Top) -> B2
  * SkipPhase (Yosei death trigger) -> B4
  * NameCard (Pithing Needle ETB, Cranial Extraction) = ApiType::Unknown; NOT exercised in any survey game -> B5
COUNT-EXPR GAP: 'Count$Valid Shrine.YouCtrl/Times.2' — '/Times.N' multiplier suffix unknown -> B1.

== BROKEN / PARTIAL findings (see backlog bead B1..B5) ==
B1 [BROKEN] Honden of Cleansing Fire (4x deck03): gain-2-life-per-Shrine count drops the /Times.2 multiplier. Puzzle debug/survey2005/honden_gainlife.pzl (2 Shrines): 'gains 6 life' (WRONG, should be 4) + 8 unknown-filter warns. Incorrect life math.
B2 [PARTIAL] Sensei's Divining Top (4x decks 02/03): {1} look-at-top-3-reorder (RearrangeTopOfLibrary) unimplemented -> no-op. The {T} draw + put-Top-on-library (Draw + ChangeZone) WORKS (v3_02.log). Library-manipulation half silently dead. Highest WARN volume.
B3 [PARTIAL/VERIFY] Umezawa's Jitte (3x decks 01/04): Charm modal emits 'ModalChoice reached execute_effect' 832x. Charge-counter trigger parses. Need equipped-creature puzzle to confirm chosen mode applies.
B4 [PARTIAL] Yosei the Morning Star (decks 01/04 + SB 02): death-trigger skip-untap half (SkipPhase) unimplemented -> no-op (confirmed v3_02.log). Tap-up-to-5 sub-ability IMPL. Lock half dead, tap half works. Flying+body fine.
B5 [UNVERIFIED] Pithing Needle (3x decks 01/04) + Cranial Extraction (SB 02): both use NameCard (ApiType::Unknown). NEITHER deployed by heuristic in any survey game — NO evidence either way. CantBeActivated static + graveyard/hand/library exile UNTESTED. Need puzzles.

== UNTESTED but parse-clean (never deployed by heuristic in survey seeds) ==
Glare of Subdual (deck namesake convoke tap-down), Seedborn Muse, Hokori Dust Drinker, Ghostly Prison/Solitary Confinement/Ivory Mask/CoP:Red (Enduring Ideal pillowfort), Faith's Fetters, Vitu-Ghazi/Meloku/Selesnya Guildmage tokens, Greater Good sac-draw, Gifts Ungiven choose-2-of-4, Congregation at Dawn, Seed Spark, Goryo's Vengeance/Debtors' Knell/Ink-Eyes reanimation, Life from the Loam dredge, Nightmare Void. Most LIKELY work (IMPL) but lack game-log evidence.

== Evidence-backed WORKING (ability visibly fired in survey logs) ==
Birds of Paradise, Llanowar Elves, Wood Elves, Loxodon Hierarch, Selesnya Guildmage, Kodama of the North Tree, Arashi, Wrath of God, Kokusho (drain on death), Mortify, Sensei's Top (draw half), Farseek, Kodama's Reach, Form of the Dragon, Enduring Ideal, all lands produce mana. ~18 / 57.

== HEADLINE % VALIDATED (2026-06-10_#3175(c6dbd34f)) ==
- Evidence-backed WORKING (ability seen firing in a real game log): ~18 / 57 = ~32%.
- Likely-fine-unvalidated (parse clean, construct IMPL, ability never exercised): ~35 / 57.
- PARTIAL/BROKEN with filed gap: 4 (Honden B1, Sensei Top B2, Jitte B3, Yosei B4) + 2 unverified-NameCard (B5).
Validated-WORKING rate = ~32% evidence-backed. 0% engine-failure (no-crash) bar MET; per-ability-WORKING bar NOT met for ~68%.

== DEEP-VERIFICATION PASS (2026-06-10_#3180(d7cc1319), agent compat-2005-deepverify slot05) ==
Method: 24 fresh verbosity-3 games (4 pairings x 6 seeds, ~29k lines) to let the heuristic exercise more cards, PLUS ~14 targeted fixed-input puzzles (debug/deepverify2005/*.pzl) forcing the "likely-fine but untested" cards to fire. Goal: promote parse-clean cards to evidence-backed WORKING and surface any newly-confirmed broken cards.

NEWLY EVIDENCE-BACKED WORKING (log evidence captured this pass): 
- Naturalize (destroys target enchantment: 'Naturalize destroys Ghostly Prison'); 
- Carven Caryatid (ETB 'Player 1 draws Swamp', enters 2/5); 
- Greater Good (sac Kodama -> draw=power -> 'discards' x3); 
- Miren, the Moaning Well (sac Carven tou=5 -> 'gains 5 life' — correct toughness math); 
- Faith's Fetters (ETB 'gains 4 life' + enchants permanent); 
- Seed Spark (destroys target + ConditionManaSpent$ G makes exactly 2 Saproling tokens); 
- Congregation at Dawn (searches 3 creatures, puts on Library top); 
- Gifts Ungiven (searches library, moves chosen to graveyard, rest to hand, clears remembered); 
- Nightmare Void (opp 'discards Brushland' — discard half works; dredge half is B7); 
- Life from the Loam (returns a land card gy->hand — return half works; dredge half is B7); 
- Selesnya Guildmage CAST (body resolves; its activated token/pump abilities still UNVERIFIED — AI never activated).
=> +~10 cards promoted to evidence-backed WORKING.

NEWLY CONFIRMED BROKEN (new backlog items B6, B7):
- B6 [BROKEN] Graveyard-targeting ChangeZone self-targets / drops target. Reclaim, Recollect, Debtors' Knell, Goryo's Vengeance — ALL in deck 02 (karsten_greater_gift, the recursion deck). Goryo's Vengeance emits the 'Unknown (0)' SENTINEL ('grants Unknown (0) gains Haste') and reanimates ITSELF instead of the legendary creature; Reclaim moves itself gy->library; Recollect 'has no matching Card in graveyard to return'; Debtors' Knell upkeep trigger fires but reanimates nothing. Same class as 1994 B22 (Regrowth self-target). HIGH VALUE for deck 02.
- B7 [BROKEN] Dredge keyword has NO draw-replacement firing logic. Parsed (keyword_set.rs KeywordArgs::Dredge{amount}) but nothing in game/ replaces a draw with mill-N+return. Life from the Loam (Dredge 3) + Nightmare Void (Dredge 2), both deck 02 — recur never; they function only as one-shot spells. Puzzle debug/deepverify2005/dredge.pzl: draw step just 'draws Mountain', no dredge offered.

STILL BROKEN/PARTIAL (no regression, no fix — refactor not landed): B1 Honden /Times.2 (warns persist), B2 Sensei RearrangeTopOfLibrary (no-op persists), B3 Jitte ModalChoice->execute_effect (persists), B4 Yosei SkipPhase (no-op persists). B5 Pithing Needle/Cranial Extraction NameCard still un-deployed (not forced this pass).

UPDATED HEADLINE (2026-06-10_#3180(d7cc1319)):
- Evidence-backed WORKING: ~28 / 57 = ~49% (was ~18/57=~32%).
- BROKEN/PARTIAL with filed gap: B1-B4 (unchanged) + B6 (4 gy-recursion cards) + B7 (2 dredge cards); B5 unverified.
- Likely-fine-unvalidated remainder: ~21/57 (Glare of Subdual, Seedborn Muse, Hokori, Ghostly Prison, Solitary Confinement, Ivory Mask, CoP:Red, Meloku token, Selesnya Guildmage activated abilities, Vitu-Ghazi token, Ink-Eyes, Reclaim/Recollect target-half pending B6 fix). Most LIKELY work but not yet log-proven; Glare/Seedborn/Meloku activations the heuristic+fixed-input did not trigger this pass.
Validated-WORKING rate rose ~32% -> ~49% evidence-backed. 0% engine-failure bar still MET.

== Definition of done ==
1. Every per-card issue reaches CARD STATUS: WORKING (or accepted PARTIAL w/ bug followup).
2. Each deck has end-to-end log with each non-vanilla ability verified by targeted puzzle.
3. Tournament 0% engine-failure (MET) AND each B1..B7 gap fixed or accepted.

== How agents pick work ==
Open umbrella -> backlog bead (B1 Honden /Times.N count-multiplier first: smallest fix, wrong life; B6 gy-targeting + B7 dredge unblock the whole deck-02 recursion plan) -> targeted_compatibility SKILL. Fixes DEFERRED until in-flight engine refactor lands (SURVEY-ONLY pass; no engine edits). Coordinate via mb; never duplicate.

Driven by agent compat-2005-survey (slot05), 2026-06-10, overnight autonomous survey. SURVEY/CLASSIFY/BEADS ONLY.
DEEP-VERIFY UPDATE 2026-06-10_#3180(d7cc1319) by agent compat-2005-deepverify (slot05): +~10 cards promoted to evidence-backed WORKING (~32%->~49%); filed B6 (graveyard-targeting ChangeZone self-targets, deck 02, Unknown(0) sentinel on Goryo's Vengeance) + B7 (Dredge keyword has no draw-replacement logic). Puzzle artifacts in gitignored debug/deepverify2005/.
