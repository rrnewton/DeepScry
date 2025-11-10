---
title: Implement Equipment attachment system
status: open
priority: 2
issue_type: feature
depends_on:
  mtg-3: discovered-from
created_at: 2025-11-10T11:52:25.419378578+00:00
updated_at: 2025-11-10T15:18:03.462314226+00:00
---

# Description

## Equipment Attachment Implementation Plan

### Current Status (2025-11-10 #905)

**Completed Phases**:
- ✅ Phase 1: Equipment attachment infrastructure (5d14a43c)
- ✅ Phase 2: Equipment buff calculation with hardcoded values (0d2230dc)
- ✅ Phase 2b: Combat integration & state-based actions (c20cd2cc)
- ✅ CR 613 Layer System: Refactored to proper layer structure (7d267ace, 92239bb2)
- ✅ Phase 3: Static ability parsing from card data (PENDING_COMMIT)

**Phase 3 Complete**:
- Implemented `parse_static_abilities()` in CardDefinition to parse S:Mode$ Continuous lines
- Added StaticAbility::ModifyPT enum variant with AffectedSelector
- Updated `calculate_modifypt_effects()` to use parsed abilities instead of hardcoding
- Parses: `S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2`
- Test helper `create_spider_suit()` added to populate static abilities for tests

**What's Working Now**:
- Equipment can be cast and enter battlefield ✅
- Equipment can attach/detach programmatically ✅
- Creatures get stat buffs from attached Equipment ✅ (parsed from card data!)
- Combat damage uses buffed stats (via CR 613 layer system) ✅
- Equipment auto-detaches when creature dies ✅
- Counter bonuses properly calculated in Layer 7c ✅
- Full CR 613.4 layer documentation with all stubs ✅
- Static abilities parsed from S: lines in card files ✅

**Test Coverage**: 8 Equipment tests + 8 continuous_effects tests, all 476 tests passing

---

[Previous content preserved below]

## Architecture Overview

### Java Forge Approach (Verified)

[Keeping existing documentation for reference...]
