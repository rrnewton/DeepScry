---
title: 2010 Championship — broken-card root-cause backlog (B1-B6)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-38g7u: parent-child
created_at: 2026-06-11T05:44:10.519162064+00:00
updated_at: 2026-06-11T05:44:10.519162064+00:00
---

# Description

2010 World Championship compat survey — shared root-cause backlog. Compiled by agent compat-2010-survey (slot05) from a 200-game tourney (seed 7) + 15 verbosity-3 games (3 pairings x 5 seeds) + static ApiType scan of all 47 union cards vs effect_converter.rs + ability_parser.rs. Tracker: mtg-38g7u (umbrella mtg-684). STAMP 2026-06-10_#3177(9e100acf).

SURVEY-ONLY pass — NO engine/web edits made. All fixes below are DEFERRED until the in-flight engine refactor (owns effects/actions) lands. Each item lists file pointers + a runnable repro.

Prioritized fix order: B3 (Wurmcoil comma-token-split — smallest, clearest, sibling-general) -> B1 (Everflowing XKicked ETB count) -> B2 (RepeatEach token payoff, sibling of 2005) -> B4 (Memoricide NameCard) -> B6 (Summoning Trap StoreSVar) -> B5 (Sorin ControlPlayer ultimate, lowest value).

== B3 [BROKEN, HIGH VALUE / SMALLEST FIX] Wurmcoil Engine — comma-separated TokenScript list not split ==
Card: cardsfolder/w/wurmcoil_engine.txt (deck 04 main 1x + SB-context Eldrazi).
Script: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | Execute$ TrigToken ; SVar:TrigToken:DB$ Token | TokenScript$ c_3_3_a_phyrexian_wurm_deathtouch,c_3_3_a_phyrexian_wurm_lifelink.
Symptom: dies-trigger FIRES (log: 'Trigger: Wurmcoil Engine - When CARDNAME dies, create a 3/3 ... deathtouch ... and a 3/3 ... lifelink') but the engine takes the WHOLE comma-joined string as a single token name: 'Warning: Token script 'c_3_3_a_phyrexian_wurm_deathtouch,c_3_3_a_phyrexian_wurm_lifelink' not found' + '[WARN] Token definition not found ... - skipping token creation'. RESULT: ZERO tokens created — both the deathtouch and lifelink Wurm are lost.
EMPIRICAL: confirmed firing in debug/2010survey/v3_ub_vs_eldrazi.log ('Wurmcoil Engine (57) dies from combat damage' -> trigger -> token-not-found, no tokens enter).
PROOF it's a split bug not missing data: BOTH individual files exist —
  forge-java/forge-gui/res/tokenscripts/c_3_3_a_phyrexian_wurm_deathtouch.txt
  forge-java/forge-gui/res/tokenscripts/c_3_3_a_phyrexian_wurm_lifelink.txt
Root cause: mtg-engine/src/loader/effect_converter.rs ApiType::Token arm (~line 1152): 'let token_script = params.get("TokenScript")?.to_string();' takes the raw param; never splits on ','. Java Forge splits TokenScript$ on comma and creates one token per entry. Generalizes to every multi-token card (Wurmcoil, Hangarback, Ghave-style, any 'create A and B' with a list).
Repro:

```sh
cat > /tmp/wurmcoil.pzl <<'P'
[metadata]
Name=Wurmcoil Dies Token Test
Goal=Survive
Turns=2
[state]
turn=3
activeplayer=p0
activephase=MAIN1
p0life=20
p0battlefield=Wurmcoil Engine
p0library=Forest;Forest;Forest
p1life=20
p1battlefield=Mountain
p1hand=Lightning Bolt;Lightning Bolt
p1library=Forest;Forest
P
./target/release/mtg tui --start-state /tmp/wurmcoil.pzl --p1 heuristic --p2 heuristic --seed 7 -v 3 2>&1 | grep -iE 'Wurmcoil.*dies|Phyrexian Wurm|token script.*not found|Created.*Wurm'
```
Expected (BUG today): 'Wurmcoil ... dies' + trigger + 'Token script ...not found' and NO 'Created ... Wurm Token'. After fix: two 'Created Phyrexian Wurm Token' lines (one deathtouch, one lifelink).

