# DHAT Heap Profile Analysis - 2025-11-04

## Summary

Profiled 100 iterations of "rewind + play again" pattern to isolate **forward gameplay allocations only** (excludes initialization overhead).

**Total allocations:** 1,861,274 bytes in 28,793 blocks
**Peak memory:** 217,311 bytes in 642 blocks
**Per-iteration average:** ~18,613 bytes/iteration

## Top 10 Allocation Hotspots

### #1: ManaEngine::update - 406,800 bytes (21.9%)
- **Location:** `src/game/mana_engine.rs:264:27`
- **Pattern:** Vec reserve during mana source collection
- **565 blocks, avg 720 bytes/block**
- **Call stack:**
  ```
  ManaEngine::update
  └─ GameLoop::priority_round::{{closure}}
     └─ GameState::cast_spell_8_step
        └─ GameLoop::priority_round
  ```
- **Analysis:** Mana engine updates happen during spell casting. The 720-byte allocations suggest Vec reallocation for source lists.
- **Optimization potential:** HIGH - can reuse buffer like we did for mana payment

### #2: GameState::advance_step (serde_json) - 153,600 bytes (8.3%)
- **Location:** `src/game/state.rs:456:17`
- **Pattern:** JSON serialization via `serde_json::to_vec`
- **800 blocks, avg 192 bytes/block**
- **Analysis:** Serializing game actions for undo log. Each step advance serializes state changes.
- **Optimization potential:** MEDIUM - could use bincode or preallocated buffer

### #3: ManaEngine::update (draw step) - 120,960 bytes (6.5%)
- **Location:** `src/game/mana_engine.rs:264:27`
- **Pattern:** Same as #1, but during draw step priority
- **168 blocks, avg 720 bytes/block**
- **Analysis:** Duplicate of #1 but from different call site (draw step vs main phase)
- **Optimization potential:** HIGH - same fix as #1

### #4: RandomController::choose_spell_ability_to_play - 93,000 bytes (5.0%)
- **Location:** `src/game/random_controller.rs:104:14`
- **Pattern:** `Vec::extend_from_slice` when collecting playable abilities
- **3,100 blocks, avg 30 bytes/block**
- **Analysis:** AI controller allocates when building list of options. Small allocations but high frequency.
- **Optimization potential:** MEDIUM - could pass output buffer to avoid allocation

### #5: EntityStore::insert (deck loading) - 84,240 bytes (4.5%)
- **Location:** `src/core/entity.rs:224:23` via `game_init.rs:78`
- **Pattern:** HashMap resize during deck loading
- **1 block, 84,240 bytes**
- **Analysis:** One-time initialization cost from first iteration. Should be excluded from forward gameplay.
- **Optimization potential:** LOW - initialization overhead only

### #6: EntityStore::insert (deck loading #2) - 83,004 bytes (4.5%)
- **Location:** `src/core/entity.rs:224:23` via `game_init.rs:57`
- **Pattern:** HashMap resize for second player
- **6 blocks, avg 13,834 bytes/block**
- **Analysis:** Same as #5, player 2 deck loading
- **Optimization potential:** LOW - initialization overhead only

### #7: ManaEngine::update (get_castable_spells) - 72,000 bytes (3.9%)
- **Location:** `src/game/mana_engine.rs:264:27`
- **Pattern:** Vec reserve during spell availability check
- **100 blocks, avg 720 bytes/block**
- **Call stack:**
  ```
  ManaEngine::update
  └─ GameLoop::get_castable_spells
     └─ GameLoop::get_available_spell_abilities
        └─ GameLoop::priority_round
  ```
- **Analysis:** Third instance of the same ManaEngine allocation pattern
- **Optimization potential:** HIGH - same fix as #1

### #8: GameLoop::get_available_spell_abilities - 52,480 bytes (2.8%)
- **Location:** `src/game/game_loop.rs:3054:23`
- **Pattern:** `Vec::push` collecting available abilities
- **775 blocks, avg 67.7 bytes/block**
- **Analysis:** Building list of playable abilities for AI decision. High frequency small allocations.
- **Optimization potential:** MEDIUM - could reuse buffer across priority rounds

### #9: UndoLog::log - 44,704 bytes (2.4%)
- **Location:** `src/undo.rs:283:26`
- **Pattern:** Vec::push for undo log growth
- **7 blocks, avg 6,386 bytes/block**
- **Analysis:** Undo log growing during initial game setup (draw opening hands). Large growth events.
- **Optimization potential:** LOW - undo log must grow dynamically

### #10: RandomController format! - 40,680 bytes (2.2%)
- **Location:** `src/game/random_controller.rs:94:17`
- **Pattern:** String formatting via `format!` macro
- **1,130 blocks, avg 36 bytes/block**
- **Call stack:**
  ```
  RandomController::choose_spell_ability_to_play (format!)
  └─ GameLoop::priority_round
  ```
- **Analysis:** Debug/logging string formatting in controller decision-making. High frequency!
- **Optimization potential:** HIGH - likely verbose-logging feature not disabled

## Summary by Category

### Critical Hot Path (forward gameplay)
1. **ManaEngine::update** - 599,760 bytes total (32.2%) across 3 call sites
   - All doing the same Vec reserve operation
   - **Action:** Reuse buffer pattern like mana payment optimization

2. **RandomController logging** - 40,680 bytes (2.2%)
   - format! macro allocations despite --no-default-features
   - **Action:** Verify verbose-logging feature is properly disabled

### Medium Priority
3. **GameLoop::get_available_spell_abilities** - 52,480 bytes (2.8%)
   - Building ability lists
   - **Action:** Reuse output buffer

4. **RandomController::choose_spell_ability_to_play** - 93,000 bytes (5.0%)
   - extend_from_slice during option collection
   - **Action:** Pass output buffer to avoid allocation

5. **GameState::advance_step** - 153,600 bytes (8.3%)
   - JSON serialization for undo
   - **Action:** Consider bincode or buffer reuse

### Low Priority (setup costs)
6. **EntityStore::insert** - 167,244 bytes (9.0%)
   - HashMap resizing during initialization
   - **Action:** None - acceptable one-time cost

7. **UndoLog::log** - 44,704 bytes (2.4%)
   - Dynamic growth
   - **Action:** None - necessary for undo system

## Recommended Actions (Priority Order)

### 1. ManaEngine buffer reuse (HIGH) - 600KB savings
Store a reusable Vec in ManaEngine for collecting mana sources, similar to the mana payment optimization.

**Estimated impact:** 32% allocation reduction

### 2. Verify verbose-logging disabled (HIGH) - 41KB savings
The RandomController is still calling format! despite --no-default-features. Check that:
- benches/dhat_profile.rs doesn't re-enable it
- The feature gate is applied correctly
- No lingering format! calls outside the feature gate

**Estimated impact:** 2% allocation reduction + performance

### 3. GameLoop ability list buffer (MEDIUM) - 53KB savings
Store reusable Vec<SpellAbilityId> in GameLoop for collecting available abilities.

**Estimated impact:** 2.8% allocation reduction

### 4. Controller output buffer (MEDIUM) - 93KB savings
Change choose_spell_ability_to_play to accept &mut Vec parameter for output.

**Estimated impact:** 5% allocation reduction

## Total Potential Savings
**787KB / 1,861KB = 42% reduction** from top 4 optimizations.

## Notes
- Profile run: 100 iterations of half-game replay
- Total profiled: ~400 game actions (4 turns × 100 iterations)
- Benchmark: simple_bolt.dck (Mountains + Lightning Bolts)
- Mode: RandomController vs RandomController
