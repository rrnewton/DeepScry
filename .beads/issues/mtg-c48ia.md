---
title: Add missing State-Based Actions (0 toughness, aura/equipment)
status: closed
priority: 3
issue_type: task
labels:
- draft
- refactoring
created_at: 2026-01-21T00:26:54.275326928+00:00
updated_at: 2026-01-21T19:11:05.945701405+00:00
---

# Description

## Description

## Problem

The SBA handling in `game/actions/mod.rs` is missing some important state-based actions defined in the comprehensive rules.

### Missing SBAs (NOW IMPLEMENTED)

1. **CR 704.5c - 0 Toughness Check**: ✅ Already handled by check_lethal_damage() since damage(0) >= toughness(<=0) is true

2. **CR 704.5d - Aura Attachment**: ✅ Implemented in check_aura_attachment() - Auras not attached to valid permanent go to graveyard

3. **CR 704.5n - Equipment Attachment**: ✅ Implemented in check_equipment_attachment() - Equipment that becomes creature or has no valid target becomes unattached

4. **CR 704.5p - Counter Annihilation**: ✅ Already handled in Card::add_counter() - +1/+1 and -1/-1 counters cancel automatically

### Implementation Details

- Added `check_aura_attachment()` to state.rs (CR 704.5d)
- Added `check_equipment_attachment()` to state.rs (CR 704.5n)  
- Both functions called from priority loop after spell/ability resolution
- Added 3 unit tests for aura and equipment SBA scenarios

## Status

CLOSED - All missing SBAs now implemented and tested
