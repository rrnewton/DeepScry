---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-03T17:29:42.378471323+00:00
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
  - Destroy and damage-based removal
  - Intelligent target selection (best opponent creature)
  - Filters indestructible and dying creatures
  - Damage amount validation for burn spells
  - Reference: DestroyAi.java:152-247
- ✅ **Activated ability evaluation and timing (2025-11-03 - mtg-119 COMPLETED)**
  - Ping abilities (Prodigal Sorcerer pattern)
  - Pump abilities (Shivan Dragon pattern)
  - Timing logic for end of turn and Main2 pings
  - Main1 timing for pump abilities
  - Skips mana abilities correctly
  - Reference: DamageDealAi.java:196-200, 682-703; PumpAi.java
- ✅ **Mana ability recognition from creatures** (ALREADY IMPLEMENTED)
  - ManaEngine detects Llanowar Elves and similar mana dorks
  - Handles summoning sickness correctly
  - Used by game engine to filter castable spells
  - Reference: mana_engine.rs:327-338, 405-457

**What's Missing:**

### High Priority (Core AI Strength):

1. **Combat outcome prediction (NEXT PRIORITY)**
   - Simulate combat before making decisions
   - Critical for knowing if attacks will be lethal
   - Reference: GameStateEvaluator.java:40-67, 91-100

2. **Activated ability improvements**
   - Expose game.stack through GameStateView for proper stack-empty checks
   - Better ping targeting (choose "best" killable creature, not just any)
   - Enhanced pump evaluation with combat simulation

3. **Creature casting mana efficiency**
   - Currently casts highest-value creature, but should consider mana efficiency
   - Cast cheap threats early, save mana for interaction
   - Reference: ComputerUtil.java creature casting logic

### Medium Priority:

4. **During-combat pump evaluation (BLOCKED: needs combat state tracking)**
   - Requires: GameStateView to expose attacking/blocking creatures
   - Requires: Combat state (which creatures are attacking/blocking which)
   - Once available, implement ComputerUtilCard.java:1468-1600 logic

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

- ✅ Basic GameStateEvaluator with hand, life, and battlefield evaluation
- ✅ Creature evaluation (faithful port from Java)
- ✅ Basic land evaluation
- ✅ Score type with summon sickness tracking (mtg-79 completed 2025-10-26)
- ✅ Opponent life access (bd-4)
- ✅ Activated ability targeting (mtg-70)
- ✅ **Comprehensive test coverage with 4ED cards (2025-10-26) - 404 tests passing**
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
- ✅ **Removal spell targeting (2025-11-03_#595(f4f9c42))**
  - Detects DestroyPermanent and DealDamage effects
  - Filters indestructible creatures
  - Filters dying creatures (toughness <= 0)
  - Validates damage amount for burn spells
  - Targets best (highest-value) opponent creature
  - Reference: DestroyAi.java:152-247, ComputerUtilCard.getBestCreatureAI
- ✅ **Activated ability evaluation (2025-11-03 - mtg-119)**
  - Ping ability timing and targeting
  - Pump ability evaluation for combat
  - Mana ability skipping
  - Reference: DamageDealAi.java, PumpAi.java
- ✅ **Intelligent creature casting priority (2025-11-03)**
  - Evaluates all castable creatures using comprehensive heuristic
  - Casts highest-value creature rather than first creature found
  - Considers power, toughness, keywords, evasion, and combat abilities
  - Reference: Choose best spell logic in HeuristicController

## Next Steps (Priority Order)

1. **Combat outcome prediction** (CURRENT PRIORITY)
   - Simulate combat before attack/block decisions
   - Determine if we can win through combat
2. Improve activated ability stack-empty checks
3. During-combat pump evaluation (BLOCKED)
4. Creature casting mana efficiency
