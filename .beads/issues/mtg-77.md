---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-02T20:36:01.505774725+00:00
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

0. **Multi-phase blocking strategy (incomplete)**
   - Current: Simple single-pass blocking with life-in-danger chump blocks
   - Missing: Java's sophisticated 3-phase strategy (AiBlockController.java:1075-1148)
     - Phase 1: Good blocks → Gang blocks → Trade blocks → Chump blocks → Reinforce
     - Phase 2: If still in danger, reset and reorder: Trade → Good → Chump → Reinforce
     - Phase 3: If serious danger: Chump → Trade → Reinforce → Good
   - Missing: Safe blockers vs killing blockers distinction
   - Missing: Gang blocking (multi-blocker combat math)
   - Missing: Reinforce against trample
   - Missing: Planeswalker defense
   - Impact: Suboptimal blocking, doesn't maximize damage prevention
   - Reference: AiBlockController.java:187-950 (block type methods)

1. **Attack logic improvements (mtg-85)**
   - Current: Only evaluates attacker stats in isolation
   - Missing: Board state evaluation, combat math, blockability checks
   - Reference: Java's SpellAbilityFactors class in AiAttackController.java:1350-1562
   - Impact: 2/2 vanilla creatures never attack even with no blockers
   - Impact: Shivan Dragon (5/5 flyer) doesn't attack Grizzly Bears (2/2 ground)

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

6. **Spell evaluation - PARTIALLY COMPLETE**
   - ✅ Pump spell evaluation (pre-combat, 2025-11-02)
   - ✅ Combat trick timing (phase restrictions implemented, 2025-11-02_#586)
   - ⏳ During-combat pump evaluation (BLOCKED: needs combat state tracking in GameStateView)
   - ❌ Removal spell targeting (ComputerUtilCard)
   - ❌ Card draw value assessment
   - Reference: ComputerUtilCard.shouldPumpCard() lines 1291-1600+
   - Reference: PumpAi.checkPhaseRestrictions() lines 98-103

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
- ✅ **Comprehensive test coverage with 4ED cards (2025-10-26) - 274 tests passing**
- ✅ Life-in-danger blocking logic (2025-10-31):
  - Ported `lifeInDanger()` from ComputerUtilCombat.java:399-466
  - Ported `lifeThatWouldRemain()` from ComputerUtilCombat.java:304-329
  - Ported `lifeInSeriousDanger()` from ComputerUtilCombat.java:477-508
  - Integrated into `should_block()` for chump blocking when life < 5
  - Tournament results: Heuristic 51.4% vs Random 48.6% (improved from 49.5/50.5)
- ✅ **Pump spell evaluation (2025-11-02_#556(889631a))**:
  - Ported `shouldPumpCard()` pre-combat logic from ComputerUtilCard.java:1291-1466
  - Evaluates making non-attackers into attackers
  - Evaluates haste granting for summoning-sick creatures
  - Evaluates evasion granting (Flying, Unblockable, etc.)
  - Integrated into `choose_best_spell()` spell selection priority
  - Added `should_cast_pump()` method to HeuristicController
  - Added `can_block_simple()` helper for evasion evaluation
- ✅ **Combat trick timing logic (2025-11-02_#586(4beac0b))**:
  - Ported phase restriction logic from PumpAi.checkPhaseRestrictions() (lines 98-103)
  - Instant-speed pumps now held for combat instead of cast pre-combat
  - Combat trick detection: pure buffs (or with Trample/FirstStrike/DoubleStrike only) are held
  - Pre-combat exception: cast if makes non-attacker attack AND threat > 30%
  - Phase-based evaluation in `should_cast_pump()`:
    - Main1: Evaluate for making attackers, hold combat tricks
    - DeclareBlockers: Placeholder for during-combat evaluation (needs combat state)
    - Other phases: Don't cast pumps
  - Added `would_attack_if_pumped()` helper method
  - Reference: ComputerUtilCard.java:1416-1431 (combat trick detection)
  - Reference: PumpAi.java:98-103 (phase restrictions)

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

## Next Steps (Priority Order)

1. **During-combat pump evaluation** (BLOCKED: needs combat state tracking)
   - Requires: GameStateView to expose attacking/blocking creatures
   - Requires: Combat state (which creatures are attacking/blocking which)
   - Once available, implement ComputerUtilCard.java:1468-1600 logic
2. Removal spell targeting logic
3. Attack logic improvements (mtg-85)
4. Multi-phase blocking strategy
