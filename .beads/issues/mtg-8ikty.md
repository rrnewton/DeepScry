---
title: 'TRACK: 2015 World Championship decks — full deck compatibility'
status: open
priority: 1
issue_type: task
depends_on:
  mtg-684: parent-child
created_at: 2026-06-11T04:04:18.693069651+00:00
updated_at: 2026-06-11T04:04:40.910398217+00:00
---

# Description

TRACK: full play-tested gameplay compatibility for all 2015 Magic World Championship decks (Seattle WA, Aug 27-30 2015; Standard / Khans of Tarkir + Magic Origins era). Sibling of mtg-709 (1994), mtg-881 (2025), mtg-901 (2020); rolls up under the championship-collections umbrella mtg-684.

User goal: each 2015 World Championship deck plays COMPLETE games with NO engine errors, and every card's abilities/keywords/effects classified WORKING (or PARTIAL/BROKEN then FIXED) per the targeted_compatibility + compatibility_tracking SKILLs.

== Scope ==
decks/championship/2015/ (Top-4 decks, incl sideboards):
- 01_manfield_abzan_control   (1st, Seth Manfield, Abzan BW/G Midrange Control — World Champion)
- 02_turtenwald_abzan_control (2nd, Owen Turtenwald, Abzan Control w/ Dragonlord Dromoka)
- 03_rietzl_abzan_aggro       (3rd, Paul Rietzl, Abzan Aggro w/ Hangarback Walker)
- 04_black_mono_white         (4th, Samuel Black, Mono-White Devotion)

This is a mid-2010s STANDARD pool: Khans-of-Tarkir wedge gold cards (Abzan Charm, Siege Rhino), Theros-block devotion/bestow, Magic Origins flip-walkers (Kytheon, Nissa Vastwood Seer), morph/megamorph (Den Protector), manifest (Mastery of the Unseen), delve (Murderous Cut, Tasigur), and a full planeswalker suite (Elspeth, Ugin, Sorin, Ajani).

