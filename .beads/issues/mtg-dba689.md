---
title: 'Fancy TUI: Enhancements and Polish'
status: open
priority: 1
issue_type: task
created_at: 2025-11-03T16:34:35.049692113+00:00
updated_at: 2025-11-03T23:54:58.323097845+00:00
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

**Interactive focus system:**
- ✓ mtg-b3f1fe: Pane focus with keyboard shortcuts (H, I, Y, O, A, S)
- ✓ Card navigation: Hand and Battlefield panes with arrow keys (commit c4d0e5c)
- ✓ mtg-fa42e3: Populate Card Details pane on selection
- mtg-1a7bae: Mouse support for card selection

**Choice presentation:**
- ✓ mtg-7bbb00: Show ownership and IDs in target choices
- mtg-8a3ffb: Enhanced choice highlighting during gameplay

**Infrastructure:**
- ✓ Logging interference fix: Memory-only mode for fancy TUI to prevent screen flickering
- ✓ mtg-f6b05f: Fix max mana calculation for dual lands (commits fb0b159, 8d61403)

**UI Reorganization:**
- ✓ mtg-f567b1: Move Stack and Actions panes, remove Dock tab

**Deferred improvements:**
- mtg-6326b9: Card rendering improvements with intelligent space usage

## Status

- [x] Initial fancy TUI implementation (commit 04dc7ed)
- [x] Basic info enhancements (mtg-4d4e33, mtg-a862ff, mtg-a6f4ce, turn display)
- [x] Initial visual polish (mtg-bc661f, mtg-b72100)
- [x] Logging infrastructure (memory-only mode)
- [x] 2D battlefield layout (mtg-fa9417) - major refactor
- [x] Pane focus system (mtg-b3f1fe)
- [x] Card navigation in Hand and Battlefield (commit c4d0e5c)
- [x] Card Details population on selection (mtg-fa42e3)
- [x] Max mana calculation fix (mtg-f6b05f)
- [x] UI reorganization (mtg-f567b1)
- [x] Turn display improvements (mtg-29343b) - commit 62cf104
- [x] Card text newlines (mtg-897dd0) - commit 62cf104
- [x] Smarter layout with variable sized cards (mtg-1af4f0) - commits 964113e, edd041f, 65ad5b3
- [ ] Remaining interactive features (mtg-1a7bae: mouse support)
- [ ] Advanced choice presentation (mtg-8a3ffb)

## Recent progress (2025-11-03)

Major card rendering improvements in today's session:
- Commit 964113e: Phase 1 - Natural card tapping with dimension swapping
- Commit edd041f: Phase 2 - Greedy card size optimization
- Commit 65ad5b3: Aspect ratio fixes and priority-based card content layout
- Commit 62cf104: Turn display and card text newline rendering

Cards now:
- Maintain proper aspect ratio (10:7) when resizing
- Dynamically size to fill battlefield (greedy optimization)
- Show detailed content (title, cost, tapped indicator, P/T, description, type)
- Display P/T in bottom-right corner
- Respect newlines in card text (Card Details pane)
- Swap dimensions when tapped to simulate rotation

## Implementation order

Updated order based on user priority:

1. **COMPLETED - Quick wins:**
   - ✓ mtg-4d4e33: Library count
   - ✓ mtg-b72100: Dim borders
   - ✓ mtg-a6f4ce: Signal handling
   - ✓ mtg-a862ff: Turn/phase indicator
   - ✓ mtg-bc661f: Card border colors
   - ✓ mtg-7bbb00: Ownership/IDs in targets
   - ✓ Logging fix: Memory-only mode
   - ✓ Turn display: Player turn and global turn

2. **COMPLETED - Visual foundation:**
   - ✓ mtg-fa9417: 2D battlefield layout (major refactor)

3. **COMPLETED - Pane focus and navigation:**
   - ✓ mtg-b3f1fe: Pane focus system
   - ✓ Card navigation in Hand and Battlefield (c4d0e5c)
   - ✓ mtg-fa42e3: Card Details population

4. **COMPLETED - Critical bugs:**
   - ✓ mtg-f6b05f: Max mana calculation bug (fb0b159, 8d61403)

5. **COMPLETED - UI reorganization:**
   - ✓ mtg-f567b1: Move Stack/Actions, remove Dock

6. **COMPLETED - Card rendering enhancements:**
   - ✓ mtg-1af4f0: Smarter layout with variable sized cards
   - ✓ mtg-29343b: Turn display improvements (spacing, active player)
   - ✓ mtg-897dd0: Card text newlines

7. **Remaining interactive features:**
   - mtg-1a7bae: Mouse support

8. **Choice improvements:**
   - mtg-8a3ffb: Enhanced choice highlighting

9. **Polish (deferred):**
   - mtg-6326b9: Further card rendering improvements
