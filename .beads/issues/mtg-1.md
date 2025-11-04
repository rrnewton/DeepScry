---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-04T16:30:24.255545403+00:00
---

# Description

This is the main tracking issue for MTG Forge Rust development.

**Major tracking issues (priority 1):**
- mtg-2: Optimization and performance tracking
- mtg-3: MTG feature completeness (keywords, abilities, effects)
- mtg-4: Gameplay features (TUI, human play, controls)
- mtg-5: Cross-cutting codebase issues (APIs, testing, architecture)
- mtg-77: Heuristic AI completeness tracking
- mtg-108: Complex mana source handling (Phase 5 in progress)
- mtg-121: Fancy TUI enhancements and polish
- mtg-143: Missing player choice opportunities tracking

**Current status as of 2025-11-04_#686(7edf8de):**
- Tests: 406 passing (nextest, all categories)
- Examples: 14/14 passing
- Performance: ~3,842 games/sec (fresh mode), 16.56 actions/turn
- Performance: ~9,177 games/sec (snapshot mode), ~332k rewinds/sec (rewind mode)
- Cards: 31k+ supported from cardsfolder
- Recent work: Mana payment refactoring (-114 LOC), closed mtg-101, mtg-118, duplicate issues

**Recent accomplishments (2025-11-04):**
- Refactored mana payment system: eliminated 180+ lines of duplicate code
- Closed mtg-118: Old School tournament errors (100% fixed, 0.0% error rate)
- Closed mtg-101: monored.dck fully playable
- Closed 6 duplicate/stale issues (mtg-93, mtg-94, mtg-115, mtg-116, mtg-117, mtg-119)

**Conventions:**
- Tracking issues (priority 1) reference granular issues
- Granular issues have priority 3-4 unless critical bugs (priority 2)
- Human-created issues have priority 0
- Reference issues in code: // TODO(mtg-N): description
- Transient info includes timestamp: YYYY-MM-DD_#depth(hash)

---
**Last updated: 2025-11-04_#686(7edf8de)**
