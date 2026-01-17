---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-01-03T04:50:07.114089917+00:00
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

---
**Checked up-to-date as of 2026-01-17_#1715 - 513 unit tests passing**
