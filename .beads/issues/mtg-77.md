---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-07T23:23:00.724008349+00:00
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

**What's Missing:**

### Medium Priority:

2. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring (CLOSED)
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics) (CLOSED)

### Lower Priority:

3. **Bluffing/deception** - Hold information when advantageous
4. **Additional static abilities** - "Can't be blocked except by" patterns

## Completed Work

- ✅ All items marked with ✅ above
- ✅ **Comprehensive test coverage with real cards**

## Next Steps (Priority Order)

1. More static abilities handling ("can't be blocked except by" types)
2. Bluffing/deception

---- ✅ **Activated/triggered ability scoring in creature evaluation (2026-03-07_#1867(b876f88))**
  - Activated abilities: classify as Ping/Pump/Destroy/Mana, score accordingly
  - Triggered abilities: score by event type (ETB +10, combat damage +15, etc.)
  - 10 new tests with real 4ED cards

---- ✅ **Regeneration ability support (AB$ Regenerate) (2026-03-07_#1868)**
  - Full-stack: parsing, Effect::Regenerate, regeneration shields, combat damage interception
  - AI activates regeneration during combat phases; creature evaluation: +20
  - 7 new tests (4 mechanical + 3 creature evaluation with Drudge Skeletons, Sedge Troll)
  - Affects 246 cards. Also implemented end-of-turn damage removal for all creatures.

---- ✅ **Board wipe / mass effect AI evaluation (2026-03-07_#1874)**
  - Board wipe (DestroyAll/DamageAll): compares creature values with 200pt threshold, low-life override
  - ForceSacrifice: cast when opponent has creatures
  - TapAll: cast when opponent has 2+ untapped creatures
  - UntapAll: cast when we have 2+ tapped creatures
  - SetLife: cast when target amount > current life
  - LoseLife: always cast (targets opponent)
  - 11 new unit tests for AI decision methods
  - Modeled after Java DestroyAllAi.doMassRemovalLogic()

---
**Checked up-to-date as of 2026-03-12_#1920(aa94be5) - 938 tests passing**

---- ✅ **Fight spell AI (2026-03-10_#1900)**
  - should_cast_fight() evaluates Fight spells (AB$ Fight)
  - Favorable matchup detection: our creature kills theirs AND survives
  - Deathtouch handling: 1 damage is lethal when attacking
  - Indestructible handling: skip unkillable targets
  - Trade-up logic: accept mutual kills if their creature worth 50+ more
  - Reference: FightAi.java:27-108 (checkApiLogic)
  - 4 new tests (favorable, unfavorable, deathtouch trade-up, no creatures)

---- ✅ **GainControl spell AI (2026-03-10_#1900)**
  - should_cast_gain_control() evaluates steal effects (AB$ GainControl)
  - Always cast if opponent has creatures (2-for-1 value)
  - Reference: ControlGainAi.java
  - 3 new tests (valuable target, no targets, always steals)

---- ✅ **Removal timing AI - use_removal_now() (2026-03-07_#1877(975720b))**
  - Phase-aware timing: hold instant removal for combat/end step/Main phases
  - Sorcery removal always fires immediately (limited windows)
  - Two-for-one detection: enchanted targets trigger immediate removal
  - High-value threshold (eval >= 200): remove dangerous creatures immediately
  - Integrated into should_cast_spell() for destroy/damage effects
  - target_has_auras() helper for aura attachment detection
  - Reference: ComputerUtilCard.useRemovalNow() lines 1062-1278 in Java Forge
  - 9 new tests with real 4ED cards (Terror, Lightning Bolt, Swords to Plowshares,
    Serra Angel, Shivan Dragon, Grizzly Bears, Holy Strength)
