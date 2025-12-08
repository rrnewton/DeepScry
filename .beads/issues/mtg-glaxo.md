---
title: 'White Weenie Deck: Complete Card Implementation'
status: open
priority: 1
issue_type: task
labels:
- feature
- deck-support
created_at: 2025-12-08T01:34:00.476371011+00:00
updated_at: 2025-12-08T05:00:00.000000000+00:00
---

# Description

## White Weenie Deck Implementation Tracking

This issue tracks all fixes needed to properly support the `decks/old_school2/white_weenie_classic.dck` deck.

**Related Issues**:
- mtg-147: Affected$ selector parsing (Crusade's Creature.White needs this) - FIXED

## Current State (2025-12-08)

### Fixed Issues

#### 1. Crusade (WW) - FIXED ✓
**Issue**: Color-based Affected selectors not implemented
- Added `AllCreaturesOfColor` variant to `AffectedSelector` enum
- Added parsing for `Creature.COLOR` pattern in `parse_tribal_selector()`
- Added handling in `continuous_effects.rs` for calculating P/T bonuses
- Unit test `test_load_crusade_all_creatures_of_color` and puzzle e2e test `test_crusade_buffs_white_creatures` verify the fix

#### 2. Spirit Link (W) - FIXED ✓
**Issue**: Aura targeting not properly implemented
- Added Aura target handling in `get_valid_targets_for_spell()` - Auras now provide valid creature targets based on their "Enchant X" keyword
- Added `attach_aura()` function in `actions/mod.rs` to attach Auras to their targets when they resolve
- Modified `resolve_spell()` to attach Auras to their targets after entering the battlefield
- Unit test `test_spirit_link_aura_targeting` verifies Spirit Link is recognized as an Aura with Enchant Creature

### Cards Working Correctly
- Savannah Lions (W, 2/1) - Vanilla creature ✓
- Tundra Wolves (W, 1/1 First Strike) - First strike keyword works ✓
- White Knight (WW, 2/2 First Strike, Pro Black) - First strike and protection work ✓
- Serra Angel (3WW, 4/4 Flying, Vigilance) - Both keywords work ✓
- Moorish Cavalry (2WW, 3/3 Trample) - Trample works ✓
- Swords to Plowshares (W) - Exile effect and life gain work ✓
- Disenchant (1W) - Artifact/enchantment destruction ✓
- Sol Ring (1) - Artifact, mana ability ✓
- Jalum Tome (3) - Artifact with activated ability ✓
- Strip Mine (Land) - Land destruction ✓
- Crusade (WW) - Static +1/+1 to white creatures ✓ (newly fixed)
- Spirit Link (W) - Aura targeting and attachment ✓ (newly fixed)

### Cards with Remaining Issues

#### 3. Preacher (1WW) - NOT IMPLEMENTED
**Issue**: Complex control-changing ability with conditional duration
- Keyword: "You may choose not to untap CARDNAME during your untap step"
- Warning logged for unknown keyword

#### 4. Army of Allah (1WW) - PARTIAL
**Issue**: Pump all attacking creatures - needs testing

#### 5. Balance (1W) - COMPLEX
**Issue**: Multi-step balance effect for lands, hands, and creatures

## Test Commands

```bash
# Quick heuristic game to test deck
cargo run --release --bin mtg -- tui --p1 heuristic --p2 heuristic --seed 42 decks/old_school2/white_weenie_classic.dck

# Run Crusade parsing test
cargo test --release -p mtg-forge-rs test_load_crusade -- --nocapture

# Run Crusade buff e2e test
cargo test --release -p mtg-forge-rs test_crusade_buffs_white -- --nocapture

# Run Spirit Link targeting test
cargo test --release -p mtg-forge-rs test_spirit_link_aura_targeting -- --nocapture
```
