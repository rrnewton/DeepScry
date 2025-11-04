---
title: 'Fancy TUI: Enhancements and Polish'
status: open
priority: 1
issue_type: task
created_at: 2025-11-03T16:34:35.049692113+00:00
updated_at: 2025-11-04T10:06:15.780899208+00:00
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
- mtg-cf6f3f: Simple stacking with multiplier prefix (e.g., "3x Island")
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

**UI Reorganization:**
- ✓ mtg-f567b1: Move Stack and Actions panes, remove Dock tab

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
- [x] Mouse support (mtg-1a7bae) - 2025-11-04
- [x] Enhanced choice highlighting (mtg-8a3ffb) - 2025-11-04
- [x] Intelligent card rendering layout (mtg-6326b9) - 2025-11-04
- [x] Max card height limit (2025-11-04)
- [ ] Simple stacking (mtg-cf6f3f) - next priority
- [ ] Visual stacking (mtg-a07166) - after simple stacking

## Recent progress (2025-11-04)

Created simple stacking issue (mtg-cf6f3f):
- Intermediate feature before full visual stacking
- Displays "3x Island" with cyan multiplier prefix
- Groups cards by (name, tapped_state)
- Introduces BattlefieldEntity abstraction
- Much simpler than full visual stacking
- Prerequisite for mtg-a07166

Max card height limit:
- Added MAX_CARD_HEIGHT constant (15 rows)
- Applied in calculate_optimal_card_size() greedy algorithm
- Prevents cards from becoming too large on big terminals
- Will become configurable parameter in future

Intelligent card rendering layout (mtg-6326b9):
- Progressive compaction for TAPPED marker: [TAPPED] → [T] → T based on width
- Multi-strategy name layout: uses vertical space to avoid truncation
- Better space usage: "Mountain" instead of "Mou..." when possible
- Frees up 5 chars on narrow cards by using [T] instead of [TAPPED] earlier

Enhanced choice highlighting (mtg-8a3ffb):
- Added ChoiceContext enum (PlayingSpell, DeclareAttackers, DeclareBlockers, TargetSelection)
- Tracks valid_choices for each decision context
- Highlights playable cards (bright white), dims unplayable cards (dark gray)
- Applied to all choice methods: spell/ability selection, attackers, blockers, targets
- Instant visual feedback shows what's possible at each decision point

Mouse support (mtg-1a7bae):
- Enabled mouse capture in terminal setup/restore
- Implemented card position tracking during rendering
- Added mouse click hit testing in input loop
- Click cards in battlefield to select and view details
- Automatic pane focus switching when clicking cards
- Selected cards highlighted with bold border

Previous progress (2025-11-03):
- Commit 964113e: Phase 1 - Natural card tapping with dimension swapping
- Commit edd041f: Phase 2 - Greedy card size optimization
- Commit 65ad5b3: Aspect ratio fixes and priority-based card content layout
- Commit 62cf104: Turn display and card text newline rendering

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

Next priorities:

9. ⏸ **Simple stacking** (mtg-cf6f3f): Multiplier prefix for duplicate cards
10. ⏸ **Visual stacking** (mtg-a07166): Diagonal offsets and partial rendering

The fancy TUI baseline is complete with excellent usability! Next: space-saving stacking features.
