---
title: 'Bug: Peter Porker (Spider-Ham) incorrectly disappears when Food token is sacrificed'
status: open
priority: 2
issue_type: bug
created_at: 2025-11-30T01:37:46.253851447+00:00
updated_at: 2025-11-30T17:39:25.767656584+00:00
---

# Description

## Peter Porker TUI Bug - Disappears After Attacking

## Status: INVESTIGATING - Debug Infrastructure In Place

## Problem

After attacking with Spider-Ham, Peter Porker (id=29) or when Food token is sacrificed, Peter Porker disappears from the TUI battlefield display. However:

- Peter Porker IS still in the game state (confirmed via debug logging)
- Peter Porker IS still targetable (shows in target selection UI)
- The "Creatures:" section shows EMPTY despite Peter Porker being on battlefield

## Debug Evidence from Zone Logs

Zone movement logs confirm Peter Porker reaches battlefield correctly:

```
[DEBUG zone] Moving card Spider-Ham, Peter Porker (id=29) from Stack to Battlefield
[DEBUG token] Created token Food Token (id=84) under player 0's control
```

## Code Analysis Complete

1. **Card Loading ✓**: Verified Peter Porker is correctly parsed as CardType::Creature
   - Card file: `Types:Legendary Creature Spider Boar Hero`
   - Parser (card.rs:52-64): Correctly splits types from subtypes
   - Result: types=[Creature], subtypes=[Legendary, Spider, Boar, Hero]

2. **Type Checking ✓**: Verified is_creature() implementation
   - card.rs:331-333: Simply checks `self.types.contains(&CardType::Creature)`
   - Should return true for Peter Porker

3. **TUI Rendering**: Found TWO identical categorization code blocks
   - `get_battlefield_cards_in_order()` (line 386): Used for target selection
   - `draw_battlefield()` (line 1041): Used for TUI display
   - Both use same if-else chain: land → creature → artifact → enchantment
   - Both filter by controller, both use same GameStateView

4. **Hypothesis**: Since Peter Porker IS targetable (via get_battlefield_cards_in_order)
   but NOT visible (via draw_battlefield), there must be a subtle difference in:
   - How/when these functions are called
   - The state of the GameStateView at different times
   - OR a caching/timing issue

## Debug Infrastructure Added (Commit 9dbfe2b)

Enhanced fancy_tui_controller.rs with comprehensive logging:
- Log total battlefield card count
- Log each card's controller vs requested owner_id
- Specifically highlight Peter Porker movements
- Log results after filtering and categorization

## Next Steps

1. **Manual Testing Required**: Run `./scripts/debug_peter_porker_tui.sh`
   - Fancy TUI requires interactive terminal (can't automate)
   - Logs automatically written to mtg_forge.log
   
2. **Reproduce Bug**:
   - Play Spider-Ham, Peter Porker
   - Attack with Peter Porker
   - Observe disappearance
   - Exit game
   
3. **Analyze Logs**: Look for these patterns:
   ```bash
   grep 'Peter Porker' mtg_forge.log
   grep 'Categorizing.*Peter' mtg_forge.log
   grep 'draw_battlefield.*player 0' mtg_forge.log
   ```

4. **Key Questions to Answer**:
   - Does Peter Porker pass the controller filter?
   - What does is_creature() return?
   - Which category bucket does it land in?
   - Is owner_id parameter correct?

## Reproduction

Occurs in interactive TUI gameplay with Spider-Man draft decks containing Peter Porker.
Cannot reproduce with non-interactive controllers (heuristic/random don't use Fancy TUI).

## Related Code

- mtg-engine/src/game/fancy_tui_controller.rs:386 (target selection categorization)
- mtg-engine/src/game/fancy_tui_controller.rs:1041 (display categorization)
- mtg-engine/src/core/card.rs:331 (is_creature implementation)
- mtg-engine/src/loader/card.rs:52 (type parsing)

## Timeline

- 2025-11-30_#218(9dbfe2b): Added debug logging infrastructure
- 2025-11-30_#217(72b552b): Initial debug logs (before chatter migration)
- Earlier: User first reported bug with screenshot showing empty Creatures section

---

Once debug logs are captured, will analyze and implement fix in subsequent commit.
