# PZL (Puzzle) File Format Analysis

## Overview

PZL files are used in Forge (Java) to represent specific game states for puzzles and testing scenarios. They allow creating mid-game situations without playing through the entire game up to that point.

## File Format

PZL files use an INI-like format with two main sections:

### 1. `[metadata]` Section

Defines puzzle metadata and goals:

```
[metadata]
Name:Pauper Puzzles #04 - Make Love, Not War
URL:https://pauperpuzzles.wordpress.com/2017/01/20/4-make-love-not-war/
Goal:Win
Turns:1
Difficulty:Hard
Description:Win this turn. (optional)
Targets:Creature.OppCtrl (optional, for specific goal types)
TargetCount:1 (optional, default 1)
HumanControl:false (optional, default false)
```

**Metadata Fields:**
- `Name`: Puzzle title
- `URL`: Reference URL for the puzzle
- `Goal`: Win condition type (see Goal Types below)
- `Turns`: Number of turns allowed to complete the puzzle
- `Difficulty`: Easy/Medium/Hard
- `Description`: Optional detailed description
- `Targets`: Optional filter for goal-specific targets (e.g., "Creature.OppCtrl")
- `TargetCount`: Number of targets required (default 1)
- `HumanControl`: If true, human controls both players (for testing complex scenarios)

**Goal Types:**
- `Win` - Win the game within turn limit (default)
- `Survive` - Survive until turn limit + 1
- `Destroy specified permanents` / `Destroy specified creatures` / `Kill specified creatures`
- `Remove specified permanents from the battlefield`
- `Put the specified permanent on the battlefield` / `Play the specified permanent`
- `Gain control of specified permanents`
- `Win before opponent's next turn`

### 2. `[state]` Section

Defines the complete game state:

#### Format Variations

**Modern Format (p0/p1 prefix):**
```
turn=20
activeplayer=p0
activephase=UPKEEP
p0life=6
p0landsplayed=0
p0landsplayedlastturn=0
p0hand=Card1;Card2
p0battlefield=Card3;Card4
p1life=6
p1battlefield=Card5
```

**Legacy Format (Human/AI prefix):**
```
ActivePlayer=Human
ActivePhase=Main1
HumanLife=1
AILife=5
humanhand=Card1;Card2
humanbattlefield=Card3;Card4
aibattlefield=Card5
```

#### State Fields

**Game-Level:**
- `turn=N` - Current turn number
- `activeplayer=p0|p1|human|ai` - Active player
- `activephase=PHASE` - Current phase (UPKEEP, MAIN1, COMBAT_DECLARE_ATTACKERS, etc.)
- `activephaseadvance=PHASE` - Auto-advance to this phase after setup
- `removesummoningsickness=true|false` - Remove summoning sickness from all creatures

**Player-Specific (prefix: p0/p1/human/ai):**
- `{prefix}life=N` - Life total
- `{prefix}landsplayed=N` - Lands played this turn
- `{prefix}landsplayedlastturn=N` - Lands played last turn
- `{prefix}counters=TYPE=N,TYPE2=N2` - Player counters (e.g., POISON=3,ENERGY=5)
- `{prefix}manapool=W W U B` - Mana in pool (space-separated)
- `{prefix}persistentmana=R R` - Persistent mana
- `{prefix}numringtemptedyou=N` - Ring tempted count (LotR)
- `{prefix}speed=N` - Speed counters
- `{prefix}precast=CardName;CardName2` - Cast these spells during setup
- `{prefix}putonstack=CardName` - Put these on the stack

**Zone Fields (prefix: p0/p1/human/ai):**
- `{prefix}hand=` - Cards in hand
- `{prefix}battlefield=` - Cards on battlefield
- `{prefix}graveyard=` - Cards in graveyard
- `{prefix}library=` - Cards in library
- `{prefix}exile=` - Cards in exile
- `{prefix}command=` - Cards in command zone
- `{prefix}sideboard=` - Cards in sideboard

## Card Notation

Cards in zones are semicolon-separated with pipe-delimited modifiers:

### Basic Format
```
CardName|Modifier1|Modifier2;NextCard|Modifier
```

### Common Modifiers

**Identification:**
- `Id:123` - Assign unique ID (for referencing in attachments, etc.)
- `Set:ABC` - Card set code
- `Art:5` - Art variant index

