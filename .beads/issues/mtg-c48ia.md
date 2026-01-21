---
title: Add missing State-Based Actions (0 toughness, aura/equipment)
status: open
priority: 3
issue_type: task
labels:
- draft
- refactoring
created_at: 2026-01-21T00:26:54.275326928+00:00
updated_at: 2026-01-21T00:26:54.275326928+00:00
---

# Description

## Problem

The SBA handling in `game/actions/mod.rs` is missing some important state-based actions defined in the comprehensive rules.

### Missing SBAs

1. **CR 704.5c - 0 Toughness Check**: Creatures with 0 or less toughness should be put into graveyard as an SBA. Currently only lethal damage is checked, not base toughness reduction.

2. **CR 704.5d - Aura Attachment**: Auras that are attached to illegal permanents or not attached to anything should be put into graveyard.

3. **CR 704.5e - Equipment Attachment**: Equipment that became a creature while attached should become unattached.

4. **CR 704.5p - +1/+1 and -1/-1 Counter Annihilation**: When a permanent has both +1/+1 and -1/-1 counters, they should annihilate in pairs.

### Current State

The `check_state_based_actions()` function handles:
- Lethal damage (toughness <= damage marked)
- Players at 0 or less life
- Players who drew from empty library (optional rule)
- Legend rule
- Planeswalker uniqueness rule

But it does NOT handle:
- Toughness-based death (toughness reduced to 0 or less without damage)
- Aura/Equipment reattachment rules
- Counter annihilation

### Impact

Cards like Tragic Slip ("-13/-13 until end of turn") or Disfigure ("-2/-2") may not correctly kill creatures because we only check damage, not toughness reduction.

### Proposed Fix

Add the missing SBA checks to `check_state_based_actions()` function in the correct order per CR 704.3.

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing:
- No performance regressions allowed - run benchmarks before/after
- SBA checks run after every action - this is a hot path
- Batch creature deaths to avoid redundant zone transitions
- Use iterators over battlefield, avoid collecting into intermediate Vecs

## Status

DRAFT - awaiting approval before implementation
