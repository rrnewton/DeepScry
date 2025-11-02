---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
  - tracking
created_at: "2025-10-26T21:06:34Z"
updated_at: "2025-11-02T14:25:00Z"
---

# Description

Track completion of heuristic AI port from Java Forge to Rust.

## Implementation Status Summary

**Java AI Total**: ~25,000 lines across 25+ classes  
**Rust AI Total**: ~2,000 lines (heuristic_controller.rs)  
**Overall Completeness**: ~60% (core combat 100%, spell/mana 10%)

### ✅ Fully Implemented (100%)
1. **Attack logic** - All aggression levels, combat factors, lethal detection
2. **Blocking logic** - All phases, gang blocks, reinforcement, danger assessment  
3. **Creature evaluation** - Line-by-line faithful port
4. **Combat math** - Kill calculations, damage totals, life danger detection

### ⚠️ Partially Implemented (10-30%)
5. **Spell selection** - Basic creatures-first heuristic only
6. **Targeting** - Simple best-creature targeting only
7. **Activated abilities** - Basic implementation, no evaluation

### ❌ Not Implemented (0%)
8. **Spell evaluation** (ComputerUtilCard, 2100+ lines) - Removal, card draw, combat tricks
9. **Mana optimization** (ComputerUtilMana, 1800+ lines) - Color availability, tap order
10. **Cost decisions** (ComputerUtilCost, 700+ lines) - Sacrifice, discard, life payment
11. **Special card AI** (SpecialCardAi, 2000+ lines) - Card-specific logic
12. **Play decisions** (SpellAbilityAi, 500+ lines) - When to cast spells

**Performance**: 66% win rate vs Random (competitive with Java)

---

## What's Missing (Detailed)

### High Priority (Major Gameplay Impact)

1. **Spell Evaluation System** (ComputerUtilCard - 2126 lines)
   - Removal spell targeting and timing
   - Card draw value assessment  
   - Pump spells and combat tricks
   - Conditional spell casting (don't waste removal on tokens)
   - Impact: Critical for non-creature spells to be effective
   - Java Reference: ComputerUtilCard.java

2. **Mana Management** (ComputerUtilMana - 1807 lines)
   - Optimal mana tapping order
   - Color availability for instant responses
   - Dual land / painland optimization
   - Mana dork recognition (Llanowar Elves, etc.)
   - Impact: Affects spell castability and efficiency
   - Java Reference: ComputerUtilMana.java

3. **Cost Decision System** (ComputerUtilCost - 724 lines)
   - Sacrifice decisions (which creatures to sacrifice)
   - Discard decisions (which cards to discard)
   - Life payment decisions
   - Impact: Required for many spells and abilities
   - Java Reference: ComputerUtilCost.java

### Medium Priority (Refinements)

4. **Activated Ability Evaluation** (ComputerUtilAbility - 431 lines)
   - When to activate abilities
   - Mana efficiency for activations
   - Timing optimization (e.g., Prodigal Sorcerer pinging)
   - Currently: Activates without evaluation
   - Java Reference: ComputerUtilAbility.java

5. **Game State Improvements**
   - Mana base evaluation (evalManaBase)
   - Combat outcome prediction/simulation
   - Better land evaluation heuristics
   - Java Reference: GameStateEvaluator.java

### Low Priority (Edge Cases)

6. **Special Card Logic** (SpecialCardAi - 1983 lines)
   - Card-specific AI for complex cards
   - Not needed for basic gameplay
   - Java Reference: SpecialCardAi.java

7. **Minor Features**
   - Damage assignment order optimization
   - Planeswalker defense (when planeswalkers added)
   - Static ability handling ("must attack", etc.)
   - Bluffing/deception strategies

## Completed Work (2025-10-26 to 2025-11-02)

### ✅ Combat System (100% Complete)
**Attack Logic**: All aggression levels (0-6), combat factors, lethal detection (+4.9% win rate)  
**Blocking Logic**: Good blocks, gang blocks (2 and 3-blocker), trade blocks, multi-phase danger reassessment, reinforcement (trample + kill)  
**Creature Evaluation**: Line-by-line faithful port from Java CreatureEvaluator  
**Combat Math**: Can destroy, total damage, life danger detection (regular + serious)

**Performance**: Final win rate 66.1% vs Random 33.9% (+3.2% from session start)  
**References**: AiAttackController.java (1784 lines), AiBlockController.java (1379 lines), CreatureEvaluator.java (321 lines)

### ⚠️ Basic Implementation
**Spell Selection**: Creatures-first heuristic only  
**Targeting**: Simple best-creature targeting  
**Activated Abilities**: Basic implementation without evaluation  
**Game State**: Basic board scoring, no mana base evaluation

See AI_COMPARISON.md for detailed feature-by-feature comparison.