== Baseline survey (2026-06-10_#3175(c6dbd34f), agent compat-2015-survey slot07) ==
- 59 unique cards across all 4 decks (union, incl sideboards). ALL 59 resolve to a single cardsfolder file (Kytheon and Nissa are flip-walker combined DFC files: kytheon_hero_of_akros_gideon_battle_forged.txt, nissa_vastwood_seer_nissa_sage_animist.txt). Every deck has a 60-card main.
- 'mtg tourney --mirror-only --games 200 --seed 7' for ALL 4 decks: every game completed, ZERO crashes / panics. Logs in (gitignored) debug/survey2015/tourney_<deck>.log on branch claude/compat-2015-survey.
- Per-deck verbosity-3 mirror games captured (debug/survey2015/game_<deck>.log): clean; only non-error 'warn' was a legitimate 'Illegal block dropped: Knight of the White Orchid can't block Bird Token (evasion)' (Wingmate Roc fliers).

KEY NUANCE (same as 2020 sweep): ZERO crashes does NOT mean every ability works. The heuristic AI exercises only a subset of abilities, and several effects are silently dropped at the converter/loader with only a WARN/debug log (never a hard error). Static script + runtime-warning scan found the gaps below. The Unimplemented/ "not yet supported" warnings only fire when such an effect actually RESOLVES during the 200 games.

== ApiType / construct survey (static scan of all 59 scripts) ==
ApiTypes used and converter status (effect_converter.rs / ability_parser.rs):
- IMPL: ChangeZone, Mana, GainLife, Token, PutCounter, Pump, LoseLife, Cleanup, Scry, Effect, Destroy, Tap, Sacrifice, PumpAll, Draw, DealDamage, Charm, Untap, Discard, Dig(->DigMultiple), DestroyAll, ChangeZoneAll, Animate, SacrificeAll, Mill, Fight, DelayedTrigger, ChooseColor, ChooseCard, Planeswalk (loyalty).
- GAP (NOT in the ApiType enum at all -> parse to Unknown -> Effect::Unimplemented no-op):
    * RepeatEach  (Tragic Arrogance)  -- shared gap, already tracked by mtg-651 (Sylvan Library)
    * Manifest    (Mastery of the Unseen)
    * GenericChoice (Palace Siege mode-select)
Keywords used: Flying, Trample, Vigilance, Lifelink, First Strike, Protection (from), Delve (Murderous Cut/Tasigur), Megamorph (Den Protector), Bestow (Herald of Torment), Strive (Silence the Believers), etbCounter, ETBReplacement. All keywords are recognized in core/keyword_set.rs + loader/card.rs (NOT silently dropped at parse); runtime honoring of the rarer ones (megamorph turn-face-up, bestow as-aura, strive per-target cost) was NOT individually exercised by the heuristic mirror games and is marked unverified per-card.

== BROKEN findings (4 cards; see backlog bead for B1..B4 detail) ==
B1 [BROKEN, runtime-confirmed] Hangarback Walker (4x main 03, 3x main 04): K:etbCounter:P1P1:X -> "apply_etb_counters: non-numeric amount 'X' on Hangarback Walker not yet supported" (game/actions/mod.rs:1363). Enters as a 0/0 with NO counters, dies immediately to SBA, never makes Thopters. Fired 411x (deck 03) + 284x (deck 04) across the 200-game mirrors. This is the namesake of Rietzl's aggro deck.
B2 [BROKEN, runtime-confirmed] Tragic Arrogance (1x main 01 + sideboards): entire effect is 'A:SP$ RepeatEach' -> Unimplemented no-op (33 RepeatEach no-op warnings in deck-01 mirror). The World Champion's board-wipe does literally nothing. Shares the RepeatEach gap with mtg-651.
B3 [BROKEN, static] Mastery of the Unseen (4x main 04): the 'AB$ Manifest' activated ability (the card's whole engine) is unimplemented. The TurnFaceUp lifegain trigger can never fire because you can never manifest. Static enchantment parses but is functionally dead.
B4 [BROKEN, static] Palace Siege (sideboard 02): ETB 'GenericChoice | Choices$ Khans,Dragons' mode-select is unimplemented; AND loader/card.rs:4283 maps 'Card.Self+ChosenMode*' to Self_ unconditionally, so BOTH the Khans and Dragons conditional S: triggers attach regardless of choice (card is incorrectly STRONGER than printed if it works at all). Sideboard 1-of.

== WORKING (55 of 59 = 93.2%) ==
All other 55 unique cards use only IMPLEMENTED ApiTypes/keywords and survived the 200-game mirrors + verbosity-3 games with no crashes, no Unimplemented-effect warnings, no sentinels. Includes the full planeswalker suite (Elspeth Sun's Champion, Ugin the Spirit Dragon, Sorin Solemn Visitor, Ajani Mentor of Heroes, Kytheon/Nissa flip-walkers), Siege Rhino, Abzan Charm, the temples/painlands/Sandsteppe Citadel, Thoughtseize/Duress, Languish, Hero's Downfall, etc. NOTE: "WORKING" here = no runtime error + all constructs IMPL + played end-to-end; the rarer keywords (megamorph/bestow/strive/delve) lack per-card targeted-puzzle game-log evidence and should be promoted to verified-WORKING with puzzles in a later pass.

HEADLINE: 55/59 unique cards (93.2%) validated WORKING; 4 BROKEN (Hangarback Walker, Tragic Arrogance, Mastery of the Unseen, Palace Siege).

== Definition of done ==
1. Every per-card issue for these decks reaches CARD STATUS: WORKING (or accepted PARTIAL w/ bug followup).
2. The 4 BROKEN cards fixed: B1 (X-valued etbCounter), B2 (RepeatEach, via mtg-651), B3 (Manifest), B4 (GenericChoice mode-select).
3. Each deck has a captured end-to-end log with no unimplemented/sentinel/silent-drop errors AND each non-vanilla ability verified by targeted puzzle.
4. The 4-deck championship mirror reaches 0% engine-failure rate (crash rate ALREADY 0%; the 4 BROKEN cards emit WARN no-ops, not crashes).

== How agents pick work ==
Open this umbrella -> pick the highest-value BROKEN card from the backlog bead. B1 Hangarback Walker (X-valued etbCounter) is the highest value: it is 4x/3x main-deck across two of the four decks and is a clean, well-scoped engine gap. COORDINATE with the in-flight mtg-245 effects/actions refactor and concurrent 1994/2020 compat agents editing effect_converter.rs / actions/mod.rs / damage paths via mb claims + rebase. Never duplicate.

Survey is BEADS/CLASSIFY ONLY (no engine edits) to avoid colliding with the refactor; fixes deferred until the refactor lands.

Driven by agent compat-2015-survey (slot07), 2026-06-10 survey pass.
