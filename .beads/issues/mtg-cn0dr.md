---
title: 2015 Championship — broken-card root-cause backlog (B1-B4)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-8ikty: parent-child
created_at: 2026-06-11T04:05:23.002147058+00:00
updated_at: 2026-06-11T04:05:28.630888078+00:00
---

# Description

2015 World Championship compat sweep — broken/partial-card root-cause backlog. Compiled by agent compat-2015-survey (slot07) from a static script scan of all 59 unique cards + runtime warning scan of 4x200-game mirror tournaments + per-deck verbosity-3 game logs. Rolls up under the 2015 TRACK bead mtg-8ikty (umbrella mtg-684).

STAMP: 2026-06-10_#3175(c6dbd34f)

Survey artifacts (gitignored, on branch claude/compat-2015-survey):
  debug/survey2015/tourney_<deck>.log   (mtg tourney --mirror-only --games 200 --seed 7, per deck)
  debug/survey2015/game_<deck>.log      (mtg tui mirror, --verbosity 3, seed 7)

4 unique cards are BROKEN (all 4 emit WARN/no-op rather than crashing — the championship mirror crash rate is 0%). Headline: 55/59 unique cards (93.2%) WORKING.

== Prioritized backlog ==

B1 [BROKEN, HIGH VALUE — DO FIRST] Hangarback Walker
  Card: cardsfolder/h/hangarback_walker.txt (4x main in 03_rietzl_abzan_aggro, 3x main in 04_black_mono_white)
  Script: ManaCost:X X | PT:0/0 | K:etbCounter:P1P1:X | SVar:X:Count$xPaid
          (dies-trigger makes 1/1 flying Thopters = # of +1/+1 counters)
  Root cause: apply_etb_counters (mtg-engine/src/game/actions/mod.rs:1363) only handles a NUMERIC counter amount; the non-numeric 'X' (= mana paid for the X X cost) is "not yet supported", so the creature enters as a 0/0 with ZERO counters and is destroyed by state-based action immediately, never making any Thopters.
  Empirical evidence: WARN "apply_etb_counters: non-numeric amount 'X' on Hangarback Walker not yet supported" fired 411x in debug/survey2015/tourney_03_rietzl_abzan_aggro.log and 284x in tourney_04_black_mono_white.log.
  Related: mtg-291 (closed) fixed the etbCounter keyword for a NUMERIC amount (Triskelion P1P1:3). This is the distinct still-open X-valued-amount case — the X must be resolved from xPaid (the X chosen when paying the X X cost) at ETB. Generalizes to other "enters with X +1/+1 counters" cards (Walking Ballista, Endless One, Hangarback siblings). COORDINATE with effect_converter / actions refactor (mtg-245) before editing actions/mod.rs.
  Repro:
```sh
./target/release/mtg tourney decks/championship/2015/03_rietzl_abzan_aggro.dck \
  --mirror-only --games 50 --seed 7 2>&1 | grep -c "Hangarback Walker not yet supported"
```
  Expected today: a nonzero count (the warning fires whenever Hangarback is cast).

B2 [BROKEN, runtime-confirmed] Tragic Arrogance
  Card: cardsfolder/t/tragic_arrogance.txt (1x main in 01_manfield_abzan_control + sideboards of 01/03/04)
  Script: A:SP$ RepeatEach | RepeatPlayers$ Player | RepeatSubAbility$ YouChoose | SubAbility$ SacAllOthers ...
  Root cause: ApiType RepeatEach is NOT in the ApiType enum (loader/ability_parser.rs) -> parses to Unknown -> Effect::Unimplemented, resolved as no-op. The ENTIRE card effect is the RepeatEach wrapper, so the sorcery does literally nothing (each player keeps everything). The World Champion's signature board-wipe (his stated trump vs Hangarback Walker) is a dead card.
  Empirical evidence: 33x "Unimplemented effect 'RepeatEach' resolved as no-op" in debug/survey2015/tourney_01_manfield_abzan_control.log (Tragic Arrogance is the ONLY RepeatEach card in deck 01).
  Shared gap: mtg-651 already tracks RepeatEach (Sylvan Library full chain). Add Tragic Arrogance as a second motivating card there; needs RepeatEach + per-player ChooseCard (ChooseEach$ Artifact & Creature & Enchantment & Planeswalker) + SacrificeAll(!IsRemembered).

B3 [BROKEN, static] Mastery of the Unseen
  Card: cardsfolder/m/mastery_of_the_unseen.txt (4x main in 04_black_mono_white)
  Script: A:AB$ Manifest | Cost$ 3 W  ; T:Mode$ TurnFaceUp ... gain 1 life per creature
  Root cause: ApiType Manifest is NOT in the ApiType enum -> Unimplemented. The {3}{W} manifest activated ability is the card's whole engine; with it dead, the "whenever a permanent is turned face up, gain life" trigger can never fire (nothing is ever manifested face-down). Static enchantment parses but the card is functionally inert in mono-white devotion.
  Engine note: face-down/Manifestation counter infra partially exists (CounterType::Manifestation in core/types.rs); Morph/Megamorph turn-face-up path exists. Manifest needs a converter arm + the "put top card face down as 2/2" action wired to that infra.
  Repro: force a game where P1 has Mastery + lands; activate {3}{W}; expected today: the activated ability is never offered / no face-down permanent appears.

B4 [BROKEN, static] Palace Siege
  Card: cardsfolder/p/palace_siege.txt (sideboard in 02_turtenwald_abzan_control)
  Script: K:ETBReplacement:Other:SiegeChoice ; SVar:SiegeChoice:DB$ GenericChoice | Choices$ Khans,Dragons | SetChosenMode$ True ...
          S:Mode$ Continuous | Affected$ Card.Self+ChosenModeKhans | AddTrigger$ KhansTrigger
          S:Mode$ Continuous | Affected$ Card.Self+ChosenModeDragons | AddTrigger$ DragonsTrigger
  Root cause TWO-part: (1) ApiType GenericChoice is NOT in the ApiType enum -> the "as it enters, choose Khans or Dragons" mode-select is unimplemented (no mode is ever recorded). (2) loader/card.rs:4283 maps the selector 'Card.Self+ChosenMode*' to AffectedSelector::Self_ UNCONDITIONALLY, ignoring the ChosenModeKhans/ChosenModeDragons discriminator. Net effect: if the static abilities apply at all, BOTH the Khans (recur a creature each upkeep) and Dragons (drain 2 each upkeep) triggers attach — incorrectly STRONGER than printed (a modal card should pick exactly one). Sideboard 1-of, low play frequency, but mechanically wrong.
  Fix shape: add a GenericChoice converter arm that records the chosen mode (SetChosenMode), and make the ChosenMode* selector match the recorded mode instead of the unconditional Self_ shortcut. Generalizes to all "As ~ enters, choose A or B" Siege-cycle enchantments (Citadel/Outpost/Frontier/Mountain Siege).

== Cards CONFIRMED playable end-to-end (no crash, constructs IMPL) but rarer keyword behavior NOT yet captured by targeted puzzle ==
Den Protector (Megamorph turn-face-up -> graveyard-return trigger), Herald of Torment (Bestow as-aura +3/+3+flying), Silence the Believers (Strive per-target extra cost + exile-with-auras), Murderous Cut / Tasigur (Delve cost reduction), the planeswalker loyalty suites (Elspeth Sun's Champion tokens/-7 destroy, Ugin the Spirit Dragon -X exile / +2 / ultimate emblem, Sorin Solemn Visitor, Ajani Mentor of Heroes), Kytheon/Nissa flip-walker transforms, Mastery-adjacent face-down interactions. These default to UNVERIFIED in per-card tracking (promote to WORKING with targeted puzzles per compatibility_tracking SKILL).
