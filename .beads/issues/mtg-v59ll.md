---
title: 2025 Championship — broken-card root-cause backlog (B1-B3)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-881: related
  mtg-crapg: related
  mtg-j3326: related
  mtg-kzsk3: related
created_at: 2026-06-11T04:08:01.235140011+00:00
updated_at: 2026-06-11T04:08:01.235140011+00:00
---

# Description

2025 World Championship 31 (Bellevue WA, Dec 2025) compat survey — broken/partial-card root-cause backlog. Compiled by agent compat-2025-survey (slot05) from: a 200-game tourney (seed 7) over all 4 decks, per-deck verbosity-3 mirror games, and a static ApiType/keyword/Count-form scan of every unique card (union of main+sideboard). Rolls up under the 2025 TRACK bead mtg-881 (umbrella mtg-684).

STAMP: 2026-06-10_#3175(c6dbd34f)

Reproducer logs live in (gitignored) debug/compat2025/ on branch claude/compat-2025-survey: tourney.log (200 games) + v3_<deck>.log (per-deck verbosity-3 mirror).

ALL FIXES ARE BLOCKED until the mtg-245 execute_effect dispatch-table refactor lands — it owns the effect/Count resolution paths these fixes touch, so fixing now would collide. This is a SURVEY: classify + file only, no engine edits.

== Survey headline ==
51 unique non-basic cards across the 4 decks. 200-game tourney completed with NO crashes, NO panics, NO local desync. The only runtime warning is the Torch DealDamageXPaid bug (B1). Two BROKEN + one PARTIAL found below; everything else parses with supported constructs and is WORKING or likely-WORKING.

== Prioritized backlog ==

B1 [BROKEN, HIGH VALUE — DO FIRST] Torch the Tower
  Card: cardsfolder/t/torch_the_tower.txt (1x main deck 02, 4x main deck 04, sideboard 01)
  Bug issue: mtg-j3326. Per-card: mtg-863.
  Root cause: A:SP$ DealDamage | NumDmg$ X with SVar:X:Count$Bargain.3.2 — the conditional-count form ("3 if bargained else 2") is not resolved before execute_effect, so it reaches the DealDamageXPaid path unresolved and deals 0 damage (logs "treating as 0 damage"). DIFFERENT Count form than the already-fixed Combustion Technique Count$ValidGraveyard (mtg-820/DealDamageDynamic).
  Empirical: 200-game tourney emits this WARN 135x, ONLY from the two decks containing Torch (02: 9 in 12 games, 04: 21 in 12 games); the two Combustion-only decks (01,03) emit ZERO. In debug/compat2025/v3_02_shibata.log (~L505-513) Torch torched a 1/2 Gran-Gran which survived and kept attacking.
  Repro: ./target/release/mtg tourney decks/championship/2025/02_shibata_izzet_lessons.dck --games 12 --seed 7 2>&1 | grep -c DealDamageXPaid  (expect >0 now, 0 after fix)
  Fix shape: resolve Count$Bargain.N.M to a scalar on the DealDamageDynamic path. Generalizes to any NumDmg$ X with a two-arm conditional Count.

B2 [BROKEN, HIGH VALUE] Multiversal Passage
  Card: cardsfolder/m/multiversal_passage.txt (4x main decks 01, 02, 03)
  Bug issue: mtg-kzsk3. Per-card: mtg-847.
  Root cause: ApiType::ChooseType has NO converter arm (effect_converter.rs: zero hits). The ETB chain (R:Event$ Moved -> DB$ ChooseType "choose a basic land type" -> conditional Tap unless pay 2 life) is silently dropped, and the S:Mode$ Continuous "AddType$ ChosenType | RemoveLandTypes$ True" static has nothing to apply. Result: the land has no basic-land subtype and no mana ability -> produces NO mana. A 4-of mana-base land that is dead. Silent drop (no runtime warning).
  Empirical: across all 4 verbosity-3 games, Multiversal Passage is played but never prompts a type choice and never taps for mana (debug/compat2025/v3_01_manfield.log L490; grep "choose a basic"/"chosen type" over all v3_*.log = zero).
  Repro: ./target/release/mtg tui decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck --p1 heuristic --p2 heuristic --seed 7 --verbosity 3 2>&1 | grep -iE "Multiversal|choose a basic"
  Fix shape: implement ApiType::ChooseType (ETB basic-land-type choice) + AddType$ ChosenType so the land gains the chosen subtype's mana ability. Generalizes to all basic-land-type-choosing / creature-type-choosing cards.

