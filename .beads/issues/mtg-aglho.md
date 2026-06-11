---
title: 1995 Championship — broken-card root-cause backlog (B1-B5)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-huyhe: parent-child
  mtg-684: parent-child
created_at: 2026-06-11T05:47:11.984499433+00:00
updated_at: 2026-06-11T05:47:20.105015159+00:00
---

# Description

1995 World Championship compat sweep — broken/partial-card root-cause backlog. Compiled by agent compat-1995-survey (slot07) from a static script-vs-converter + static-ability-enum scan of all 66 unique cards, 800 tourney games (4x200 mirror, seed 7), verbosity-3 AI games, and 3 targeted puzzle reproducers. Rolls up under the 1995 TRACK bead mtg-huyhe (umbrella mtg-684).

STAMP: 2026-06-10_#3177(9e100acf)

NOTE: SURVEY-ONLY pass — NO engine/web source edited (a separate effects/actions refactor owns those files). All fixes below DEFERRED until that refactor lands. Reproducer evidence in gitignored debug/compat1995/ on branch claude/compat-1995-survey.

HEADLINE: 1995 is the healthiest championship year surveyed. Only 5 cards broken/partial (~8%), ALL sideboard or niche — NO maindeck combo engine is broken. The marquee prison/control/reanimator/burn cards (Black Vise, The Rack, Winter Orb, Dance of the Dead, Hypnotic Specter, Sengir Vampire, Royal Assassin, the burn suite, the global sweepers) are all confirmed WORKING.

Decks: 01_blumke_bw_rack, 02_hernandez_rw_vise_orb, 03_justice_red_artifact (LOW-CONFIDENCE reconstruction — see tracker), 04_stern_rg_burn.

== Prioritized backlog (all LOW value — no maindeck breakage) ==

B1 [BROKEN, low value] Magical Hack + Sleight of Mind
  Cards: cardsfolder/m/magical_hack.txt, cardsfolder/s/sleight_of_mind.txt (both in 01_blumke sideboard, 1 copy each)
  Script: A:SP$ ChangeText | ValidTgts$ Card | ChangeTypeWord$/ChangeColorWord$ ...
  Root cause: ApiType ChangeText has NO parser variant in ability_parser.rs (it is not in the ApiType enum at all) -> the spell converts to Effect::Unimplemented. Casting it changes no text.
  Impact: niche text-changing tech (swap a basic land type / a color word). Strictly a sideboard answer card; near-zero metagame value. PARTIAL-toward-BROKEN.
  Fix shape: add ChangeText ApiType + a continuous text-modification effect (replace one basic land type / color word with another, Duration$ Permanent). Bigger feature for tiny payoff; lowest priority.

B2 [BROKEN, low value] Lhurgoyf
  Card: cardsfolder/l/lhurgoyf.txt (04_stern sideboard, 2 copies)
  Script: PT:*/1+* ; S:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X | SetToughness$ Y (X=creature cards in all graveyards, Y=X+1)
  Root cause: continuous CharacteristicDefining SetPower/SetToughness is NOT in the StaticAbility runtime enum (ModifyPT/GrantKeyword/ReduceCost/RaiseCost/GrantAbility/GainControl/SacrificeMatchingPresent/CantBeCast/CantPlayLand/CantBlockMatching/CastWithFlash). SetPower is only handled inside CopyPermanent. Lhurgoyf never recomputes its P/T from the graveyards; it sits at its base (effectively 0/1).
  Empirical: lhurgoyf_test.pzl (3 creature cards in graveyards -> expected 3/4). The AI declined to attack into an empty board with its only creature — consistent with a 0/1 valuation, not a 3/4.
  Fix shape: implement continuous CharacteristicDefining P/T as a real layer-7b CDA static recomputed from a Count$ expression. THIS IS THE SAME ENGINE GAP as 2000's Serra Avatar (B4) and Opalescence (B5) in backlog mtg-xxtl0 — drive it from there (those are high-value combo pieces; Lhurgoyf is a sideboard 2-of that rides along for free once the CDA static exists).

B3 [PARTIAL, niche] Orgg
  Card: cardsfolder/o/orgg.txt (04_stern, 2 copies)
  Script: S:Mode$ CantAttack | ValidCard$ Card.Self | UnlessDefender$ !controlsCreature.untapped+powerGE3 ; S:Mode$ CantBlockBy | ValidAttacker$ Creature.powerGE3 | ValidBlocker$ Creature.Self
  Root cause: the CantBlockBy half is special-cased and the plain CantAttackOrBlock keyword exists, but the CONDITIONAL "can't attack unless the defending player controls NO untapped power>=3 creature" form is not modelled as a runtime static (CantAttack with an UnlessDefender$ predicate is not in the StaticAbility enum). Orgg may be allowed to attack in situations where it should be restricted (and vice-versa for the block half — needs confirmation).
  Impact: a 6/6 beater with a downside that is probably just ignored -> Orgg plays as a vanilla-ish 6/6. Slightly STRONGER than printed. Low priority (sideboard-ish fattie in a burn deck).
  Fix shape: conditional CantAttack static evaluated against the defending player's board at declare-attackers. Generalizes to other "can't attack if defender controls X" creatures.

