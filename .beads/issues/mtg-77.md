---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-03T01:31:14.532848524+00:00
---

# Description

Track completion of heuristic AI port from Java Forge to Rust.

## Current Status

**What's Implemented in HeuristicController:**
- ✅ Creature evaluation (comprehensive, faithful port)
- ✅ Attack decisions with board state evaluation AND aggression levels (mtg-85 COMPLETED)
- ✅ Multi-phase blocking strategy with gang blocks (3-phase: good/gang/trade/chump) (COMPLETED 2025-11-03)
- ✅ Basic targeting (best creature)
- ✅ Basic spell selection (creatures first)
- ✅ GameStateEvaluator (basic holistic board evaluation)
- ✅ Opponent life access (bd-4 completed)
- ✅ Life-in-danger detection for blocking (2025-10-31)
- ✅ **Pump spell evaluation with combat trick timing (2025-11-02_#586(4beac0b))**
  - Pre-combat evaluation: Makes non-attackers into attackers
  - Haste granting evaluation
  - Evasion granting evaluation (Flying, etc.)
  - **Combat trick detection and timing** (NEW)
  - Phase-based pump spell restrictions (instant pumps held for combat)
  - Reference: ComputerUtilCard.shouldPumpCard() lines 1291-1466
  - Reference: PumpAi.checkPhaseRestrictions() lines 98-103

**What's Missing:**

### High Priority (Core AI Strength):

1. **Removal spell targeting logic (NEXT PRIORITY)**
   - Current: `should_cast_spell()` returns false for removal to prevent targeting bugs
   - Missing: ComputerUtilCard removal evaluation logic
   - Missing: Intelligent targeting (best threat, can't regenerate, etc.)
   - Reference: ComputerUtilCard.java removal spell methods
   - Impact: AI doesn't use Terror, Swords to Plowshares, Lightning Bolt on creatures

2. **Activated ability evaluation and timing**
   - Current: `should_activate_ability()` returns false to prevent infinite loops
   - Needed: Value assessment, timing optimization, mana efficiency
   - Example: Prodigal Sorcerer should ping opponents when valuable
   - Example: Pump abilities (Shivan Dragon) before combat damage

3. **Mana ability recognition from creatures**
   - Need to recognize Llanowar Elves and similar mana dorks
   - Should use them to cast bigger threats earlier
   - Mana engine needs to see creature mana abilities

### Medium Priority:

4. **During-combat pump evaluation (BLOCKED: needs combat state tracking)**
   - Requires: GameStateView to expose attacking/blocking creatures
   - Requires: Combat state (which creatures are attacking/blocking which)
   - Once available, implement ComputerUtilCard.java:1468-1600 logic

5. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics)

6. **Combat outcome prediction**
   - Simulate combat before making decisions
   - Critical for knowing if attacks will be lethal
   - Reference: GameStateEvaluator.java:40-67, 91-100

7. **Mana tapping order** - ComputerUtilMana
   - Leave up correct colors for instant responses
   - Optimize painland/fetchland usage

### Lower Priority:

8. **Damage assignment order** - Kill blockers efficiently
9. **Bluffing/deception** - Hold information when advantageous
10. mtg-80: Improve enchantment evaluation
11. **Static abilities** - "Must attack", "Can't be blocked by walls", etc.

## Completed Work

- ✅ Basic GameStateEvaluator with hand, life, and battlefield evaluation
- ✅ Creature evaluation (faithful port from Java)
- ✅ Basic land evaluation
- ✅ Score type with summon sickness tracking (mtg-79 completed 2025-10-26)
- ✅ Opponent life access (bd-4)
- ✅ Activated ability targeting (mtg-70)
- ✅ **Comprehensive test coverage with 4ED cards (2025-10-26) - 274 tests passing**
- ✅ Life-in-danger blocking logic (2025-10-31)
- ✅ **Pump spell evaluation (2025-11-02_#556(889631a))**
- ✅ **Combat trick timing logic (2025-11-02_#586(4beac0b))**
- ✅ **Attack logic with board state evaluation (mtg-85 COMPLETED)**
  - SpellAbilityFactors equivalent with combat math
  - Aggression level implementation (6 levels)
  - Blockability checks, value comparison
  - Reference: AiAttackController.java:1350-1562
- ✅ **Multi-phase blocking strategy (COMPLETED 2025-11-03)**
  - Phase 1: Good -> Gang -> Trade -> Chump -> Reinforce
  - Phase 2: If danger, reset: Trade -> Good -> Chump -> Reinforce
  - Phase 3: If serious danger: Chump -> Trade -> Reinforce -> Good
  - Gang blocking with combat math
  - Reinforce against trample
  - Reference: AiBlockController.java:1075-1148, 187-950

## Next Steps (Priority Order)

1. **Removal spell targeting logic** (CURRENT PRIORITY)
   - Port ComputerUtilCard removal evaluation
   - Implement intelligent targeting for Terror, Doom Blade, Lightning Bolt, etc.
2. Activated ability evaluation and timing
3. Mana ability recognition from creatures
4. During-combat pump evaluation (BLOCKED)
