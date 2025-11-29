---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-29T22:22:03.264796994+00:00
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

**What's Missing:**

### High Priority (Core AI Strength):

1. ~~**Combat outcome prediction**~~ ✅ **COMPLETED 2025-11-28_#955**

2. **Activated ability improvements**
   - ✅ Expose game.stack through GameStateView (2025-11-28_#956)
   - ✅ Better ping targeting - choose best KILLABLE creature (2025-11-29_#968)
   - ✅ During-combat pump evaluation (2025-11-29_#975)
   - Enhanced pump *activated abilities* during combat (pending)

3. ~~**Creature casting mana efficiency**~~ ✅ **COMPLETED 2025-11-29_#973**

### Medium Priority:

4. ~~**During-combat pump evaluation**~~ ✅ **COMPLETED 2025-11-29_#975**

5. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics)

6. **Mana tapping order** - ComputerUtilMana
   - Leave up correct colors for instant responses
   - Optimize painland/fetchland usage

### Lower Priority:

7. **Damage assignment order** - Kill blockers efficiently
8. **Bluffing/deception** - Hold information when advantageous
9. mtg-80: Improve enchantment evaluation
10. **Static abilities** - "Must attack", "Can't be blocked by walls", etc.

## Completed Work

- ✅ All items marked with ✅ above
- ✅ **Comprehensive test coverage with 4ED cards (2025-10-26) - 508 tests passing**

## Next Steps (Priority Order)

1. GameStateEvaluator improvements
2. Mana tapping order
3. Damage assignment order