B4 [VERIFY] Island Sanctuary
  Card: cardsfolder/i/island_sanctuary.txt (02_hernandez, 2 copies)
  Script: R:Event$ Draw | ActivePhases$ Draw | PlayerTurn$ True | Optional$ True | ReplaceWith$ SanctuaryEffect ("skip your draw -> until your next turn you can only be attacked by fliers/islandwalkers")
  Status: a skip-draw-for-evasion-protection replacement. No clear support found for an Event$ Draw replacement that grants a conditional can't-be-attacked-except-by shield. Needs a targeted puzzle: activate Island Sanctuary, skip the draw, then have a non-flying attacker declared and assert it is an illegal attack. Classify after confirmation; PARTIAL pending.

B5 [VERIFY] Spirit Link + Prismatic Ward
  Cards: cardsfolder/s/spirit_link.txt (01_blumke, 1), cardsfolder/p/prismatic_ward.txt (01_blumke SB, 1)
  Spirit Link: T:Mode$ DamageDealtOnce | ValidSource$ Card.AttachedBy | Execute$ TrigGain (gain life = damage dealt by enchanted creature). Confirm the DamageDealtOnce trigger mode fires and the GainLife matches the damage amount.
  Prismatic Ward: K:ETBReplacement:Other:ChooseColor + R:DamageDone prevent-by-chosen-color. Color-based damage-prevention infra EXISTS (core/prevention.rs colored_source_next_event) -> Prismatic Ward is LIKELY WORKING; confirm with a puzzle (chosen color source deals 0 to the enchanted creature; off-color source deals normally).

== Cards confirmed WORKING this pass (evidence-backed — NOT in backlog) ==
Black Vise (puzzle: handsize-4 dmg to chosen opp), The Rack (puzzle: 3-handsize dmg), Winter Orb (UntapAdjust:Land:1 lock — code path + regression test), Dance of the Dead (reanimates Sengir Vampire / Hypnotic Specter / Royal Assassin from graveyard onto battlefield; correctly self-sacrifices when no valid target), Hypnotic Specter (2/2 flier + discard-on-combat-damage trigger), Sengir Vampire (4/4 flier, grows via +1/+1 counter on a creature death, counterable by Power Sink), Royal Assassin (tap: destroy target tapped creature), Armageddon + Wrath of God + Jokulhaups + Anarchy + Pyroclasm + Earthquake (mass destruction / DamageAll), Hymn to Tourach + Mind Twist + Disrupting Scepter (discard), Howling Mine (symmetric extra draw), Lightning Bolt + Incinerate + Fireball (burn), Llanowar Elves + Dark Ritual (mana accel), Power Sink (counter), Land Tax (basic-land search), basic lands. Logs: debug/compat1995/v3_{rack,vise,burn}.log + *_out.log puzzle captures.

== Cards LIKELY-FINE but UNVALIDATED (constructs IMPL; next agent should puzzle-verify) ==
Zuran Orb (sac land: gain 2), Ivory Tower (upkeep lifegain if 5+ cards), Fellwar Stone (ManaReflected), Icy Manipulator (tap target), Mishra's Factory (Animate man-land + Pump), the Circles of Protection (ChooseSource + prevention Effect), Disenchant + Crumble + Terror + Dark Banishing (Destroy), Swords to Plowshares (exile + lifegain), Balance (symmetric rebalance), Pestilence (DamageAll + Sacrifice — never cast in games), Stormbind (discard-to-ping), Spirit Link/Prismatic Ward (B5 verify), Orcish Lumberjack (sac Forest for RRR), Whirling Dervish (protection-from-black + pump), Shivan Dragon (firebreathing Pump), Channel (life-for-mana Effect), Bottomless Vault + Ruins of Trokair + Dwarven Ruins (storage/sac lands), Strip Mine + Mishra's Factory (utility lands), Magical Hack/Sleight of Mind are B1. Headline: evidence-backed WORKING ~22/66 (~33%); estimated functional (WORKING + likely-fine) ~61/66 (~92%); known broken/partial 5/66 (~8%).

Driven by agent compat-1995-survey (slot07), 2026-06-10 overnight survey pass.