== B1 [BROKEN] Everflowing Chalice — etbCounter 'XKicked' (Count$TimesKicked) non-numeric, drops charge counters ==
Card: cardsfolder/e/everflowing_chalice.txt (deck 04, ramp; up to 3x + more via Eldrazi ramp).
Script: K:Multikicker:2 ; K:etbCounter:CHARGE:XKicked:no Condition:... ; SVar:XKicked:Count$TimesKicked ; A:AB$ Mana | Cost$ T | Produced$ C | Amount$ X ; SVar:X:Count$CardCounters.CHARGE.
Symptom: '[WARN] apply_etb_counters: non-numeric amount 'XKicked' on Everflowing Chalice not yet supported' (46x in tourney). The ETB charge-counter amount XKicked = Count$TimesKicked (number of times the multikicker was paid) is not evaluated; apply_etb_counters bails and the Chalice enters with ZERO charge counters. Its mana ability ({T}: Add {C} per charge counter) then produces 0 -> a kicked Chalice is a dead mana rock. (Unkicked Chalice = 0 counters by design, harmless.)
Root cause class: apply_etb_counters (mtg-engine/src/game/actions/mod.rs:1333, warn at :1363) doesn't resolve Count$TimesKicked / non-numeric SVar amounts for etbCounter. Needs kicker-count plumbing into the ETB-counter site. Affects every '...enters with a counter for each time it was kicked' card (Everflowing Chalice, Hangarback Walker-adjacent, etc.).
Repro: the deck04 tourney emits the warn; a clean puzzle would multikicker-cast Chalice (needs kicker-cost input plumbing — note casting kicked may itself be the harder part).

== B2 [BROKEN/PARTIAL] Terastodon — RepeatEach token payoff unimplemented (sibling of 2005) ==
Card: cardsfolder/t/terastodon.txt (deck 04, 1x).
Script: ETB Destroy up-to-3 noncreature permanents (DB$ Destroy, IMPL) -> SubAbility$ MakeTokens ; SVar:MakeTokens:DB$ RepeatEach | RepeatSubAbility$ DBToken | DefinedCards$ Targeted ; SVar:DBToken:DB$ Token | ... g_3_3_elephant | TokenOwner$ RememberedController.
Symptom: '[WARN actions] Unimplemented effect 'RepeatEach' resolved as no-op' (11x). RepeatEach is NOT in ApiType enum -> Unknown -> converter catch-all -> no-op. The Destroy half likely resolves; the 'for each permanent put into a graveyard this way, its controller creates a 3/3 Elephant' compensation is NEVER made. So Terastodon strictly over-performs (destroys without giving the Elephants back).
Root cause class: missing ApiType::RepeatEach + the per-element repeat-subability loop (iterate DefinedCards, bind Remembered, run sub-ability). SAME gap as 2005 backlog (mtg-nzozr referenced RepeatEach-adjacent) — coordinate one fix. Affects Terastodon, Avenger-of-Zendikar-style 'each' payoffs that route through RepeatEach, etc.
Repro: deck04 tourney/v3 logs show the warn after a Terastodon ETB; a puzzle placing Terastodon + 3 opponent noncreature permanents reproduces deterministically.