B3 [PARTIAL] Artist's Talent (Level 3 damage bonus)
  Card: cardsfolder/a/artists_talent.txt (decks 01, 02)
  Bug issue: mtg-crapg (shared damage-INCREASE replacement gap; also blocks Torbran, mtg-902 B1). Per-card: mtg-843.
  Root cause: NO damage-INCREASE replacement layer. The Level 3 ability (DB$ ReplaceEffect | VarName$ DamageAmount | VarValue$ X with X = ReplaceCount$DamageAmount/Plus.2) has no support — ApiType::ReplaceEffect does not exist in the parser; no converter arm; only damage-PREVENTION replacements exist (core/prevention.rs). L1 (cast-noncreature discard->draw) and L2 (S:Mode$ ReduceCost {1}) WORK; L3 "your noncombat damage to opponents +2" silently does nothing. Strictly weaker than printed at L3.
  Fix shape: add a damage-modification replacement category at the single damage-application chokepoint (combat + ability), filter from ValidSource/ValidTarget/IsCombat. Generalizes to Torbran, Fiery Emancipation, Gratuitous Violence, City on Fire.

== Stale-tracker corrections (NOW WORKING / improved, not broken) ==
- Stormchaser's Talent: tracker note "BROKEN (PumpCreature fizzled L3 Otter pump)" is STALE. Fixed (mtg-879, closed WORKING). Survey confirms: creates Otter Token + levels up cleanly, no fizzle in any of the 4 games (debug/compat2025/v3_01_manfield.log L248-263).
- Valley Floodcaller (mtg-846): tracker note "BROKEN (partial) PumpAll fizzle" is STALE. The fizzle warning is GONE in the 200-game tourney (zero fizzle/unresolved-target warnings). Enters/blocks/dies cleanly. Still PARTIAL only because the PumpAll trigger + CastWithFlash static were not EXERCISED by the heuristic AI (need a targeted puzzle), not because they are broken.
- Combustion Technique (mtg-831): confirmed genuinely FIXED/WORKING — the decks containing it but not Torch (01,03) emit zero DealDamageXPaid warnings.

== Known-issue cross-reference ==
- Thundertrap Trainer ETB Dig network desync: mtg-0o9s9 (and the turn-13 REWIND/REPLAY divergence mtg-908). Not reproduced in LOCAL tourney (desync is network-mode only). Per-card mtg-842.

== Cards confirmed playable end-to-end (no crash; supported constructs; per-card game-log to be captured with puzzles) ==
Counterspells (Annul, Negate, Spell Pierce, Essence Scatter, Disdainful Stroke, It'll Quench Ya!, Spider-Sense via SP$ Counter incl. TargetType Triggered), cantrips/draw (Opt, Stock Up, Accumulate Wisdom, Boomerang Basics, Abandon Attachments, Quantum Riddler), Charm modals (Iroh's Demonstration, Fire Magic, Bushwhack, Pawpatch Formation — all use supported Charm/DamageAll/DealDamage/Fight/Token/Destroy), lands (Spirebluff Canal, Riverpyre Verge, Willowrush Verge, Botanical Sanctum, Breeding Pool, Stomping Ground, Starting Town, Agna Qel'a), and the creatures/enchantments whose ApiTypes are all in the converter (Gran-Gran, Eddymurk Crab, Enduring Vitality, Ghost Vacuum, Soul-Guide Lantern, Torpor Orb, Roaring Furnace, The Legend of Kuruk, Broadside Barrage, Abrade, Frostcliff Siege). These remain UNTESTED in per-card tracking (no captured game-log) — promote to WORKING with targeted puzzles in the post-refactor fix pass.
