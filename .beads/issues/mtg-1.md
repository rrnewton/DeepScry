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

**Current status as of 2025-11-30_#1036(34a3747):**
- Tests: 541 passing (nextest, all categories)
- Examples: 14/14 passing
- Cards: 31k+ supported from cardsfolder
- Recent work: Integrated prompt_table01 and bugfix-01 branches into main

**Recent accomplishments (2025-11-30):**
- Integrated two feature branches into main:
  - prompt_table01 (17 commits): mana fixes, triggers, AI improvements, optimizations
  - bugfix-01 (9 commits): logging infrastructure, debug tools, SearchLibrary fix
- Key features merged:
  - Death triggers (Su-Chi "dies" ability)
  - Upkeep trigger effects (Juzám Djinn fix)
  - Counterspell AI for heuristic controller
  - Intelligent mana tapping order (mtg-77)
  - Land subtype caching optimization
  - Shadow/Horsemanship evasion keywords
  - Standard Rust logging infrastructure (log + env_logger)
  - FancyFixed controller for scripted TUI debugging
  - SearchLibrary player interaction fix
- Archived branches as tags: prompt_table01.v1, bugfix-01.v1

**Conventions:**
- Tracking issues (priority 1) reference granular issues
- Granular issues have priority 3-4 unless critical bugs (priority 2)
- Human-created issues have priority 0
- Reference issues in code: // TODO(mtg-N): description
- Transient info includes timestamp: YYYY-MM-DD_#depth(hash)

---
**Last updated: 2025-11-30_#1036(34a3747)**
