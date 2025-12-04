---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-04T14:41:47.636249333+00:00
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

**What's Missing:**

### Medium Priority:

1. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring (CLOSED)
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics) (CLOSED)

### Lower Priority:

2. **Bluffing/deception** - Hold information when advantageous
3. **Additional static abilities** - "Can't be blocked except by" patterns

## Completed Work

- ✅ All items marked with ✅ above
- ✅ **Comprehensive test coverage with real cards**

## Next Steps (Priority Order)

1. More static abilities handling ("can't be blocked except by" types)
2. Bluffing/deception

---
**Checked up-to-date as of 2025-12-04_#1134(28100f8) - 597 tests passing**
