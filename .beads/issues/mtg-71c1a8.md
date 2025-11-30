---
title: 'Bug: Vibrant Cityscape tutor ability doesn''t search library (fizzles)'
status: closed
priority: 2
issue_type: bug
created_at: 2025-11-30T01:38:10.628973363+00:00
updated_at: 2025-11-30T13:57:11.001975377+00:00
closed_at: 2025-11-30T13:57:11.001975307+00:00
---

# Description

## Bug Report

## Investigation (2025-11-29)

### Root Cause FOUND

The Vibrant Cityscape ability IS being parsed and IS executing - but it's **missing player interaction**.

**Code Flow:**
1. ✅ Ability parsing:  correctly creates  effect
2. ✅ Effect execution:  searches library and moves card
3. ❌ **BUG**: No player choice - automatically picks first matching card (line 893)

**Current Implementation (actions/mod.rs:887-897):**
```rust
let mut found_card = None;
for &card_id in &library_cards {
    if let Ok(card) = self.cards.get(card_id) {
        if Self::card_matches_search_filter(card, card_type_filter) {
            found_card = Some(card_id);  // AUTO-SELECT first match\!
            break;  // No player choice\!
        }
    }
}
```

**What's Happening:**
- Vibrant Cityscape ability activates correctly ✓
- Cost is paid (tap + sacrifice land) ✓
- SearchLibrary effect executes ✓
- First basic land in library is auto-selected
- Land enters battlefield tapped ✓
- Library is shuffled ✓
- **Player never sees library or makes a choice** ❌

### Why It Seems to "Fizzle"

From the player's perspective:
- They activate the ability
- No library search UI appears
- No choice is presented
- A random land (first in library) appears on battlefield
- This looks like it "did nothing" or "fizzled"

But the code IS working - it's just invisible/automatic\!

### Fix Required

**Add player interaction to SearchLibrary:**

1. **Controller method needed:**
   ```rust
   fn choose_from_library(
       &mut self,
       view: &GameStateView,
       valid_cards: &[CardId],
   ) -> Result<Option<CardId>>;  // None = decline to find
   ```

2. **Update SearchLibrary execution (actions/mod.rs:863-915):**
   - Filter library cards by card_type_filter
   - Present filtered list to controller
   - Let player choose (or decline)
   - Move chosen card to destination

3. **TUI Implementation:**
   - Show library view with filtered cards
   - Allow scrolling/selection
   - Confirm choice
   - Similar to existing targeting UI

### Implementation Plan

1. Add  variant (if needed)
2. Add  method
3. Update  execution to call controller
4. Implement TUI library view
5. Handle "decline to find" (legal in MTG - you can fail to find)

### Test Case

```rust
// Test that SearchLibrary asks for player choice
let mut game = GameState::new_two_player("P1", "P2", 20);
let p1 = game.players[0].id;

// Put basic lands in P1's library
add_forest_to_library(&mut game, p1);
add_plains_to_library(&mut game, p1);

// Execute SearchLibrary for Land.Basic
let effect = Effect::SearchLibrary {
    player: p1,
    card_type_filter: "Land.Basic".to_string(),
    destination: Zone::Battlefield,
    enters_tapped: true,
    shuffle: true,
};

// Should call controller.choose_from_library([Forest, Plains])
// Currently: auto-picks first without asking
```

### Priority

**Medium-High** - Common mechanic (fetchlands, tutors, ramp spells)

### Workaround

For testing: The ability DOES work, it just auto-selects the first matching card. So Vibrant Cityscape will fetch a basic land, it's just not interactive.

---

**Status:** Root cause identified. Needs player interaction implementation.

**Complexity:** Medium (requires TUI changes + controller method)

**Related:** Similar issue affects ALL library search effects (Demonic Tutor, Evolving Wilds, fetchlands, etc.)
