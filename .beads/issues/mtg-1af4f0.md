---
title: Smarter layout with variable sized cards
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T20:52:42.261713655+00:00
updated_at: 2025-11-03T21:01:36.470327241+00:00
closed_at: 2025-11-03T21:01:36.470327111+00:00
---

# Description

## Description

## Goal

Implement smart card layout with variable sizing to maximize use of battlefield space.

## Phase 1: Natural Card Tapping ✓ COMPLETED (commit 964113e)

Support tapped cards by swapping width/height dimensions to simulate 90-degree rotation.

**Details:**
- Tapped cards swap dimensions: if normal card is 10x7, tapped becomes 7x10
- Text remains in normal orientation (not rotated)
- This creates dynamic card widths, requiring smart layout
- Pack as many cards as possible in each row based on actual widths

**Impact:**
- No longer fixed grid layout
- Dynamic row packing based on card widths (tapped vs untapped)

## Phase 2: Grow Cards to Fill Battlefield ✓ COMPLETED (commit edd041f)

Implement algorithm to maximize card size within battlefield constraints.

**Algorithm:**
1. Treat cards as linear list grouped by sections (Lands:, Creatures:, Other:)
2. Section headers remain, but cards wrap dynamically at row boundaries
3. For given card size, pack cards left-to-right:
   - When next card would exceed right edge, wrap to next row
   - If insufficient vertical space for new row, overflow detected
4. Start with default size and increase until overflow detected (greedy)
5. If default overflows, shrink until minimum size (5 chars: "Fores")
   - Don't bother with ellipsis at minimum (no "Badlan.." for "Badlands")
6. Make this the default behavior: always maximize card size to fit screen

**Constraints:**
- Minimum card width: 5 characters
- Must maintain section headers
- Must respect terminal dimensions
- Dynamic wrapping based on current card size

**Example Flow:**
- Default card size: 10x7
- Try 11x8: fits? Try 12x9
- 12x9: overflows vertically
- Use 11x8 as final size
- Pack all cards with this size using wrap algorithm

## Location

src/game/fancy_tui_controller.rs: 
- render_card_group() method (now accepts card_width/card_height parameters)
- render_card_box() method (lines 723-813)
- calculate_optimal_card_size() method (new - greedy algorithm)
- test_card_size_fits() method (new - simulation function)
- get_card_dimensions_with_size() method (new - parameterized dimensions)

## Status

✅ Both phases complete and validated (all 405 tests passing)
