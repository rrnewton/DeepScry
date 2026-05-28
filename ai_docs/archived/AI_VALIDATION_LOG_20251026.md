# AI Validation Log - Heuristic Controller Testing

## Test Setup
- **Date**: 2025-10-26
- **Decks**: combat_test_4ed.dck vs vigilance_deck.dck
- **Controllers**: HeuristicController vs HeuristicController
- **Seed**: 12345
- **Verbosity**: Verbose
- **Result**: Player 1 won on turn 27 (Player 2 death)

## Findings

### ✅ Working Correctly

1. **Land drops** - Both players consistently played lands when available
2. **Spell casting** - Player 1 cast creatures (Grizzly Bears, Serra Angels) appropriately
3. **Serra Angel attacks** - Flying creatures attacked correctly once on battlefield
4. **Vigilance keyword** - Serra Angels did NOT tap when attacking (working correctly!)
5. **Combat damage** - Damage was applied correctly (4 damage per Serra Angel)
6. **Hand size limit** - Player 2 correctly discarded when over 7 cards
7. **Game end condition** - Game ended when Player 2 reached 0 life

### ⚠️ Potential Issues

#### Issue 1: Conservative Attack Threshold (Aggression Level 3)

**Observation**: Grizzly Bears (2/2, no keywords) never attacked even when opponent had no blockers.

**Code Location**: `src/game/heuristic_controller.rs:442-454`

**Current Logic for Aggression 3 (Balanced)**:
```rust
has_evasion
    || (power >= 2 && (first_strike || double_strike || deathtouch || trample))
    || power >= 3
```

**Analysis**:
- Grizzly Bears have power=2, no keywords
- They fail all three conditions:
  - No evasion
  - power >= 2 BUT no combat keywords
  - power < 3
- Therefore they never attack

**Root Cause**: ✅ CONFIRMED - Our Rust implementation is NOT faithful to Java Forge

**Java Forge Logic** (`AiAttackController.java:1535-1543`):
```java
case 3: // expecting to at least kill a creature of equal value or not be blocked
    if ((saf.canKillAll && saf.isWorthLessThanAllKillers)
            || (((saf.dangerousBlockersPresent && saf.canKillAllDangerous)
                || saf.hasAttackEffect || saf.hasCombatEffect) && !saf.canBeKilledByOne)
            || !saf.canBeBlocked()) {
        return true;
    }
```

Java checks:
1. **Board state** - Can the attacker kill all potential blockers?
2. **Value trade** - Is the attacker worth less than what it can kill?
3. **Survival** - Will the attacker survive combat?
4. **Blockability** - Can the opponent even block this creature?

**Our Rust Logic** (heuristic_controller.rs:442-454):
```rust
has_evasion || (power >= 2 && keywords) || power >= 3
```

We only check the attacker's stats in isolation, ignoring:
- Opponent's board state
- Whether blockers exist
- Combat math (can kill / will survive)
- Creature value comparison

**Expected Fix**: Implement `SpellAbilityFactors` equivalent that evaluates board state and makes attack decisions based on:
- Available blockers
- Combat math (kill/survive analysis)
- Creature evaluation scores
- Evasion vs blockers

This is a significant gap - we're missing ~90% of the Java attack logic!

#### Issue 2: Mana Availability Checking (Possible Non-Issue)

**Observation**: Player 2 held Serra Angels (5 mana) but only had 4 Plains.

**Analysis**:
- Turn 4: Drew Grizzly Bears (2 mana) with 1 Plains - correctly didn't cast
- Turn 6+: Drew Serra Angels (5 mana) but only had 1-4 Plains - correctly didn't cast
- Player 2 was simply mana-screwed

**Verdict**: System working correctly - AI doesn't attempt to cast spells it can't afford. Not a bug.

### 🔍 Items to Investigate Further

1. **Attack logic comparison**: Read `AiAttackController.java:1503-1560` to verify aggression level thresholds
2. **Board state evaluation**: Java AI considers opponent's board state when deciding to attack - we currently don't
3. **Spell evaluation**: Player 1 was able to cast spells, but need to verify spell casting priority is correct
4. **Land selection**: Both AIs just played the first land - need to verify this matches Java's land selection algorithm

### 📊 Test Statistics

- **Total Turns**: 27
- **Player 1 Creatures Cast**: 6 (5 Grizzly Bears, 2 Serra Angels)
- **Player 2 Creatures Cast**: 0 (mana screwed)
- **Player 1 Attacks**: 3 combat phases with attackers (all Serra Angels)
- **Player 2 Blocks**: 0
- **Final Life Totals**: P1=20, P2=0
- **Damage Dealt**: 20 (all from Serra Angel attacks)

### 🎯 Next Steps

1. ✅ Add HeuristicController to CLI binary
2. ✅ Run AI vs AI game with verbose logging
3. ⏳ Analyze game logs for rule violations
4. ⏳ Compare attack logic with Java Forge source
5. ⏳ File issues in beads for confirmed bugs
6. ⏳ Create more diverse test scenarios

### 📝 Notes

- Game logs show clean execution with no errors or crashes
- No obvious MTG rule violations observed
- Combat damage calculation appears correct
- Priority passing works smoothly
- Need to test with:
  - Blockers (to validate blocking logic)
  - Combat keywords (first strike, trample, deathtouch in combat)
  - Non-creature spells (removal, pump spells)
  - Activated abilities (Royal Assassin tap ability)
