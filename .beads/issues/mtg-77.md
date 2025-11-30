---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-30T17:46:00.244517544+00:00
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
  - Save our creature from dying in combat
  - Kill opposing creatures that would survive
  - Pump unblocked attackers for lethal damage
  - Reduce trample damage by pumping blocker toughness
  - Support both attacking and blocking creatures
- ✅ **Damage assignment order optimization (2025-11-30_#993)**
  - Intelligent blocker ordering to maximize kills
  - Sort by creature evaluation, prioritize killable blockers
  - Port of Java Forge's AiBlockController.orderBlockers()
- ✅ **Enhanced pump activated abilities during combat (2025-11-30_#998)**
  - Firebreathing support (Shivan Dragon's {R}: +1/+0)
  - Evaluate during Declare Blockers step
  - Save creatures, kill blockers, deal lethal with pump abilities
  - Test with real 4ED cards (Shivan Dragon)
- ✅ **Upkeep cost penalties in creature evaluation (2025-11-30_#988)**
  - Cumulative Upkeep: -30 penalty (costs increase each turn)
  - Echo: -10 penalty (must pay again or sacrifice)
  - Fading: -15 to -50 penalty (scaled by fade counters)
  - Vanishing: -15 to -50 penalty (scaled by time counters)
  - Reference: CreatureEvaluator.java:235-276
- ✅ **Intelligent mana tapping order (2025-11-30_#1009)**
  - Port of Java's ComputerUtilMana.scoreManaProducingCard()
  - Score mana sources by alternate uses (lower = tap first)
  - Basic lands: low score (tap first)
  - Mana creatures: +13 for attack potential, +13 for block potential
  - Cards with non-mana abilities: +13 per ability
  - Test with Llanowar Elves vs Forest vs Strip Mine
- ✅ **Counterspell AI (2025-11-30_#1012)**
  - Port of Java's CounterAi.checkApiLogic()
  - Counter opponent spells on the stack (never own spells)
  - Prioritize countering creatures, damage spells, removal, other counters
  - CMC-based filtering (counter CMC 1+ spells)
  - Test with Counterspell vs monored deck
- ✅ **Evasion keyword evaluation (2025-11-30_#1015)**
  - Horsemanship: +power*10 (like flying, only blocked by horsemanship)
  - Shadow: +power*10 (only blocked by creatures with shadow)
  - Reference: CreatureEvaluator.java evasion keyword handling

**What's Missing:**

### High Priority (Core AI Strength):

1. ~~**Combat outcome prediction**~~ ✅ **COMPLETED 2025-11-28_#955**

2. **Activated ability improvements**
   - ✅ Expose game.stack through GameStateView (2025-11-28_#956)
   - ✅ Better ping targeting - choose best KILLABLE creature (2025-11-29_#968)
   - ✅ During-combat pump evaluation (2025-11-29_#975)
   - ✅ Enhanced pump *activated abilities* during combat (2025-11-30_#998)

3. ~~**Creature casting mana efficiency**~~ ✅ **COMPLETED 2025-11-29_#973**

### Medium Priority:

4. ~~**During-combat pump evaluation**~~ ✅ **COMPLETED 2025-11-29_#975**

5. ~~**Damage assignment order**~~ ✅ **COMPLETED 2025-11-30_#993**

6. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring (CLOSED)
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics) (CLOSED)

7. ~~**Mana tapping order**~~ ✅ **COMPLETED 2025-11-30_#1009** - ComputerUtilMana
   - Score-based source selection
   - Preserve mana creatures for combat
   - Preserve utility lands

### Lower Priority:

8. **Bluffing/deception** - Hold information when advantageous
9. mtg-80: Improve enchantment evaluation
10. **Static abilities** - "Must attack", "Can't be blocked by walls", etc.

## Completed Work

- ✅ All items marked with ✅ above
- ✅ **Comprehensive test coverage with real cards (2025-11-30) - 540 tests passing**

## Next Steps (Priority Order)

1. Enchantment evaluation (mtg-80)
2. Static abilities handling
