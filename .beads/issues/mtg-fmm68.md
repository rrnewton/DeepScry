---
title: Julian Avatar Deck Compatibility
status: open
priority: 1
issue_type: task
created_at: 2026-01-17T23:09:03.931957063+00:00
updated_at: 2026-01-22T20:47:41.079396656+00:00
---

# Description

## Julian Avatar Deck Compatibility Tracking

This issue tracks compatibility testing for the **julian_avatar_deck.dck** deck (UG).

## Engine Implementation Gaps Summary

Key gaps affecting this deck:
- ~~**T:Mode$ Drawn** - "When you draw your second card" triggers~~ **FULLY IMPLEMENTED 2026-01-19** (includes undo support, opponent draw triggers for Underworld Dreams)
- ~~**T:Mode$ Taps** - "Whenever ~ becomes tapped" triggers (Gran-Gran)~~ **FULLY IMPLEMENTED 2026-01-20** (includes DiscardCards effect)
- ~~**T:Mode$ Phase** - Beginning of combat triggers (Avatar Kyoshi)~~ **FIXED 2026-01-21** (SubAbility$ chain to DBUntap now followed)
- ~~**Count$YouDrewThisTurn** - Track cards drawn this turn (Messenger Hawk)~~ **IMPLEMENTED 2026-01-21** (CountExpression::CardsDrawnThisTurn)
- ~~**Count$Valid** - Count creatures matching filter (Elephant-Mandrill)~~ **IMPLEMENTED 2026-01-21** (CountExpression::ValidPermanents with filter parsing)
- ~~**Exhaust$ True** - Once-per-game activation (Rebellious Captives)~~ **IMPLEMENTED 2026-01-22** (exhaust field on ActivatedAbility, exhausted_abilities tracking on Card)
- ~~**Ward:Waterbend<N>** - Ward with Waterbend cost (The Unagi)~~ **IMPLEMENTED 2026-01-21** (WardWaterbend keyword variant)
- ~~**T:Mode$ AttackersDeclared** - Batch trigger when attackers declared (Teo)~~ **IMPLEMENTED 2026-01-22** (AttackersDeclared TriggerEvent, ValidAttackers$ keyword filter)
- ~~**ImmediateTrigger / RememberDiscarded** - Complex conditional triggers (Teo's counter placement)~~ **IMPLEMENTED 2026-01-22** (remembered_cards on GameState, ImmediateTriggerCondition enum, DB$ Cleanup)
- ~~**DB$ EachDamage** - Multiple sources dealing damage (Allies at Last)~~ **IMPLEMENTED 2026-01-24** (Effect::EachDamage variant, damagers from parent targets, power-based damage)
- ~~**K:Affinity:Ally** - Cost reduction (Allies at Last)~~ **IMPLEMENTED 2026-02-10** (calculate_effective_cost in GameState and GameLoop, counts controlled permanents of specified type)
- ~~**S:Mode$ RaiseCost** - Additional sacrifice costs (Tectonic Split)~~ **IMPLEMENTED 2026-02-14** (RaisedCost::Mana and RaisedCost::Sacrifice variants)
- ~~**S:Mode$ ReduceCost** - Cost reduction static abilities (Gran-Gran)~~ **IMPLEMENTED 2026-02-13**
- ~~**DB$ Effect with StaticAbilities$** - Grant can't be blocked (Otter-Penguin, Giant Koi)~~ **FIXED 2026-01-21** (GrantCantBeBlocked placeholder resolution in CardDrawn triggers)
- ~~**Conditional hexproof (Condition$ PlayerTurn)**~~ **IMPLEMENTED 2026-01-21** (StaticCondition enum, Avatar Kyoshi)

---

## Deck Card Verification Checklist

### Lands (17):
- [x] Island (x10) - basic land (works)
- [x] Forest (x7) - basic land (works)

### Creatures (15):

**Avatar Kyoshi, Earthbender (x1)** - 8GGG 6/6 Legendary Human Avatar
- [x] Conditional hexproof during your turn (S:Mode$ Continuous | Condition$ PlayerTurn) **IMPLEMENTED 2026-01-21**
- [x] Beginning of combat trigger earthbend 8 (T:Mode$ Phase | Phase$ BeginCombat) **FIXED 2026-01-21**
- [x] Untap the earthbended land (SubAbility$ DBUntap) **FIXED 2026-01-21**

**Elephant-Mandrill (x1)** - 2G 3/2 Elephant Monkey
- [x] Reach keyword (VERIFIED 2026-01-19)
- [x] ETB each player creates Food token (VERIFIED 2026-01-19)
- [x] Beginning of combat pump based on opponent artifacts (Count$Valid Artifact.OppCtrl) **IMPLEMENTED 2026-01-21**

**Forecasting Fortune Teller (x2)** - 1U 1/3 Human Advisor Ally
- [x] ETB create Clue token (VERIFIED 2026-01-18)

**Giant Koi (x1)** - 4UU 5/7 Fish
- [x] Islandcycling {2} (VERIFIED 2026-01-19)
- [x] Waterbend 3: Can't be blocked (AB$ Effect with Waterbend cost) **FULLY WORKING 2026-02-13** (params_to_effect_with_svars resolves StaticAbilities$ SVar to GrantCantBeBlocked)

**Gran-Gran (x1)** - U 1/2 Legendary Human Peasant Ally
- [x] Taps trigger: draw then discard (VERIFIED 2026-01-20)
- [x] Cost reduction static (S:Mode$ ReduceCost based on Lessons in graveyard) **IMPLEMENTED 2026-02-13**

**Knowledge Seeker (x2)** - 1U 2/1 Fox Spirit
- [x] Vigilance (should work)
- [x] Second card drawn trigger: put +1/+1 counter (VERIFIED 2026-01-19)
- [x] Dies trigger: create Clue token (VERIFIED 2026-01-18)

**Otter-Penguin (x3)** - 1U 2/1 Otter Bird
- [x] Second card drawn trigger: pump +1/+2 (VERIFIED 2026-01-19)
- [x] "Can't be blocked" effect from second draw trigger **FIXED 2026-01-21** (GrantCantBeBlocked placeholder resolved to self)

**Raucous Audience (x1)** - 1R 2/2 Human Rebel
- [x] Mana ability with conditional Count$Compare **IMPLEMENTED 2026-02-19**

**Rebellious Captives (x1)** - 1G 2/2 Human Peasant Ally
- [x] Exhaust {6}: Put counters + earthbend 2 (Exhaust$ True) **IMPLEMENTED 2026-01-22**

**Teo, Spirited Glider (x1)** - 3U 1/4 Legendary Human Pilot Ally
- [x] Flying (should work)
- [x] Flying attackers trigger: draw/discard (T:Mode$ AttackersDeclared) **IMPLEMENTED 2026-01-22**
- [x] Conditional counter on discard nonland (ImmediateTrigger, RememberDiscarded) **IMPLEMENTED 2026-01-22**

**The Unagi of Kyoshi Island (x1)** - 3UU 5/5 Legendary Serpent
- [x] Flash (VERIFIED 2026-01-19)
- [x] Ward—Waterbend {4} (K:Ward:Waterbend<4>) **IMPLEMENTED 2026-01-21**
- [x] Opponent draws second card trigger (T:Mode$ Drawn | ValidPlayer$ Opponent) - works with 2026-01-19 Drawn trigger implementation

**Turtle-Duck (x1)** - 1U 0/4 Turtle Bird
- [x] AB$ Animate ability (VERIFIED in Gabriel deck - mtg-5hvly)

**Messenger Hawk (x1)** - 2UB 1/2 Bird Scout
- [x] Flying (VERIFIED 2026-01-19)
- [x] ETB create Clue token (VERIFIED 2026-01-19)
- [x] Static pump based on cards drawn this turn (Count$YouDrewThisTurn) **IMPLEMENTED 2026-01-21**

### Spells/Other (8):

**Abandon Attachments (x1)** - 1UR Instant Lesson
- [x] "You may discard. If you do, draw 2" (UnlessCost$ Discard | UnlessSwitched$ True) **FULLY WORKING 2026-02-19**

**Allies at Last (x2)** - 2G Instant
- [x] Affinity for Allies (K:Affinity:Ally) **IMPLEMENTED 2026-02-10**
- [x] Multi-target creature damage (DB$ EachDamage | ValidTgts$ Creature.OppCtrl) **IMPLEMENTED 2026-01-24**

**Ember Island Production (x1)** - 3UU Sorcery
- [x] SP$ Charm with CopyPermanent modes (VERIFIED 2026-01-18)

**Meteor Sword (x1)** - 7 Artifact Equipment
- [x] ETB destroy target permanent (VERIFIED 2026-01-18)
- [x] Equipped creature gets +3/+3 (VERIFIED 2026-01-18)
- [x] Equip {3} (VERIFIED 2026-01-18)

**Pillar Launch (x1)** - 1R Instant
- [x] Pump + Untap SubAbility (VERIFIED in Gabriel deck)

**Tectonic Split (x1)** - 4GG Enchantment
- [x] Additional cost: sacrifice half lands (S:Mode$ RaiseCost | Cost$ Sac<X/Land>) **IMPLEMENTED 2026-02-14**
- [x] Hexproof (should work)
- [x] Lands gain triple mana ability (S:Mode$ Continuous | AddAbility$) **IMPLEMENTED 2026-02-19** (ManaEngine integration)

---

## Verified Cards Summary (34/40 fully working)

Working cards:
1. **Island** - basic land
2. **Forest** - basic land
3. **Turtle-Duck** - AB$ Animate
4. **Pillar Launch** - Pump + SubAbility
5. **Forecasting Fortune Teller** - ETB Clue token
6. **Ember Island Production** - SP$ Charm + CopyPermanent
7. **Meteor Sword** - Equipment with ETB destroy, equip, +3/+3 bonus
8. **Knowledge Seeker** - Vigilance + Dies trigger + Second draw trigger (FULLY WORKING 2026-01-19)
9. **Messenger Hawk** - Flying + ETB Clue token + Count$YouDrewThisTurn pump **FULLY WORKING 2026-01-21**
10. **Giant Koi** - Islandcycling + Waterbend "can't be blocked" **FULLY WORKING 2026-02-13**
11. **Elephant-Mandrill** - Reach + ETB Food + Count$Valid combat pump **FULLY WORKING 2026-01-21**
12. **The Unagi** - Flash + Opponent Drawn trigger + Ward:Waterbend **FULLY WORKING 2026-01-21**
13. **Otter-Penguin** - Second draw trigger for +1/+2 pump + "can't be blocked" **FULLY WORKING 2026-01-21**
14. **Gran-Gran** - Taps trigger + ReduceCost static for non-creature spells **FULLY WORKING 2026-02-13**
15. **Avatar Kyoshi** - BeginCombat trigger + Earthbend + Untap + Conditional hexproof **FULLY WORKING 2026-01-21**
16. **Rebellious Captives** - Exhaust ability for counters + earthbend **FULLY WORKING 2026-01-22**
17. **Teo, Spirited Glider** - AttackersDeclared trigger for flying creatures + ImmediateTrigger for counter **FULLY WORKING 2026-01-22**
18. **Allies at Last** - Affinity for Ally + EachDamage power-based damage **FULLY WORKING 2026-02-10**
19. **Abandon Attachments** - UnlessCost$ Discard optional draw 2 **FULLY WORKING 2026-02-19**
20. **Raucous Audience** - Conditional mana ability with Count$Compare **FULLY WORKING 2026-02-19**
21. **Tectonic Split** - RaiseCost sacrifice + AddAbility for land mana tripling **FULLY WORKING 2026-02-19**

## Recent Fixes (2026-02-19)

1. **ManaEngine granted ability integration**: Extended ManaEngine to recognize mana abilities granted by continuous effects (like Chromatic Lantern's "Lands you control have '{T}: Add any color'"). Added `get_effective_mana_production()` helper to merge cached production with granted abilities, `merge_mana_production_kinds()` for OR semantics. Updated `compute_from_scratch()` and `scan_battlefield_fallback()` to use effective production. Tectonic Split's "lands tap for 3 mana" now fully functional.

2. **UnlessCost$ parsing infrastructure**: Added data types and parsing for UnlessCost$ parameters. New types: `UnlessCostType` enum (Mana, Discard, Sacrifice, PayLife, Reveal), `UnlessCost` struct, `Effect::UnlessCostWrapper` variant. Parsing functions: `parse_unless_cost()`, `wrap_with_unless_cost()`, `params_to_effect_with_unless()`. Supports patterns like `UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True`.

3. **UnlessCost$ resolution logic**: Implemented full resolution for UnlessCostWrapper effect in actions/mod.rs:
   - Payer resolution: Resolves "You", "TargetedController" references to concrete PlayerId
   - Cost checking: Verifies player can pay (hand size for discard, life total, controlled permanents for sacrifice)
   - AI heuristics: AI always pays if it can (beneficial for UnlessSwitched=true effects)
   - Payment execution: Discard removes cards from hand, PayLife reduces life total
   - Conditional execution: Inner effect executes based on switched flag (if paid vs if not paid)

4. **Count$Compare**: Implemented conditional count expressions for variable mana production:
   - Added CountExpression::Compare variant with source, condition, true_value, false_value
   - Added CompareCondition enum with GreaterOrEqual, LessOrEqual, Equal, GreaterThan, LessThan
   - Parses patterns like "Count$Compare Y GE1.2.1" (if Y >= 1 then return 2 else return 1)
   - Added amount_var field to Effect::AddMana for variable mana amounts
   - Raucous Audience now fully working (adds {G} or {G}{G} based on creature power)

## Recent Fixes (2026-02-18)

1. **S:Mode$ RaiseCost**: Implemented additional cost static abilities, mirroring the existing ReduceCost pattern. Added `StaticAbility::RaiseCost` variant with `RaisedCost` enum supporting two types:
   - `RaisedCost::Mana(u8)`: Increases generic mana cost (for effects like Thalia, Guardian of Thraben)
   - `RaisedCost::Sacrifice { amount, valid_type }`: Requires sacrificing permanents as additional cost (for Tectonic Split)
   - `RaisedCostAmount::Variable(String)`: Supports SVar X calculation with `Count$Valid Type.YouCtrl/HalfUp` pattern
   - Added `can_pay_sacrifice_costs()` and `pay_sacrifice_costs()` to check/execute sacrifice costs during spell casting
   - Integrated into `calculate_effective_cost()` for mana increases and `push_castable_spells()` for sacrifice castability checks

2. **AddAbility$**: Implemented parsing and infrastructure for granting abilities to permanents. Added `StaticAbility::GrantAbility` variant that stores parsed `ActivatedAbility`. Added `get_granted_abilities()` method to query granted abilities for a permanent. Supports `Affected$ Land.YouCtrl` selector.

## Recent Fixes (2026-02-13)

1. **S:Mode$ ReduceCost**: Implemented cost reduction static abilities. Added `StaticAbility::ReduceCost` variant with `CostReductionTarget` (NonCreature, AllSpells, Creature, Subtype) and `CostReductionCondition` (IsPresent filter, zone, min_count). Enhanced `calculate_effective_cost` in both GameState and GameLoop to query ReduceCost static abilities from controlled permanents. Added `count_cards_matching_filter` helper to check conditions like "3+ Lessons in graveyard". Gran-Gran now fully functional.

2. **Giant Koi Waterbend ability**: Fixed activated ability parsing to use `params_to_effect_with_svars()` instead of `params_to_effect()`. This enables SVar resolution for `StaticAbilities$ Unblockable`, which resolves to `Mode$ CantBlockBy` and returns `Effect::GrantCantBeBlocked`. Giant Koi's Waterbend<3> ability now correctly grants "can't be blocked this turn".

## Recent Fixes (2026-02-10)

1. **K:Affinity:Ally**: Implemented Affinity keyword cost reduction. Reduces generic mana cost by 1 for each permanent of the specified type you control. Added `calculate_effective_cost` methods to both `GameState` and `GameLoop` for consistent cost calculation in both spell affordability checks and actual mana payment. Works for any Affinity type (Ally, Artifact, Spirit, etc.). Allies at Last now fully functional.

## Recent Fixes (2026-01-24)

1. **DB$ EachDamage**: Added Effect::EachDamage variant for multiple creatures dealing damage to a single target. Supports DefinedDamagers$ ParentTarget (parent ability's targets become damagers) and NumDmg$ Count$CardPower (damage equals each damager's power). Used by Allies at Last, Band Together, Tandem Takedown.

## Recent Fixes (2026-01-22)

1. **Exhaust$ True**: Added exhaust field to ActivatedAbility and exhausted_abilities tracking on Card. Once activated, exhaust abilities cannot be activated again.
2. **T:Mode$ AttackersDeclared**: Added batch trigger for "whenever one or more creatures attack". Supports ValidAttackers$ keyword filtering (e.g., Flying). Fires once per declare attackers step.
3. **ImmediateTrigger / RememberDiscarded**: Added remembered_cards field to GameState, ImmediateTriggerCondition enum for conditional execution (RememberedNonLand, AnyRemembered), DB$ Cleanup effect, and DiscardCards remember_discarded parameter. Enables Teo's "when you discard a nonland card this way, put a +1/+1 counter" ability.

## Recent Fixes (2026-01-21)

1. **Otter-Penguin can't be blocked effect**: Fixed GrantCantBeBlocked placeholder resolution in CardDrawn trigger execution - target now correctly resolves to self
2. **Avatar Kyoshi BeginCombat trigger**: Fixed SubAbility$ chain to follow DBUntap after Earthbend effect
3. **Count$Valid**: Added CountExpression enum with ValidPermanents variant, supports Artifact.OppCtrl, Creature.YouCtrl, etc.
4. **Count$YouDrewThisTurn**: CountExpression::CardsDrawnThisTurn reads player.cards_drawn_this_turn
5. **Ward:Waterbend<N>**: Added WardWaterbend keyword variant, parses Ward:Waterbend<4> pattern
6. **Conditional hexproof (Condition$ PlayerTurn)**: Added StaticCondition enum, checks turn ownership when granting keywords

## Remaining Gaps (Complex Features)

1. ~~**S:Mode$ ReduceCost** - Cost reduction static abilities~~ **IMPLEMENTED 2026-02-13**
2. ~~**S:Mode$ RaiseCost** - Additional sacrifice costs~~ **IMPLEMENTED 2026-02-18**
3. ~~**UnlessCost$ / UnlessSwitched$** - Optional cost/discard mechanics~~ **FULLY IMPLEMENTED 2026-02-19**
   - Parsing: ✅ UnlessCostType enum (Mana, Discard, Sacrifice, PayLife, Reveal)
   - Parsing: ✅ Effect::UnlessCostWrapper wraps effects with unless_cost
   - Parsing: ✅ parse_unless_cost() and wrap_with_unless_cost() in effect_converter.rs
   - Resolution: ✅ Payer resolution, cost checking, AI heuristics, payment execution
   - Cards: Abandon Attachments, Academy Loremaster, Aether Barrier
   - Pattern: `UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True`
4. ~~**AddAbility$ for lands** - Grant abilities to land permanents~~ **FULLY IMPLEMENTED 2026-02-19**
   - Parsing: ✅ StaticAbility::GrantAbility with parsed ActivatedAbility
   - Query: ✅ get_granted_abilities() in continuous_effects.rs
   - ManaEngine: ✅ get_effective_mana_production() merges granted abilities with cached production

## Testing Protocol

1. ~~Test "likely to work" cards first with puzzles~~ DONE for 5 cards
2. Identify and implement missing mechanics
3. Run full deck vs deck games
4. Verify web GUI compatibility

## Related Issues
- mtg-5hvly: Gabriel Avatar Deck Compatibility
- mtg-0iad2: Ryan Avatar Deck Compatibility
