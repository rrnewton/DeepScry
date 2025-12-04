---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-04T14:40:28.637673311+00:00
---

# Description

This is the main tracking issue for MTG Forge Rust development.

**Major tracking issues (priority 1):**
- mtg-2: Optimization and performance tracking
- mtg-3: MTG feature completeness (keywords, abilities, effects)
- mtg-4: Gameplay features (TUI, human play, controls)
- mtg-5: Cross-cutting codebase issues (APIs, testing, architecture)
- mtg-77: Heuristic AI completeness tracking
- mtg-108: Complex mana source handling
- mtg-121: Fancy TUI enhancements and polish
- mtg-143: Missing player choice opportunities tracking
- mtg-147: Affected$ selector parsing improvements

**Current status as of 2025-12-04_#1134(28100f8):**
- Tests: 597 passing (nextest, all categories)
- Examples: 14/14 passing
- Cards: 31,545 loaded from cardsfolder
- Parser warnings: 1,694 (reduced from 2,672 - 37% improvement)

**Recent accomplishments (2025-12-04):**
- Fixed AddMana effect player placeholder resolution (Dark Ritual bug)
- Added variable P/T parsing (AddPower$/AddToughness$ X, Y, Z, Count$)
- Added EnchantedBy selectors for Artifact/Planeswalker/Equipment
- Parser warning reduction: 2,672 → 1,694 (-978 warnings)

**Previous accomplishments (2025-11-30 to 2025-12-03):**
- Card.Self+attacking selector for combat keywords
- Blocking restriction evasion abilities (Fear, Intimidate, Shadow, Skulk)
- Combat restriction penalties in AI evaluation
- Extended keyword evaluation in AI
- Trigger self-only fix (ETB triggers for Card.Self)
- Death triggers, upkeep triggers
- Counterspell AI, intelligent mana tapping
- Shadow/Horsemanship evasion keywords

**Conventions:**
- Tracking issues (priority 1) reference granular issues
- Granular issues have priority 3-4 unless critical bugs (priority 2)
- Human-created issues have priority 0
- Reference issues in code: // TODO(mtg-N): description
- Transient info includes timestamp: YYYY-MM-DD_#depth(hash)

---
**Last updated: 2025-12-04_#1134(28100f8)**