**Battlefield State:**
- `Tapped` - Card is tapped
- `SummonSick` - Has summoning sickness
- `Attacking` - Is attacking (combat state)
- `Attacking:123` - Attacking specific planeswalker (by ID)
- `Damage:3` - Has 3 damage marked
- `Owner:P0` - Owner (if different from zone owner)

**Transformations:**
- `Transformed` - DFC on back side
- `Modal` - Modal DFC on back side
- `Flipped` - Flip card in flipped state
- `FaceDown` - Face down (morph/manifest)
- `FaceDown:Manifested` - Manifested
- `FaceDown:Cloaked` - Cloaked
- `Meld` - Melded
- `Meld:OtherCardName` - Melded with specific card

**Keyword States:**
- `Renowned` - Is renowned
- `Monstrous` - Is monstrous
- `Solved` - Is solved (case/puzzle)
- `Suspected` - Is suspected
- `Saddled` - Is saddled

**Attachments:**
- `AttachedTo:123` - Attached to card with Id:123
- `EnchantingPlayer:P0` - Enchanting player 0

**Counters:**
- `Counters:P1P1=3,LOYALTY=5` - +1/+1 and loyalty counters

**Memory/Choice:**
- `ChosenColor:W,U` - Chosen colors
- `ChosenType:Goblin` - Chosen card type
- `ChosenType2:Soldier` - Second chosen type
- `ChosenCards:123,456` - Chosen cards by ID
- `NamedCard:Lightning Bolt,Counterspell` - Named cards
- `RememberedCards:123,456` - Remembered cards by ID
- `Imprinting:123` - Imprinted cards by ID
- `ExiledWith:123` - Exiled with card ID

**Special:**
- `MergedCards:CardName1,CardName2` - Merged cards (mutate)
- `ClassLevel:3` - Class level
- `UnlockedRoom:LeftDoor` - Unlocked room
- `IsCommander` - Is a commander
- `IsRingBearer` - Is the ring bearer
- `Foretold` - Is foretold
- `ForetoldThisTurn` - Foretold this turn
- `OnAdventure` - Adventure card in exile
- `PhasedOut:P0` - Phased out (belongs to P0)
- `NoETBTrigs` - Don't trigger ETB effects
- `ExecuteScript:SVarName` - Execute script after setup

**Tokens:**
- `t:TokenInfo` - Token (uses TokenInfo format)
- `T:TokenName` - Token by name

### Examples

```
# Simple tapped land
Mountain|Tapped

# Creature with counters and attachments
Aura Gnarlid|Id:18;Slippery Bogle|Id:19;Ethereal Armor|AttachedTo:19

# Planeswalker with loyalty
Saheeli Rai|Counters:LOYALTY=5

# Transformed DFC
Werewolf Pack Leader|Transformed

# Face-down manifested creature
Grizzly Bears|FaceDown:Manifested

# Creature with damage
Serra Angel|Damage:3

# Token
t:1/1 G Saproling
```

## Scripting Features

### Precast Spells

```
p0precast=Lightning Bolt->AI;Shock->123
```

Format: `CardName->Target` where target can be:
- `HUMAN` or `AI` - Player
- `123` - Card ID
- (no target) - Untargeted spell

### Execute Scripts

```
p0battlefield=Goblin Guide|Id:50|ExecuteScript:SVarName
```

Executes a SVar from the card or custom script after setup.

### Ability References

```
ability1=AB$ Draw | NumCards$ 3
p0battlefield=Jace, Memory Adept|Ability:1
```

Define custom abilities and attach them to cards.

## Parser Implementation (Java)

Key files:
- `forge-gui/src/main/java/forge/gamemodes/puzzle/PuzzleIO.java` - File loading
- `forge-gui/src/main/java/forge/gamemodes/puzzle/Puzzle.java` - Metadata parsing and goal setup
- `forge-ai/src/main/java/forge/ai/GameState.java` - State parsing and application (1400+ lines)

### Parse Flow

1. **Load file** - Read .pzl file, split into sections
2. **Parse metadata** - Extract name, goal, turns, etc.
3. **Parse state** - Line-by-line parsing of key=value pairs
4. **Build card objects** - Create cards with all modifiers
5. **Apply to game** - Set up zones, attachments, memory, combat
6. **Add goal enforcement** - Create trigger to check win condition

### Key Features

- **ID tracking** - Cards can reference each other by ID
- **Deferred setup** - Attachments applied after all cards created
- **Trigger suppression** - Triggers disabled during setup
- **State-based effects** - Applied after full setup
- **Combat support** - Can start in middle of combat phases

## Use Cases

