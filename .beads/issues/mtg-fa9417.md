---
title: 'Fancy TUI: Proportionate card rectangles and 2D battlefield layout'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T16:35:32.051260815+00:00
updated_at: 2025-11-04T01:18:38.794835057+00:00
---

# Description

Aspect ratio bug in fancy TUI card rendering - top row cards stretched vertically

## Problem
Cards in the battlefield display were showing inconsistent aspect ratios:
- Top row: cards stretched too tall relative to their width  
- Second row: correct aspect ratio

This was particularly noticeable when tapped cards (which swap width/height for rotation) appeared in the same battlefield as untapped cards.

## Root Cause
The `calculate_optimal_card_size()` greedy algorithm was:
1. Incrementing WIDTH
2. Computing height FROM width to maintain aspect ratio

However, this approach could create subtle inconsistencies when tapped cards (dimensions swapped) were mixed with untapped cards in the layout calculation.

## Solution (2025-10-22)
Refactored the aspect ratio calculation in src/game/fancy_tui_controller.rs:

1. **Centralized aspect ratio logic** - Created `compute_width_from_height()` helper function (lines 718-723):
   - Single source of truth for aspect ratio calculation
   - Computes width = height * (DEFAULT_WIDTH / DEFAULT_HEIGHT)

2. **Reversed greedy algorithm** in `calculate_optimal_card_size()` (lines 799-851):
   - Now increments HEIGHT (not width)
   - Computes width FROM height using centralized function
   - Both growing and shrinking paths use consistent calculation

3. **Maintained default 10:7 aspect ratio** throughout

## Result
- All cards maintain correct 10:7 aspect ratio consistently
- Tapped and untapped cards scale proportionally
- Centralized calculation prevents future aspect ratio bugs
- All 405 tests still passing

## Fixed in commits
- 2025-10-22_#162: Initial aspect ratio fix
- 2025-11-04 (commit 01c1dd7): Further refinement - "Centralize aspect ratio calculation and scale by height in fancy TUI"

Status: ✅ COMPLETED (2025-11-04)
