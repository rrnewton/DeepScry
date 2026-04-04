---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-04-04T12:24:25.740557762+00:00
---

# Description

Track implementation of MTG game features including keywords, abilities, card effects, and the card script parsing infrastructure.

## Effect Bugfixes (2026-04-04_#1870(98d588d1))

**Card Parsing & Selectors:**
- mtg-147: Unhandled Affected$ selectors tracking (270 Unknown Affected$ remaining)

**ETB & Replacement Effects:**
- mtg-zeuy0: Thriving Grove doesn't enter tapped or prompt for color choice (affects all Thriving lands)

**Effect Bugfixes (incoming):**
- mtg-e78d1f: Bazaar of Baghdad SubAbility chain fix in parse_activated_abilities()
- mtg-fd5bf7: ExtraTurn effect (Time Walk) - AddTurn ApiType + extra_turns queue
- mtg-9915fe: DestroyAll effect (Nevinyrral's Disk) - filter-based mass destroy
- mtg-66e8cd: Replace silent no-ops with Effect::Unimplemented for visible warnings
- ApiType::Attach now handled in params_to_effect() (was special-cased only in extract_effects_from_svar)

**Keywords:**
- ✅ Living Weapon keyword parsing fixed (38 cards)
- ✅ For Mirrodin keyword parsing fixed (30 cards)
- ✅ Umbra armor keyword parsing fixed (30 cards)
- ✅ Partner variants (- Survivors, - Father & Son, - Character select) fixed (22 cards)
- ✅ Bare Vanishing (no counter) fixed (4 cards)
- ✅ Protection variants fixed (8 cards)
- ✅ Lure effects (must be blocked) fixed (50 cards)

**AB$ ChooseColor Effect (2026-04-03_#2059):**
- ✅ AB$ ChooseColor: choose a color and store on source card (30 cards)
- AI heuristic: pick most prominent color in deck (pick_prominent_color)
- SubAbility chaining enables color change, protection, discard filtering
- Examples: Caldera Kavu, Spiritmonger, Crosis the Purger, Skrelv
- Tests: 4 new tests (unit + integration)
- Issue: mtg-dxjtq (CLOSED)

**Variable P/T:**
- ✅ Parsing implemented (X, Y, Z, AffectedX, Count$ references)
- ⏳ Runtime evaluation still TODO (values default to 0)

## Recent Completions

**TapOrUntap Effect (2026-03-25_#1987(38586cdb)):**
- ✅ AB$ TapOrUntap: tap or untap target permanent (49 cards)
- AI heuristic: untap our permanents, tap opponent's
- Examples: Bounding Krasis, Captain of the Mists, Component Collector

**MultiplyCounter Effect (2026-03-25_#1978(911880cb)):**
- ✅ AB$ MultiplyCounter: counter doubling/multiplying (44 cards)
- Doubles (or multiplies by N) counters on a permanent
- Supports specific counter types or "all counters" mode
- Examples: Ascendant Acolyte, Aetheric Amplifier, Aragorn Hornburg Hero

**SacrificeAll Effect (2026-03-14_#1950(46f42a1e)):**
- ✅ AB$ SacrificeAll: mass sacrifice (143 cards)
- Each player sacrifices all permanents matching ValidCards$ filter
- Bypasses indestructible and regeneration (CR 701.17)
- Examples: All is Dust, Archfiend of Depravity

**ChangeZoneAll Effect (2026-03-14_#1940(a6fb17e)):**
- ✅ AB$ ChangeZoneAll: mass zone changes (636 cards)
- Moves cards matching ChangeType$ filter between Origin$ and Destination$ zones
- Supports Battlefield and Graveyard origins
- Examples: Aetherize, Tormod's Crypt, All Hallow's Eve, Aether Snap

**PutCounterAll Effect (2026-03-14_#1933(3e3513d)):**
- ✅ AB$ PutCounterAll: mass counter placement (264 cards)
- Puts counters on all permanents matching ValidCards$ filter
- Examples: Ajani the Greathearted, Arcbound Overseer, Anduril

**Combat Restrictions, Damage Prevention & Alternate Costs (2026-03-14_#1931(823b4bd)):**
- ✅ CantAttackAlone, CantAttackOrBlockAlone (22 cards)
- ✅ PreventAllDamage, PreventAllCombatDamage, PreventAllCombatDamageDealtAndReceived (20 cards)
- ✅ UntapsDuringOthersUntapStep (8 cards), CanBlockShadow (6 cards)
- ✅ DeckAnyNumber (20 cards), CanBeCommander (10 cards), AnteRemoval (18 cards)
- ✅ AlternateAdditionalCost parameterized (62 cards)
- ✅ MustBeBlockedByAllFiltered, MayEffectFromOpeningDeck, Prize (14 cards)
- ✅ Trample:Planeswalker (4 cards)
- 182 fewer keyword parsing warnings (528 → 346)

**Protection & Lure Keyword Parsing (2026-03-12_#1924(6b3b518)):**
- ✅ "Protection from everything" (4 cards: Progenitus, Hexdrinker)
- ✅ "Protection from each color" (4 cards: Etched Champion, Iridescent Angel)
- ✅ "CARDNAME must be blocked if able" - Lure effect (24 cards)
- ✅ "All creatures able to block CARDNAME do so" (22 cards)
- ✅ "CARDNAME must be blocked by two or more creatures if able" (2 cards)
- ✅ "CARDNAME must be blocked by exactly one creature if able" (2 cards)
- 54 fewer keyword parsing warnings (582 → 528)

**Partner & Vanishing Keyword Variants (2026-03-12_#1919(4871277)):**
- ✅ "Partner - Survivors" variant (8 cards)
- ✅ "Partner - Father & Son" variant (4 cards)
- ✅ "Partner - Character select" variant (10 cards)
- ✅ Bare "Vanishing" for ETB counter cards (4 cards)
- 26 fewer keyword parsing warnings (608 → 582)

**Keyword Text Variants (2026-03-12_#1917(a5f047a)):**
- ✅ "For Mirrodin" variant (card files omit the "!")
- ✅ "Living Weapon" variant (capital W)
- ✅ "Umbra armor" variant (alternate spelling)
- 98 fewer keyword parsing warnings (706 → 608)

**DealsCombatDamage Triggers (2026-03-12_#1916(bc98cc2)):**
- ✅ Fire DealsCombatDamage triggers at runtime when creatures deal combat damage
- Enables Hypnotic Specter, Ophidian, etc. to work correctly

## Completed Work (older)

**New Effect Types (2026-03-07_#1872(e04b78d)):**
- ✅ ForceSacrifice (891 card usages) - Diabolic Edict, Barter in Blood
- ✅ TapAll (64 card usages) - Sleep, Cryptic Command tap mode
- ✅ UntapAll (100 card usages) - Mobilize, Aggravated Assault
- ✅ SetLife (39 card usages) - Angel of Grace, Blessed Wind

Checked up-to-date as of 2026-03-26_#1997(bba0fbb0) - 942 tests passing

## AB$ Dig Enhancement (2026-04-03_#2063(5d3489e8))

**Full Dig effect implementation (192 cards):**
- ✅ ChangeValid$ filter: AI selects only matching card types (Creature, Land, etc.)
- ✅ DestinationZone2$: Non-selected cards go to correct zone (Graveyard, Exile, etc.)
- ✅ Partial selection (ChangeNum < DigNum): AI ranks and picks best N cards
- ✅ Optional$ support: AI skips when no good cards available
- ✅ RestRandomOrder$: shuffle non-selected cards before putting on bottom
- ✅ Reveal$ logging: proper 'reveals' vs 'looks at' messages
- ✅ DigFilter enum: 9 card type variants including Permanent
- ✅ Library ordering fix: dig from top (not bottom)
- Examples: Impulse, Wrenn and Seven, Seismic Sense, Trail of Crumbs

## AB$ Proliferate Effect (2026-04-03_#2067(pending))

**Full Proliferate implementation (89 card files, mtg-mr0v1 CLOSED):**
- ✅ AB$ Proliferate: choose any number of permanents with counters, give each one additional counter of each kind (CR 701.34a)
- No parameters needed - pure effect (simplest AB$ type)
- AI heuristic: classified as always-beneficial (like PutCounter, MultiplyCounter)
- Targeting: NoTargetNeeded (choices made during resolution)
- Execution: iterates battlefield permanents with counters, adds 1 of each counter type
- Examples: Yawgmoth Thran Physician, Martyr for the Cause, Metastatic Evangel, Merfolk Skydiver
- Tests: 3 new unit tests (basic, no-cost, with SubAbility)

## AB$ Debuff Effect (2026-04-03_#2065(819fc050))

**Full Debuff implementation (23 cards, 26 usages):**
- ✅ AB$ Debuff: remove keywords from creatures (inverse of Pump keyword granting)
- Parses Keywords$ parameter (split by " & ") for keyword removal
- AI heuristic: activates "lose Defender" in Main1 to enable attacking
- Supports self-targeting (Defined$ Self) and opponent targeting (ValidTgts$)
- Full undo support restores removed keywords
- Examples: Grozoth, Gargoyle Sentinel, Manor Gargoyle, Phyrexian Splicer
- Tests: 3 new unit tests (parsing)

## AB$ AnimateAll Effect (2026-04-03_#2069(f772a07a))

**Mass animation implementation (26 card files, mtg-tquvf CLOSED):**
- ✅ AB$ AnimateAll: set base P/T and/or grant keywords to all matching permanents
- ValidCards$ filter: Creature.YouCtrl, Planeswalker.YouCtrl, Permanent.OppCtrl, etc.
- Optional Power$/Toughness$ base P/T setting + Keywords$ granting
- AI heuristic: classified as always-beneficial (like PumpAllCreatures)
- Examples: Sarkhan the Masterless, Oko the Trickster, Shadowspear, Mirror Entity
- Tests: 4 new unit tests (parsing variants)

## AB$ PreventDamage Effect (2026-04-03_#2071(pending))

**Damage prevention shield implementation (81 card files, mtg-rhqes CLOSED):**
- ✅ AB$ PreventDamage: create damage prevention shield on target (CR 615.1)
- damage_prevention field on Card and Player, cleared at cleanup step
- Prevention checked in deal_damage() and deal_damage_to_creature()
- Supports ValidTgts$ (Any, Creature) and Defined$ (Self, You)
- AI heuristic: activates during combat phases (like Regenerate)
- Examples: Militant Monk, Master Healer, Eiganjo Castle, Esper Battlemage
- Tests: 8 new tests (4 parsing + 4 execution)

# Notes

2026-03-07_#1869: LoseLife (108 cards), DestroyAll (34 cards), DamageAll (58 cards) implemented. Board wipes (Wrath of God) and mass damage (Pyroclasm) now work.
2026-03-10_#1898: AB$ Fight effect (125+ cards, CR 701.12) implemented.
