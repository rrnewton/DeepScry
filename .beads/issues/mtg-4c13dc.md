---
title: 'Bug: Peter Porker (Spider-Ham) incorrectly disappears when Food token is sacrificed'
status: open
priority: 2
issue_type: bug
created_at: 2025-11-30T01:37:46.253851447+00:00
updated_at: 2025-11-30T14:15:14.321642821+00:00
---

# Description

## Peter Porker disappearing bug - TUI rendering issue

## Status
PARTIALLY DIAGNOSED - It's a TUI rendering bug, not game logic

## Problem
After attacking with Spider-Ham, Peter Porker (id=29) or when Food token is sacrificed, Peter Porker disappears from the TUI battlefield display. However:
- Peter Porker IS still in the game state (confirmed via debug logging)
- Peter Porker IS still targetable (shows in target selection UI)
- The "Creatures:" section shows EMPTY despite Peter Porker being on battlefield

## Debug Evidence (2025-11-30)
```
[DEBUG zone] Moving card Spider-Ham, Peter Porker (id=29) from Stack to Battlefield (owner: player 0)
[DEBUG token] Created token Food Token (id=84) under player 0's control
```

Both cards are on battlefield in game state, but TUI doesn't render Peter Porker in the Creatures section.

## Root Cause Analysis
The bug is in `fancy_tui_controller.rs` battlefield rendering code. Lines 1050-1069 group cards by type:
- is_land() → lands
- is_creature() → creatures  
- is_artifact() → artifacts
- is_enchantment() → enchantments

Peter Porker should pass `is_creature()` check (it has Types: "Legendary Creature Spider Boar Hero").

**Hypothesis:** Something is making Peter Porker fail the is_creature() check OR there's a bug in the entity grouping/rendering code after line 1417.

## Next Steps
1. Add debug logging to fancy_tui_controller.rs battlefield rendering
2. Log which cards are being grouped into creatures vs other categories
3. Check if Peter Porker's types field is correctly populated
4. Investigate if the Food token creation affects Peter Porker's type somehow

## Reproducer
```bash
RUST_LOG=zone=debug,token=debug ./agentplay/start_game.sh decks/ryan_spiderman_draft.dck decks/julian_spiderman_draft.dck --p1-draw="Forest;Forest;Spider-Ham, Peter Porker"
## Then play forest, cast Peter Porker, attack with it
## Check Your Battlefield creatures section - it will be empty despite Peter being there
```
