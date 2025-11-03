---
title: 'Fancy TUI: Enhancements and Polish'
status: open
priority: 1
issue_type: task
created_at: 2025-11-03T16:34:35.049692113+00:00
updated_at: 2025-11-03T20:13:00.046612305+00:00
---

# Description

Tracking issue for enhancements to the fancy TUI controller (`--p1=fancy`).

This tracks the evolution from the initial implementation to a fully-featured, polished TUI experience inspired by MTG Arena but adapted for terminal use.

## Sub-issues

**Status/Info improvements:**
- ✓ mtg-4d4e33: Library count in status bar
- ✓ mtg-a862ff: Turn counter and phase indicator
- ✓ mtg-a6f4ce: Ctrl-C and Ctrl-Z handling
- ✓ Turn display: Show player turn and global turn (commit 2051ece)

**Card display improvements:**
- ✓ mtg-fa9417: Proportionate card rectangles (3.5:2.5 ratio) and 2D battlefield layout
- ✓ mtg-bc661f: Card border colors reflecting mana colors
- ✓ mtg-b72100: Dim pane borders (grey instead of white)

**Interactive focus system:**
- ✓ mtg-b3f1fe: Pane focus with keyboard shortcuts (H, I, Y, O, A)
- ✓ Card navigation: Hand and Battlefield panes with arrow keys (commit c4d0e5c)
- ✓ mtg-fa42e3: Populate Card Details pane on selection
- mtg-1a7bae: Mouse support for card selection

**Choice presentation:**
- ✓ mtg-7bbb00: Show ownership and IDs in target choices
- mtg-8a3ffb: Enhanced choice highlighting during gameplay

**Infrastructure:**
- ✓ Logging interference fix: Memory-only mode for fancy TUI to prevent screen flickering

**Bugs (Priority 2):**
- mtg-f6b05f: Fix max mana calculation for dual lands

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
- [ ] Critical bugs (mtg-f6b05f: max mana calculation)
- [ ] Remaining interactive features (mtg-1a7bae: mouse support)
- [ ] Advanced choice presentation (mtg-8a3ffb)

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

4. **PRIORITY 2 - Critical bugs:**
   - mtg-f6b05f: Max mana calculation bug ← HIGH PRIORITY

5. **Remaining interactive features:**
   - mtg-1a7bae: Mouse support

6. **Choice improvements:**
   - mtg-8a3ffb: Enhanced choice highlighting

7. **Polish (deferred):**
   - mtg-6326b9: Improved card rendering
