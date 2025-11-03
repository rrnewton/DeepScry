---
title: 'Fancy TUI: Enhancements and Polish'
status: open
priority: 1
issue_type: task
created_at: 2025-11-03T16:34:35.049692113+00:00
updated_at: 2025-11-03T16:37:43.904238048+00:00
---

# Description

Tracking issue for enhancements to the fancy TUI controller (`--p1=fancy`).

This tracks the evolution from the initial implementation to a fully-featured, polished TUI experience inspired by MTG Arena but adapted for terminal use.

## Sub-issues

**Status/Info improvements:**
- mtg-4d4e33: Library count in status bar
- mtg-a862ff: Turn counter and phase indicator
- mtg-a6f4ce: Ctrl-C and Ctrl-Z handling

**Card display improvements:**
- mtg-fa9417: Proportionate card rectangles (3.5:2.5 ratio) and 2D battlefield layout
- mtg-bc661f: Card border colors reflecting mana colors
- mtg-b72100: Dim pane borders (grey instead of white)

**Interactive focus system:**
- mtg-b3f1fe: Pane focus with keyboard shortcuts (H, I, Y, O)
- mtg-1a7bae: Mouse support for card selection

**Choice presentation:**
- mtg-7bbb00: Show ownership and IDs in target choices
- mtg-8a3ffb: Enhanced choice highlighting during gameplay

## Status

- [x] Initial fancy TUI implementation (commit 04dc7ed)
- [ ] Basic info enhancements (mtg-4d4e33, mtg-a862ff, mtg-a6f4ce)
- [ ] Visual polish (mtg-fa9417, mtg-bc661f, mtg-b72100)
- [ ] Interactive features (mtg-b3f1fe, mtg-1a7bae)
- [ ] Advanced choice presentation (mtg-7bbb00, mtg-8a3ffb)

## Implementation order

Suggested order based on dependencies:

1. **Quick wins** (can be done in parallel):
   - mtg-4d4e33: Library count (simple)
   - mtg-b72100: Dim borders (simple)
   - mtg-a6f4ce: Signal handling (independent)

2. **Info enhancements:**
   - mtg-a862ff: Turn/phase indicator (builds on mtg-4d4e33)

3. **Visual foundation:**
   - mtg-fa9417: 2D battlefield layout (major refactor, do early)
   - mtg-bc661f: Card colors (easier after 2D layout)

4. **Interactive features:**
   - mtg-b3f1fe: Pane focus (builds on dim borders)
   - mtg-1a7bae: Mouse support (builds on focus system)

5. **Choice improvements:**
   - mtg-7bbb00: Ownership/IDs (independent)
   - mtg-8a3ffb: Choice highlighting (works best after pane focus)
