# Java Heuristic AI Porting Analysis

## Overview

This document outlines the structure and key components of the Java Forge heuristic AI that need to be ported to Rust. The goal is faithful behavioral reproduction - matching the Java AI's decision-making logic as closely as possible.

## Architecture Overview

### Main Components

1. **PlayerControllerAi** (`forge-ai/src/main/java/forge/ai/PlayerControllerAi.java`)
   - ~1,647 lines
   - Entry point for all AI decisions
   - Delegates to `AiController` for core logic
   - Implements `PlayerController` interface with ~50+ methods
   - Key responsibilities:
     - Mulligan decisions
     - Choosing spells to play
     - Combat decisions (delegates to AiAttackController/AiBlockController)
     - Targeting choices
     - Trigger confirmations
     - Card selection for effects

2. **AiController** (`forge-ai/src/main/java/forge/ai/AiController.java`)
   - ~2,700 lines
   - Core AI brain
   - Manages:
     - Spell selection and prioritization
     - Mana management
     - Card memory (tracking revealed cards, etc.)
     - Simulation support (optional)
   - Key fields:
     - `player`: The AI player
     - `game`: Current game state
     - `memory`: AiCardMemory for tracking cards
     - `predictedCombat`: Combat prediction
     - `useSimulation`: Whether to use Monte Carlo simulation

3. **Combat Controllers**
   - `AiAttackController.java` (~1,784 lines)
   - `AiBlockController.java` (~1,379 lines)
   - `ComputerUtilCombat.java` (~2,624 lines)
   - Total: ~5,787 lines of combat logic
   - Responsibilities:
     - Attack target selection
     - Attacker selection with aggression levels
     - Blocker assignment
     - Combat damage assignment
     - Combat trick evaluation

4. **Evaluation Utilities**
   - `CreatureEvaluator.java` (~321 lines)
   - `ComputerUtilCard.java` (~2,500+ lines)
   - `ComputerUtil.java` (~3,600+ lines)
   - Responsibilities:
     - Card value assessment
     - Creature scoring (power/toughness + keywords)
     - Spell evaluation
     - Permanent selection heuristics

5. **Specialized AI Modules**
   - `ComputerUtilMana.java` (~2,000+ lines) - Mana payment and availability
   - `SpellAbilityAi.java` - Spell ability evaluation framework
   - `AiCostDecision.java` - Cost payment decisions
   - `ability/` directory - ~100+ files for specific ability types

## Key Evaluation Functions

### Creature Evaluation (CreatureEvaluator.java)

The `evaluateCreature()` function is foundational. It scores creatures based on:

**Base Value: 80 points**
- +20 if not a token

**Stats Scoring:**
- Power: `power * 15` points
- Toughness: `toughness * 10` points
- CMC: `cmc * 5` points

**Evasion Keywords:**
- Flying/Horsemanship: `+power * 10`
- Unblockable: `+power * 10`
- Fear/Intimidate: `+power * 6`
- Menace: `+power * 4`
- Skulk: `+power * 3`

**Combat Keywords:**
- Double Strike: `+10 + (power * 15)`
- First Strike: `+10 + (power * 5)`
- Deathtouch: `+25`
- Lifelink: `+power * 10`
- Trample: `+(power - 1) * 5` (if power > 1)
- Vigilance: `+(power * 5) + (toughness * 5)`
- Infect: `+power * 15`
- Wither: `+power * 10`

**Defensive Keywords:**
- Indestructible: `+70`
- Hexproof: `+35`
- Shroud: `+30`
- Ward: `+10`
- Protection: `+20`
- Reach (non-flyers): `+5`

**Negative Modifiers:**
- Defender/Can't attack: `-(power * 9 + 40)`
- Can't block: `-10`
- Can't attack or block: Set to base `50 + (cmc * 5)`
- Cumulative Upkeep: `-30`
- Phasing: `-max(20, value / 2)`

**Special Bonuses:**
- Paired: `+14`
- Encoded card: `+24`
- Undying/Persist: `+30`
- Annihilator X: `+X * 50`
- Mana abilities: `+10`
- Activated abilities: `+10` per ability

### Card Selection Utilities (ComputerUtilCard.java)

**Best Creature Selection:**
```java
public static void sortByEvaluateCreature(final CardCollection list) {
    list.sort(EvaluateCreatureComparator);  // Uses evaluateCreature()
}
```

**Mana Cost Evaluation:**
- getBestArtifactAI() - Returns highest CMC artifact
- getBestPlaneswalkerAI() - Returns highest CMC planeswalker
- getMostExpensivePermanentAI() - Returns highest CMC permanent

