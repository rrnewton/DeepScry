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

== Definition of done ==
1. Every per-card issue reaches CARD STATUS: WORKING (or accepted PARTIAL w/ bug followup).
2. Each deck has end-to-end log with each non-vanilla ability verified by targeted puzzle.
3. Tournament 0% engine-failure (MET) AND each B1..B5 gap fixed or accepted.

== How agents pick work ==
Open umbrella -> backlog bead (B1 Honden /Times.N count-multiplier first: smallest fix, wrong life) -> targeted_compatibility SKILL. Fixes DEFERRED until in-flight engine refactor lands (SURVEY-ONLY pass; no engine edits). Coordinate via mb; never duplicate.

Driven by agent compat-2005-survey (slot05), 2026-06-10, overnight autonomous survey. SURVEY/CLASSIFY/BEADS ONLY.
