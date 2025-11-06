# Allocation Hotspot Breakdown - Specific Code Locations

**Date**: 2025-11-06
**Analysis**: Post-optimization (SmallVec + text_lowercase)
**Total allocations**: 552.72 KB in 4,406 blocks

This document identifies the **exact code locations** responsible for each allocation hotspot.

---

## GAMEPLAY ALLOCATIONS (during actual game turns)

### #1: UndoLog Vec Growth - 97.92 KB (17.7%) ⚠️ HIGH PRIORITY

**Location**: `mtg-engine/src/undo.rs:294-295`
**Function**: `UndoLog::log()`
**Called from**: `GameState::draw_card()` (state.rs:293)

```rust
// undo.rs:294-295
pub fn log(&mut self, action: UndoAction) {
    self.actions.push(action);  // <-- HOTSPOT: Vec growth
    self.metadata.push(UndoMetadata::new());  // <-- Also allocates
}
```

**What's allocating**:
- `self.actions: Vec<UndoAction>` grows dynamically as game actions occur
- Called every time a card is drawn, land is played, spell is cast, etc.
- Over 20 turns with realistic decks, this grows to thousands of actions

**Optimization opportunity**:
```rust
// Pre-size the UndoLog based on estimated actions per turn
impl UndoLog {
    pub fn new() -> Self {
        const ESTIMATED_ACTIONS_PER_TURN: usize = 50;
        const TYPICAL_GAME_LENGTH: usize = 20;
        let estimated_capacity = ESTIMATED_ACTIONS_PER_TURN * TYPICAL_GAME_LENGTH;

        Self {
            actions: Vec::with_capacity(estimated_capacity),  // ~1000
            metadata: Vec::with_capacity(estimated_capacity),
            // ...
        }
    }
}
```

**Expected impact**: ~50-80 KB reduction (50-80% of this hotspot)

---

### #8: UndoLog Vec Growth (Secondary) - 8.16 KB (1.5%)

**Same location as #1, different call path**
- Same code: `undo.rs:295` (metadata push)
- Different stack trace from `draw_card()` during opening hand setup

---

## INITIALIZATION ALLOCATIONS (one-time, during game setup)

### #2 + #3: EntityStore<Card> HashMap Resizing - 175.37 KB (31.7%) ⚠️ HIGH PRIORITY

**Location**: `mtg-engine/src/core/entity.rs:224`
**Function**: `EntityStore<T>::insert()`
**Type**: `GameState.cards: EntityStore<Card>` - main card storage HashMap
**Called from**: `GameInitializer::load_deck_into_game()` (game_init.rs:78)

```rust
// entity.rs:224
impl<T: GameEntity<Id>, Id: EntityId> EntityStore<T> {
    pub fn insert(&mut self, id: Id, entity: T) -> Result<()> {
        self.entities.insert(id, entity);  // <-- HOTSPOT: HashMap resize
        // ...
    }
}
```

