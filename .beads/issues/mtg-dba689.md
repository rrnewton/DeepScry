---
title: 'Fancy TUI: Enhancements and Polish'
status: open
priority: 1
issue_type: task
created_at: 2025-11-03T16:34:35.049692113+00:00
updated_at: 2025-11-04T11:24:38.873295822+00:00
---

# Description

## Description

Tracking issue for enhancements to the fancy TUI controller (`--p1=fancy`).

This tracks the evolution from the initial implementation to a fully-featured, polished TUI experience inspired by MTG Arena but adapted for terminal use.

## Sub-issues

**Status/Info improvements:**
- ✓ mtg-4d4e33: Library count in status bar
- ✓ mtg-a862ff: Turn counter and phase indicator
- ✓ mtg-a6f4ce: Ctrl-C and Ctrl-Z handling
- ✓ Turn display: Show player turn and global turn (commit 2051ece)
- ✓ mtg-29343b: Improve turn display with spacing and active player indication (commit 62cf104)

**Card display improvements:**
- ✓ mtg-fa9417: Proportionate card rectangles (3.5:2.5 ratio) and 2D battlefield layout
- ✓ mtg-bc661f: Card border colors reflecting mana colors
- ✓ mtg-b72100: Dim pane borders (grey instead of white)
- ✓ mtg-897dd0: Respect \n in card text display (commit 62cf104)
- ✓ mtg-1af4f0: Smarter layout with variable sized cards (commits 964113e, edd041f)
- ✓ Card rendering improvements: Aspect ratio and priority-based layout (commit 65ad5b3)
- ✓ mtg-6326b9: Intelligent space usage with progressive compaction (2025-11-04)
- ✓ Max card height limit (15 rows, configurable parameter for future)
- ✓ mtg-cf6f3f: Simple stacking with multiplier prefix (e.g., "3x Island") - 2025-11-04
- mtg-a07166: Visual stacking with diagonal offsets (depends on mtg-cf6f3f)

**Interactive focus system:**
- ✓ mtg-b3f1fe: Pane focus with keyboard shortcuts (H, I, Y, O, A, S)
- ✓ Card navigation: Hand and Battlefield panes with arrow keys (commit c4d0e5c)
- ✓ mtg-fa42e3: Populate Card Details pane on selection
- ✓ mtg-1a7bae: Mouse support for card selection (2025-11-04)

**Choice presentation:**
- ✓ mtg-7bbb00: Show ownership and IDs in target choices
- ✓ mtg-8a3ffb: Enhanced choice highlighting during gameplay (2025-11-04)

**Infrastructure:**
- ✓ Logging interference fix: Memory-only mode for fancy TUI to prevent screen flickering
- ✓ mtg-f6b05f: Fix max mana calculation for dual lands (commits fb0b159, 8d61403)
- ✓ mtg-7216cc: Replace println/eprintln with logger calls (commit f4a6938)

**UI Reorganization:**
- ✓ mtg-f567b1: Move Stack and Actions panes, remove Dock tab

## Recent progress (2025-11-04)

Simple stacking (mtg-cf6f3f) - COMPLETED:
- Phase 1: Trait abstraction with BattlefieldEntity trait (commit 97c1ef1)
- Phase 2: Enable actual stacking with grouping logic (commit 22ac0c0)
- Displays "3x Island" with cyan multiplier prefix
- Groups cards by (name, tapped_state)
- Aspect ratio fix for tapped stacks (commit 4d807df)
- All 405 tests passing

Logging improvements (mtg-7216cc) - COMPLETED:
- Replaced all println!/eprintln! with logger calls (commit f4a6938)
- Centralized damage logging with life totals
- All combat logs captured (attacks, blocks, damage)
- All player actions logged (lands, spells, abilities, mana)
- Verified no stdout interference in fancy TUI mode
- All 276 tests passing

## Implementation order

Completed phases:

1. ✅ **Quick wins**: Library count, dim borders, signal handling, turn/phase indicator, card colors, ownership display, logging fix
2. ✅ **Visual foundation**: 2D battlefield layout with proper aspect ratios
3. ✅ **Pane focus and navigation**: Keyboard shortcuts (H/I/Y/O/A/S), arrow key navigation, card details
4. ✅ **Critical bugs**: Max mana calculation, aspect ratio consistency
5. ✅ **UI reorganization**: Stack/Actions panes repositioned
6. ✅ **Card rendering enhancements**: Variable sized cards, turn display, card text newlines, intelligent layout, max height
7. ✅ **Interactive features**: Mouse support, choice highlighting
8. ✅ **Polish**: Intelligent space usage, progressive compaction
9. ✅ **Simple stacking** (mtg-cf6f3f): Multiplier prefix for duplicate cards
10. ✅ **Logging improvements** (mtg-7216cc): Centralize logging, add life totals, capture all actions

Next priorities:

11. ⏸ **Visual stacking** (mtg-a07166): Diagonal offsets and partial rendering

The fancy TUI is feature-complete! All logging is centralized and working properly. Next: add visual stacking for even more polish.