1. **Puzzle Mode** - Create challenging scenarios to solve
2. **Testing** - Test specific game states and interactions
3. **Bug Reproduction** - Save problematic game states
4. **Tutorial** - Teach specific mechanics
5. **Achievement Testing** - Verify achievement conditions

## Rust Implementation Considerations

### Phase 1: Core Format Support
- INI-style parser for [metadata] and [state] sections
- Player state (life, lands played, counters, mana pool)
- Zone contents (hand, battlefield, graveyard, library, exile)
- Basic card modifiers (tapped, counters, damage)

### Phase 2: References and Attachments
- ID assignment and tracking (`Id:123`)
- Attachment support (`AttachedTo:123`, `EnchantingPlayer:P0`)
- Memory/imprinting (`RememberedCards`, `Imprinting`)

### Phase 3: Advanced Features
- Card state (transformed, flipped, face-down)
- Combat state (attacking, blocking)
- Precast spells and scripting
- Goal enforcement triggers

### Phase 4: Testing Integration
- Load PZL files as test cases
- Verify game states match expected results
- Use for regression testing
- Generate PZL from live games for bug reports

### Suggested File Structure

```
src/
  puzzle/
    mod.rs           - Public API
    format.rs        - PZL format parser
    metadata.rs      - Metadata section handling
    state.rs         - Game state section handling
    card_notation.rs - Card string parsing
    loader.rs        - Load and apply to game
    goal.rs          - Goal enforcement
```

### Key Rust Types

```rust
pub struct PuzzleFile {
    pub metadata: PuzzleMetadata,
    pub state: GameStateDefinition,
}

pub struct PuzzleMetadata {
    pub name: String,
    pub url: Option<String>,
    pub goal: GoalType,
    pub turns: u32,
    pub difficulty: Difficulty,
    pub description: Option<String>,
    pub targets: Option<String>,
    pub target_count: usize,
    pub human_control: bool,
}

pub enum GoalType {
    Win,
    Survive,
    DestroySpecifiedPermanents { targets: String },
    PlaySpecifiedPermanent { targets: String, count: usize },
    GainControlOfPermanents { targets: String },
    WinBeforeOpponentTurn,
}

pub struct GameStateDefinition {
    pub turn: u32,
    pub active_player: PlayerRef,
    pub active_phase: Step,
    pub players: Vec<PlayerStateDefinition>,
}

pub struct PlayerStateDefinition {
    pub life: i32,
    pub lands_played: u32,
    pub lands_played_last_turn: u32,
    pub counters: HashMap<String, i32>,
    pub mana_pool: Vec<ManaColor>,
    pub zones: HashMap<ZoneType, Vec<CardDefinition>>,
}

pub struct CardDefinition {
    pub name: String,
    pub set_code: Option<String>,
    pub art_id: Option<u32>,
    pub id: Option<u32>,
    pub modifiers: Vec<CardModifier>,
}

pub enum CardModifier {
    Tapped,
    SummonSick,
    Counters(HashMap<CounterType, i32>),
    Damage(i32),
    AttachedTo(u32),
    Attacking(Option<u32>),
    // ... many more
}
```

### Benefits for Rust Implementation

1. **Deterministic Testing** - Create exact game states for tests
2. **Fast Test Setup** - Skip game initialization and playing to state
3. **Edge Case Coverage** - Test rare/complex interactions
4. **Regression Prevention** - Save bug states as test cases
5. **Documentation** - PZL files serve as examples
6. **Portability** - Share test cases with Java Forge
7. **Debugging** - Save/load game states during development

### Example Test Usage

```rust
#[tokio::test]
async fn test_puzzle_pauper_04() -> Result<()> {
    let puzzle = PuzzleFile::load("test_puzzles/PP04.pzl")?;
    let mut game = Game::new_from_puzzle(&puzzle)?;

    // Game is now in the exact state described in PP04.pzl
    // Run AI or test specific moves

    let result = run_puzzle(&mut game, &puzzle.metadata.goal)?;
    assert!(result.success, "Failed to solve puzzle PP04");
    Ok(())
}
```

## References

- Java source: `forge-gui/res/puzzle/*.pzl` (50+ example puzzles)
- Parser: `forge-ai/src/main/java/forge/ai/GameState.java`
- Loader: `forge-gui/src/main/java/forge/gamemodes/puzzle/PuzzleIO.java`
- Integration: `forge-gui/src/main/java/forge/gamemodes/puzzle/Puzzle.java`