== B4 [UNVERIFIED, suspect BROKEN] Memoricide — NameCard (ApiType::Unknown) ==
Cards: cardsfolder/m/memoricide.txt (deck 02 main 2x + SB 3x; deck 01 SB 3x).
Script: A:SP$ NameCard | Defined$ You | ValidCards$ Card.nonLand -> SubAbility$ DBExile (ChangeZone Card.NamedCard from Graveyard,Hand,Library to Exile).
Symptom: NameCard is NOT in the ApiType enum (ability_parser.rs) -> ApiType::Unknown -> converter catch-all -> Effect::Unimplemented no-op. The DBExile sub keys off Card.NamedCard which is never set, so nothing is exiled. NO runtime evidence — Memoricide was never cast by the heuristic in any survey seed (it's a hate card the AI doesn't value). Likely BROKEN but UNCONFIRMED.
Root cause class: missing ApiType::NameCard + 'choose a card name' choice infra + Card.NamedCard predicate wiring (SAME gap as 2005 B5: Pithing Needle / Cranial Extraction). Coordinate one fix across both surveys. Needs a targeted puzzle (cast Memoricide at an opponent with a known library/hand/graveyard, assert the named card is exiled from all zones).

== B5 [UNVERIFIED, suspect partial] Sorin Markov — ControlPlayer ultimate (ApiType::Unknown) ==
Card: cardsfolder/s/sorin_markov.txt (deck 01 SB 1x, deck 02 SB 1x).
Script: A:AB$ DealDamage (+2, 2 dmg any target) + DBGainLife (IMPL) ; A:AB$ SetLife (-3, opp life becomes 10; IMPL) ; A:AB$ ControlPlayer (-7, control target player next turn).
Symptom: ControlPlayer is NOT in the ApiType enum -> Unknown -> no-op. The +2 and -3 abilities use IMPL arms (DealDamage/GainLife/SetLife) and are LIKELY WORKING. The -7 mind-control ultimate would silently do nothing. Sorin never activated in any survey seed (sideboard 1-of) -> no evidence for any of the three abilities.
Root cause class: missing ApiType::ControlPlayer + the take-control-of-a-player's-turn engine machinery (large feature; rare). Lowest priority. Needs a planeswalker puzzle to confirm the two IMPL abilities and the no-op ultimate.

== B6 [UNVERIFIED] Summoning Trap — StoreSVar trackers (ApiType::Unknown) for the free-cast trigger ==
Card: cardsfolder/s/summoning_trap.txt (deck 04, 4x).
Script: T:Mode$ Countered | ValidSA$ Spell.Creature+wasCastByYou -> Execute$ TrackValidCounters ; SVar:TrackValidCounters:DB$ StoreSVar ... ; S:Mode$ AlternativeCost | Cost$ 0 | CheckSVar$ SetTrap (pay {0} if a creature spell you cast was countered this turn). Main: A:SP$ Dig | DigNum$ 7 | ChangeValid$ Creature -> Battlefield (IMPL).
Symptom: StoreSVar is NOT in the ApiType enum -> Unknown -> no-op. The 'countered-creature' counter that arms the {0} AlternativeCost is never incremented, so the free-cast discount likely never triggers. The Dig-7-put-a-creature MAIN ability uses IMPL Dig (DigMultiple) and is LIKELY WORKING at full mana cost. No survey game had a creature spell countered then Summoning Trap cast free -> AlternativeCost path UNVERIFIED.
Root cause class: missing ApiType::StoreSVar (SVar mutation effect) + CheckSVar-gated AlternativeCost evaluation. Needs a puzzle: opponent counters your creature spell, then cast Summoning Trap for {0}.

== Cross-links ==
Tracker mtg-38g7u. Umbrella mtg-684. Sibling backlogs: mtg-713 (1994), mtg-902 (2020), mtg-v59ll (2025), mtg-nzozr (2005). Shared root-cause gaps with 2005: RepeatEach (B2 ~ 2005), NameCard (B4 ~ 2005 B5) — fix once, fix both. Survey artifacts (gitignored): debug/2010survey/{tourney.log, v3_ub_vs_eldrazi.log, v3_ub_mirror.log, v3_eldrazi_mirror.log, unique_cards.txt}.