**What's allocating**:
- `self.entities: HashMap<CardId, Card>` starts empty (default capacity ~0)
- As we load 60-card deck for player 1, it resizes: 0 → 3 → 7 → 14 → 28 → 56 → 112
- As we load 60-card deck for player 2, it resizes again to 224
- Each resize allocates new table and rehashes all entries
- Two separate hotspots (#2 and #3) represent different resize events

**Optimization opportunity**:
```rust
// In GameState::new() or GameInitializer
pub fn new_with_deck_size(deck_size: usize) -> Self {
    const TYPICAL_DECK_SIZE: usize = 60;
    let total_cards = deck_size * 2;  // Both players

    Self {
        cards: EntityStore::with_capacity(total_cards),  // Pre-size to 120
        // ...
    }
}

// In entity.rs
impl<T: GameEntity<Id>, Id: EntityId> EntityStore<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entities: HashMap::with_capacity(capacity),
            next_id: PhantomData,
        }
    }
}
```

**Even better - use FxHashMap**:
```rust
use rustc_hash::FxHashMap;  // or ahash::HashMap

pub struct EntityStore<T, Id = EntityId> {
    entities: FxHashMap<Id, T>,  // 2-3x faster for integer keys
    // ...
}
```

**Expected impact**:
- Pre-sizing: ~100 KB reduction (eliminates most resizes)
- FxHashMap: Additional 20-30% speed improvement + 10-20 KB memory reduction

---

### #10: String Formatting in Card Loader - 6.87 KB (1.2%)

**Location**: `mtg-engine/src/loader/card.rs:88`
**Function**: `CardLoader::parse()`
**Called during**: Card loading from file

```rust
// card.rs:88 (approximately)
raw_abilities.push(format!("{key}:{value}"));  // <-- String allocation
```

**What's allocating**:
- Creating formatted strings for raw ability parsing
- Called 74 times (likely once per ability line in card files)

**Optimization**:
- Low priority - this is one-time initialization
- Could use string interning if needed

---

### #9: Arc Allocations for CardDefinition - 6.91 KB (1.2%)

**Location**: `mtg-engine/src/loader/database_async.rs:96`
**Function**: `CardDatabase::get_card()`

```rust
// database_async.rs:96
Arc::new(card_def)  // <-- Arc allocation for sharing card definitions
```

**What's allocating**:
- Wrapping CardDefinition in Arc for shared ownership
- 36 blocks = 36 unique cards loaded
- Necessary for async card loading

**Optimization**: None needed - this is acceptable overhead for shared ownership

---

### #6: File Reading Buffers - 16.55 KB (3.0%)

**Location**: `std/src/fs.rs:351` → called from tokio
**Function**: `std::fs::read_to_string()`
**Called from**: Card file loading

**What's allocating**:
- String buffer for reading .txt card files
- 36 blocks = 36 card files read from disk
- Pre-reserves exact file size

**Optimization**: None needed - this is efficient already

---

### #4 + #5 + #7: Tokio Task Allocation - 50.92 KB (9.2%)

**Locations**:
- `tokio::runtime::task::new_task()`
- Called from `CardDatabase::load_cards()` (database_async.rs:140)

**What's allocating**:
- Tokio async task structures for parallel card loading
- 36 + 28 + 35 = 99 tasks total
- Each task ~256-640 bytes

**Optimization**: None needed - this is async runtime overhead for initialization

---

## SUMMARY TABLE: Optimization Priorities

| Hotspot | Location | Type | Size | Priority | Potential Savings |
|---------|----------|------|------|----------|-------------------|
| **#1** | `undo.rs:294` `UndoLog::log()` | Vec growth | 97.92 KB | 🔴 HIGH | 50-80 KB |
| **#2+#3** | `entity.rs:224` `EntityStore::insert()` | HashMap resize | 175.37 KB | 🔴 HIGH | 100-150 KB |
| #4+#5+#7 | Tokio tasks | Async init | 50.92 KB | 🟢 LOW | N/A (init) |
| #6 | File reading | I/O buffers | 16.55 KB | 🟢 LOW | N/A (init) |
| **#8** | `undo.rs:295` | Vec growth | 8.16 KB | 🟡 MED | Covered by #1 |
| #9 | `database_async.rs:96` | Arc | 6.91 KB | 🟢 LOW | N/A (init) |
| #10 | `card.rs:88` | String format | 6.87 KB | 🟢 LOW | N/A (init) |

---

## CONCRETE OPTIMIZATION PLAN

### Phase 1: EntityStore Pre-sizing (Expected: ~100-150 KB savings)

**Files to modify**:
1. `mtg-engine/src/core/entity.rs`
   - Add `EntityStore::with_capacity()`

2. `mtg-engine/src/game/state.rs`
   - Add `GameState::with_deck_capacity()`

3. `mtg-engine/src/loader/game_init.rs`
   - Use pre-sized EntityStore when initializing game

**Code changes**:
```rust
// entity.rs
impl<T: GameEntity<Id>, Id: EntityId> EntityStore<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entities: HashMap::with_capacity(capacity),
            next_id: PhantomData,
        }
    }
}

// state.rs
impl GameState {
    pub fn with_deck_capacity(deck_size: usize) -> Self {
        let total_cards = deck_size * 2;  // Both players
        Self {
            cards: EntityStore::with_capacity(total_cards),
            // ... rest of initialization
        }
    }
}

// game_init.rs - in init_game()
let mut game = GameState::with_deck_capacity(60);  // Instead of GameState::new()
```

### Phase 2: UndoLog Pre-sizing (Expected: ~50-80 KB savings)

**Files to modify**:
1. `mtg-engine/src/undo.rs`
   - Update `UndoLog::new()` to pre-allocate based on typical game

**Code changes**:
```rust
// undo.rs
impl UndoLog {
    pub fn new() -> Self {
        const ESTIMATED_ACTIONS_PER_TURN: usize = 50;
        const TYPICAL_GAME_LENGTH: usize = 20;
        let estimated_capacity = ESTIMATED_ACTIONS_PER_TURN * TYPICAL_GAME_LENGTH;

        Self {
            actions: Vec::with_capacity(estimated_capacity),
            metadata: Vec::with_capacity(estimated_capacity),
            choice_points: Vec::new(),  // Smaller, can grow naturally
            enabled: true,
        }
    }
}
```

### Phase 3 (Optional): FxHashMap for EntityStore (Expected: 10-20 KB + speed)

**Files to modify**:
1. `mtg-engine/Cargo.toml` - add `rustc-hash` dependency
2. `mtg-engine/src/core/entity.rs` - use FxHashMap instead of HashMap

**Rationale**:
- EntityStore uses integer IDs (CardId, PlayerId)
- FxHashMap is 2-3x faster for integer keys
- Uses less memory (simpler hash function)

---

## TOTAL EXPECTED IMPROVEMENT

**Current**: 552.72 KB
**After Phase 1+2**: ~350-400 KB (35-40% reduction)
**After Phase 3**: ~330-380 KB (40-45% total reduction from current)

**Combined with previous ManaEngine work**:
- Original baseline: 669.64 KB
- After all optimizations: ~330-380 KB
- **Total reduction: 290-340 KB (43-51% improvement)**