**Prominent Card Analysis:**
- getMostProminentColor() - Most common color in card list
- getMostProminentCardName() - Most frequent card name

## Decision Logic Flow

### Main Phase Spell Selection (AiController)

1. **Generate Playable Spells:**
   - Enumerate all castable spells from hand
   - Check mana availability
   - Filter by legality (sorcery speed, etc.)

2. **Prioritize Spells:**
   - Lands (if land drop available)
   - Creatures (evaluated by CreatureEvaluator)
   - Removal (targets opponent threats)
   - Pump spells (combat tricks)
   - Card draw
   - Other spells

3. **Target Selection:**
   - For removal: Target best opponent permanent
   - For pump: Target best own creature
   - For card draw: Always play if affordable

4. **Mana Management:**
   - Reserve mana for instant-speed responses
   - Pay costs in optimal order (colored before generic)
   - Consider future turns (don't overextend)

### Combat Logic (AiAttackController)

**Aggression Levels:**
- Ranges from 0 (defensive) to 6 (all-in aggressive)
- Default is 3 (balanced)
- Adjusted based on:
  - Life totals (attack more when ahead)
  - Board state (attack more with advantage)
  - Opponent blockers (attack less when outmatched)

**Attack Selection Algorithm:**
1. Calculate aggression level
2. Evaluate each potential attacker:
   - Can it survive blockers?
   - Does it have evasion?
   - Will it deal meaningful damage?
3. Select attackers based on aggression and safety
4. Choose attack targets (players vs planeswalkers)

**Block Selection Algorithm:**
1. Identify lethal damage
2. Calculate best blocks to minimize damage
3. Consider:
   - Trading up (blocking bigger creature with smaller)
   - Saving key creatures
   - Using expendable tokens
4. Assign blockers to attackers

## Implementation Strategy for Rust

### Phase 1: Core Infrastructure (Weeks 1-2)

1. **Create `HeuristicController` struct:**
   ```rust
   pub struct HeuristicController {
       player_id: PlayerId,
       rng: Box<dyn RngCore>,
       card_memory: CardMemory,
       aggression_level: i32,
   }
   ```

2. **Implement basic `PlayerController` trait methods:**
   - `choose_land_to_play()`
   - `choose_spell_to_cast()` (simple version)
   - Basic pass/priority logic

3. **Port `CreatureEvaluator`:**
   - Create `evaluate_creature(card: &Card) -> i32`
   - Implement keyword scoring
   - Add unit tests comparing to Java values

### Phase 2: Card Evaluation (Weeks 3-4)

1. **Port card selection utilities:**
   - `get_best_creature()`, `get_worst_creature()`
   - `get_best_permanent_to_target()`
   - `evaluate_permanent()`

2. **Implement targeting logic:**
   - Removal spell targeting (destroy best opponent permanent)
   - Pump spell targeting (pump best own creature)
   - Damage spell targeting (kill best opponent creature)

3. **Add hand evaluation:**
   - Mulligan decisions (port `wantMulligan()`)
   - Card priority in hand

### Phase 3: Main Phase Logic (Weeks 5-6)

1. **Spell selection and prioritization:**
   - Land drops (always play if available)
   - Creature casting (in order of evaluation score)
   - Instant/sorcery casting (based on board state)

2. **Mana management:**
   - Calculate available mana
   - Reserve mana for responses
   - Optimal mana tapping order

3. **Ability activation:**
   - Evaluate activated abilities
   - Decide when to activate
   - Target selection for abilities

### Phase 4: Combat AI (Weeks 7-9)

1. **Attack logic:**
   - Port aggression calculation
   - Attacker selection algorithm
   - Attack target selection (player vs planeswalker)

2. **Block logic:**
   - Threat assessment
   - Block assignment algorithm
   - Damage minimization

3. **Combat tricks:**
   - Evaluate instant-speed responses
   - Decide when to use combat tricks
   - Calculate profitable trades

### Phase 5: Testing and Refinement (Weeks 10-12)

1. **Comparative testing:**
   - Play same deck configurations
   - Compare decisions to Java AI
   - Track win rates against random controller

2. **Behavior alignment:**
   - Fix discrepancies with Java AI
   - Ensure faithful reproduction
   - Document intentional deviations

3. **Performance optimization:**
   - Profile hot paths
   - Optimize evaluation functions
   - Maintain readability

## Testing Strategy

### Unit Tests

For each evaluation function:
- Test known cards with expected scores
- Compare Rust scores to Java scores
- Tolerance: ±5 points due to rounding

Example:
```rust
#[test]
fn test_grizzly_bears_evaluation() {
    // Grizzly Bears: 2/2 vanilla creature
    // Expected: 80 (base) + 30 (power) + 20 (toughness) + 10 (cmc) + 20 (non-token) = 160
    let score = evaluate_creature(grizzly_bears);
    assert!((score - 160).abs() <= 5);
}
```

### Integration Tests

Compare AI decisions in identical game states:
1. Set up game state
2. Run Java AI decision
3. Run Rust AI decision
4. Assert they match (or document why they differ)

### E2E Tests

Play complete games:
- AI vs AI (deterministic with seeds)
- Track decision points
- Compare game outcomes
- Measure win rates

## Key Algorithms to Port

### 1. Creature Evaluation (High Priority)
- **File:** `CreatureEvaluator.java:26-284`
- **Complexity:** Medium
- **Dependencies:** Card state, keywords
- **Lines:** ~260

### 2. Best Creature Selection (High Priority)
- **File:** `ComputerUtilCard.java:74-76`
- **Complexity:** Low
- **Dependencies:** CreatureEvaluator
- **Lines:** ~3

### 3. Attack Selection (High Priority)
- **File:** `AiAttackController.java`
- **Complexity:** High
- **Dependencies:** Combat state, creature evaluation
- **Lines:** ~1,784

### 4. Block Assignment (High Priority)
- **File:** `AiBlockController.java`
- **Complexity:** High
- **Dependencies:** Combat state, creature evaluation
- **Lines:** ~1,379

### 5. Spell Selection (Medium Priority)
- **File:** `AiController.java:chooseSpellAbilityToPlay()`
- **Complexity:** High
- **Dependencies:** Mana, card evaluation
- **Lines:** ~100-200

### 6. Mana Management (Medium Priority)
- **File:** `ComputerUtilMana.java`
- **Complexity:** High
- **Dependencies:** Mana pool, costs
- **Lines:** ~2,000+

### 7. Targeting Logic (Medium Priority)
- **File:** Various in `ComputerUtil.java`
- **Complexity:** Medium
- **Dependencies:** Card evaluation
- **Lines:** ~500

### 8. Mulligan Logic (Low Priority - Later)
- **File:** `ComputerUtil.java:wantMulligan()`
- **Complexity:** Medium
- **Dependencies:** Hand evaluation
- **Lines:** ~50

## Reference Game State Snapshots

For testing, we'll use:
1. **Early game:** T3 with 2-3 creatures on board
2. **Mid game:** T7 with developed board
3. **Combat scenarios:** Various attack/block situations
4. **Spell decisions:** When to cast removal vs threats

Each snapshot will have:
- Full game state serialization
- Java AI decision + reasoning
- Expected Rust AI decision
- Tolerance for acceptable variations

## Documentation Requirements

For each ported function, document:
1. **Purpose:** What does it evaluate/decide?
2. **Java source:** File and line number
3. **Algorithm:** Brief description of logic
4. **Scoring:** Point values and reasoning
5. **Edge cases:** Special situations
6. **Tests:** Unit tests covering the function

## Success Criteria

### Phase 1 Success:
- Heuristic AI can play lands
- Heuristic AI can cast creatures
- Passes basic integration tests

### Phase 2 Success:
- Creature evaluation matches Java ±5 points
- Can select best targets for removal
- Passes targeting tests

### Phase 3 Success:
- Makes reasonable spell casting decisions
- Manages mana appropriately
- Plays multiple spells per turn

### Phase 4 Success:
- Attacks when advantageous
- Blocks to minimize damage
- Wins >60% against random controller

### Final Success:
- Behavior closely matches Java AI
- Passes all comparative tests
- Documented deviations are intentional
- Win rate vs random: >70%
- Performance: <100ms per decision

## Current Rust Codebase Status

### Existing AI Controllers:

1. **RandomController** (`src/game/random_controller.rs`)
   - Fully functional
   - Makes random valid decisions
   - Good baseline for testing

2. **PlayerController Trait** (`src/game/controller.rs`)
   - Defines interface for all controllers
   - Key methods to implement:
     - `choose_land_to_play()`
     - `choose_spell_to_cast()`
     - `choose_attackers()`
     - `choose_blockers()`
     - `choose_target()`
     - Many more...

### Game State Access:

- **GameStateView:** Immutable view of game state
- Provides access to:
  - Player zones (hand, battlefield, graveyard)
  - Card information
  - Game phase/turn
  - Combat state
- Clean API for AI to query state

### Next Steps:

1. Create `src/game/heuristic_controller.rs`
2. Implement basic structure
3. Start with creature evaluation
4. Add land play logic
5. Iterate through phases

## Notes

- Focus on faithful reproduction, not improvement
- Document any intentional deviations from Java
- Performance is secondary to correctness
- Extensive testing is critical
- Use Java AI as oracle for expected behavior
