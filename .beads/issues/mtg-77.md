---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-12T02:49:11.452190735+00:00
---

# Description

Track completion of heuristic AI port from Java Forge to Rust.

## Current Status

**What's Implemented in HeuristicController:**
- ✅ Creature evaluation (comprehensive, faithful port)
- ✅ Attack decisions with board state evaluation AND aggression levels (mtg-85 COMPLETED)
- ✅ Multi-phase blocking strategy with gang blocks (3-phase: good/gang/trade/chump) (COMPLETED 2025-11-03)
- ✅ Basic targeting (best creature)
- ✅ **Intelligent creature casting priority** (cast best creature first) (2025-11-03)
- ✅ GameStateEvaluator (basic holistic board evaluation)
- ✅ Opponent life access (bd-4 completed)
- ✅ Life-in-danger detection for blocking (2025-10-31)
- ✅ **Pump spell evaluation with combat trick timing (2025-11-02_#586(4beac0b))**
- ✅ **Removal spell targeting (2025-11-03_#595(f4f9c42))**
- ✅ **Activated ability evaluation and timing (2025-11-03 - mtg-119 COMPLETED)**
- ✅ **Mana ability recognition from creatures** (ALREADY IMPLEMENTED)
- ✅ **Creature casting mana efficiency (2025-11-29_#973)**
- ✅ **During-combat pump evaluation (2025-11-29_#975)**
- ✅ **Damage assignment order optimization (2025-11-30_#993)**
- ✅ **Enhanced pump activated abilities during combat (2025-11-30_#998)**
- ✅ **Upkeep cost penalties in creature evaluation (2025-11-30_#988)**
- ✅ **Intelligent mana tapping order (2025-11-30_#1009)**
- ✅ **Counterspell AI (2025-11-30_#1012)**
- ✅ **Evasion keyword evaluation (2025-11-30_#1015)**
- ✅ **Extended keyword evaluation (2025-12-01_#1063(a3228a1))**
- ✅ **Enchantment evaluation (2025-12-01_#1065 - mtg-80 CLOSED)**
- ✅ **Combat restriction penalties (2025-12-03_#1113(93d67975))**
- ✅ **Blocking restriction evasion abilities (2025-12-03_#1118)**
  - Fear: Only black creatures/artifacts can block
  - Intimidate: Only artifacts/same-color can block
  - Shadow: Only shadow can block shadow (and vice versa)
  - Skulk: Only greater power can block
  - Horsemanship: Only horsemanship can block
  - Protection from color: Can't be blocked by that color
  - Reference: CombatUtil.canBlock() in Java Forge
  - Test: test_blocking_restrictions_evasion() with 6 test scenarios
- ✅ **Deathtouch/Indestructible damage assignment (2025-12-04_#1151(d920ac0))**
  - Deathtouch attackers: 1 damage is lethal per MTG Rules 702.2c
  - Indestructible blockers: Always put last (can't be killed)
  - Tests: test_damage_assignment_with_deathtouch, test_damage_assignment_with_indestructible
- ✅ **Landwalk evasion evaluation (2025-12-04_#1154(0d099e3))**
  - Creatures with Landwalk get power*10 bonus when opponent has matching land
  - can_block_with_view() checks landwalk against defender's lands
  - Reference: CR 702.14 - Landwalk grants unblockability
  - Tests: 5 new tests for swampwalk/islandwalk/forestwalk + Bog Wraith e2e
- ✅ **Enchantment casting from hand (2026-01-03_#1466(5e904d3))**
  - Global enchantments like Crusade, Bad Moon - evaluate creature benefit
  - Aura enchantments like Spirit Link, Holy Strength - check for targets
  - should_cast_global_enchantment() counts benefiting creatures
  - should_cast_aura() checks for beneficial static/triggered abilities
  - creature_matches_selector() helper for AffectedSelector patterns
  - Reference: AttachAi.java, PumpAllAi.java from Java Forge
- ✅ **Destroy activated ability AI (2026-01-03_#1481)**
  - ActivatedAbilityType::Destroy for abilities like Royal Assassin
  - has_valuable_destroy_target() evaluates tapped creatures by value
  - Prioritizes high-power targets, considers keywords (deathtouch, lifelink)
  - Respects indestructible creatures
  - Reference: DestroyAi.java in forge-ai
  - Tests: test_destroy_ability_classification, test_royal_assassin_from_cardsfolder,
    test_has_valuable_destroy_target
- ✅ **SMART multi-blocker damage assignment (2026-01-17_#1715(ecf0d0c))**
  - If lethal for all blockers → auto-assign, no choice needed
  - Otherwise iteratively ask which killable blocker to kill first
  - choose_blocker_for_lethal_damage(): picks most valuable creature (highest eval)
  - choose_blocker_for_remaining_damage(): picks least valuable for leftover damage
  - Accounts for deathtouch (1 damage lethal) and indestructible (can't kill)
  - Uses effective toughness with all buffs applied
  - Reference: New SMART approach, reduces decision tree vs Java Forge
- ✅ **Land drop timing bluffing - Main Phase 2 hold logic (2026-03-12_#1921(a7668bf))**
  - Probabilistic land-holding for Main 2 when mana won't be used
  - 50% chance to hold on turn 3+ if land doesn't enable spell casting
  - Bluffs having instant-speed interaction
  - Reference: AiController.isSafeToHoldLandDropForMain2()
- ✅ **Instant-speed spell timing bluffing (2026-03-12_#1922)**
  - Hold instant-speed non-combat spells (draw, utility) for better timing
  - Prefer casting at opponent's end step (maximizes bluffing)
  - Acceptable fallback: our Main 2
  - Emergency override: hand size 7+ (avoid discard)
  - should_cast_instant_now() implements phase-aware timing decisions
  - Reference: Java Forge phase restriction patterns ("AtEOT", etc.)
  - Test: test_instant_spell_bluffing_timing with 4 scenarios

**What's Missing:**

### Medium Priority:

2. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring (CLOSED)
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics) (CLOSED)

### Lower Priority:

3. **Additional static abilities** - "Can't be blocked except by" patterns (mostly covered via keywords)

## Completed Work

- ✅ All items marked with ✅ above
- ✅ **Comprehensive test coverage with real cards**
- ✅ **Bluffing/deception** - Land drop timing AND instant-speed spell timing

---- ✅ **PutCounterAll spell casting AI (2026-03-14_#1935(d85204a))**
  - should_cast_put_counter_all() evaluates mass counter effects
  - Beneficial (+1/+1): only cast when we benefit more creatures than opponent
  - Curse (-1/-1): only cast when 3+ opponent creatures would be killed
  - Reference: CountersPutAllAi.java:25-115
  - Test: test_should_cast_put_counter_all with 3 scenarios

---- ✅ **ChangeZoneAll spell casting AI (2026-03-14_#1945(e6856211))**
  - should_cast_change_zone_all() evaluates mass zone change effects
  - Battlefield bounce/exile: only cast when opponent loses more creature value
  - Graveyard effects: always beneficial to cast
  - Reference: ChangeZoneAllAi.java:20-200
  - Test: test_should_cast_change_zone_all with 3 scenarios

---- ✅ **Always-beneficial spell casting (2026-03-14_#1956(cbf568e7))**
  - AI now casts SearchLibrary, CreateToken, Scry, CopyPermanent, ExilePermanent, Balance
  - Previously the AI was passing priority holding these spells (e.g., Demonic Tutor)
  - Catch-all for effects that always benefit the caster with no conditions

---- ✅ **Undying/Persist counter-state awareness (2026-03-25_#1974(bfd9503b))**
  - Undying bonus (+25) only applied when creature has NO +1/+1 counters
  - Persist bonus (+20) only applied when creature has NO -1/-1 counters
  - Previously gave flat bonuses regardless, overvaluing already-triggered creatures
  - Reference: Java ComputerUtilCard.java:1872-1883 (hasActiveUndyingOrPersist)

---- ✅ **Surveil/Loot/Dig always-beneficial + SacrificeAll board wipe AI (2026-03-25_#1976(de75f815))**
  - Added Surveil, Loot, Dig to always-beneficial spell casting list
  - Added SacrificeAll routing through board wipe evaluator (alongside DestroyAll/DamageAll)
  - Previously AI wouldn't cast Thought Erasure (Surveil) or All is Dust (SacrificeAll)

---- ✅ **Expanded always-beneficial spell list (2026-03-25_#1980(6a81181c))**
  - Added Mill, GainLife, PumpAllCreatures, MultiplyCounter, PutCounter
  - AI now casts 18+ effect types automatically when affordable
  - Previously passed on Mind Sculpt (Mill), Overrun (PumpAll), etc.

---- ✅ **Counter board wipes, extra turns, and steal effects (2026-03-26_#1994(9a98d0d7))**
  - Expanded should_counter_spell() to prioritize high-value targets:
    DestroyAll (Wrath), SacrificeAll (All is Dust), DamageAll (Pyroclasm),
    ChangeZoneAll (Aetherize), AddTurn (Time Walk), GainControl (Control Magic)
  - Previously only countered creatures, damage, removal, counters, pump
  - Reference: Java CounterAi.java:151-182 (configurable counter preferences)

## Next Steps (Priority Order)

1. More static abilities handling (if needed beyond current keyword coverage)
2. Additional effect AI evaluations (Play, ChooseCard, etc.)
3. Conditional casting improvements (when NOT to cast beneficial spells)

**Checked up-to-date as of 2026-03-26_#1994(9a98d0d7) - 942 tests passing**
