---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
  - tracking
created_at: "2025-10-26T21:06:34Z"
updated_at: "2025-10-26T21:06:51Z"
---

# Description

Track completion of heuristic AI port from Java Forge to Rust.

## Current Status

**What's Implemented in HeuristicController:**
- ✅ Creature evaluation (comprehensive, faithful port)
- ✅ Attack decisions with aggression levels (basic - needs improvement)
- ✅ Block decisions with value trading AND life-in-danger logic (2025-10-31)
- ✅ Basic targeting (best creature)
- ✅ Basic spell selection (creatures first)
- ✅ GameStateEvaluator (basic holistic board evaluation)
- ✅ Opponent life access (bd-4 completed)
- ✅ Life-in-danger detection for chump blocking (2025-10-31)

**What's Missing:**

### High Priority (Core AI Strength):

0. **Multi-phase blocking strategy (partial implementation - 2025-11-01)**
   - ✅ Basic gang blocking implemented (2-blocker combinations)
   - ✅ First strike gang blocking logic
   - ✅ Value-based gang block selection
   - Current: Single-pass with gang blocks → single blocks → life-in-danger chump blocks
   - Missing: Java's sophisticated 3-phase strategy (AiBlockController.java:1075-1148)
     - Phase 1: Good blocks → Gang blocks → Trade blocks → Chump blocks → Reinforce
     - Phase 2: If still in danger, reset and reorder: Trade → Good → Chump → Reinforce
     - Phase 3: If serious danger: Chump → Trade → Reinforce → Good
   - Missing: Safe blockers vs killing blockers distinction (for makeGoodBlocks)
   - Missing: 3-blocker gang combinations
   - Missing: Reinforce against trample
   - Missing: Planeswalker defense
   - Missing: Multi-phase danger reassessment and block reordering
   - Impact: Better but still suboptimal blocking compared to Java AI
   - Reference: AiBlockController.java:187-950 (block type methods)

1. **Attack logic improvements (✅ COMPLETED - mtg-84, mtg-85)**
   - ✅ Board state evaluation implemented
   - ✅ Combat math (can_kill_all, can_be_killed, etc.)
   - ✅ Blockability checks
   - ✅ CombatFactors struct mirrors Java's SpellAbilityFactors
   - ✅ All 7 aggression levels faithfully ported
   - Reference: Java's SpellAbilityFactors class in AiAttackController.java:1350-1562

2. **GameStateEvaluator improvements:**
   - mtg-78: Port evalManaBase() - mana base quality scoring
   - mtg-79: Track summon sickness properly (COMPLETED 2025-10-26)
   - mtg-81: Complete land evaluation (detailed heuristics)

3. **Combat outcome prediction**
   - Simulate combat before making decisions
   - Critical for knowing if attacks will be lethal
   - Reference: GameStateEvaluator.java:40-67, 91-100

4. **Activated ability evaluation and timing**
   - Current: Activates any available ability without evaluation
   - Needed: Value assessment, timing optimization, mana efficiency
   - Example: Prodigal Sorcerer should ping opponents when valuable
   - Example: Pump abilities (Shivan Dragon) before combat damage

5. **Mana ability recognition from creatures**
   - Need to recognize Llanowar Elves and similar mana dorks
   - Should use them to cast bigger threats earlier
   - Mana engine needs to see creature mana abilities

### Medium Priority:

6. **Spell evaluation** - Beyond creatures
   - Removal spell targeting (ComputerUtilCard)
   - Card draw value assessment
   - Pump spells, combat tricks

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
- ✅ Opponent life access (bd-4) - GameStateView now provides player_life(), opponents(), and opponent_life() methods
- ✅ Activated ability targeting (mtg-70) - Royal Assassin can now target and destroy tapped creatures
- ✅ **Comprehensive test coverage with 4ED cards (2025-10-26) - 312 tests passing**
- ✅ Life-in-danger blocking logic (2025-10-31):
  - Ported `lifeInDanger()` from ComputerUtilCombat.java:399-466
  - Ported `lifeThatWouldRemain()` from ComputerUtilCombat.java:304-329
  - Ported `lifeInSeriousDanger()` from ComputerUtilCombat.java:477-508
  - Integrated into `should_block()` for chump blocking when life < 5
  - Tournament results: Heuristic 51.4% vs Random 48.6% (improved from 49.5/50.5)
- ✅ Gang blocking implementation (2025-11-01):
  - Implemented basic 2-blocker gang blocking
  - Added `total_damage_of_blockers()` to calculate combined blocker damage
  - Added `can_gang_kill()` to determine if gang can destroy attacker
  - Added `find_gang_block()` to search for optimal gang block combinations
  - Added `assign_blocks_with_gang()` for improved block assignment
  - Prioritizes first strike gang blocks against non-first-strike attackers
  - Uses value-based evaluation to minimize losses while maximizing kills
  - Reference: AiBlockController.makeGangBlocks() lines 368-598

## Test Coverage Expansion (2025-10-26)

Added 4 new e2e tests exercising different AI scenarios:
- Prodigal Sorcerer activated ability usage (pinging)
- Llanowar Elves mana dork recognition and ramping
- Shivan Dragon pump ability and flying attacks
- Juggernaut "must attack" static ability

These tests reveal areas for improvement:
- Activated ability timing and evaluation needs work
- Mana ability recognition from creatures needs implementation
- Pump ability evaluation and usage needs improvement
- Static abilities like "must attack" not yet implemented

