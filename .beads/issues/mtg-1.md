---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-08T14:57:29.974399272+00:00
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
- mtg-hcahb: Web GUI implementation with shared TUI/GUI architecture
- mtg-m7v83: Upstream Java Forge card script issues (PRs to upstream)

**Current status as of 2025-12-04_#1141(00682bc):**
- Tests: 597 passing (nextest, all categories)
- Examples: 14/14 passing
- Cards: 31,545 loaded from cardsfolder
- Parser warnings: 1,488 (reduced from 2,672 - 44% improvement)

**Recent accomplishments (2025-12-04):**
- AffectedSelector::Any for comma-separated OR conditions (dbce929)
- State-based selectors: SelfWhenUntapped, SelfWhenMonstrous (66cc504)
- Card.AttachedBy and Land.YouOwn selectors (cb54e35)
- SmallVec optimization for spell_targets (+1.6% throughput)
- Parser warning reduction: 2,672 → 1,488 (-1184 warnings, 44%)

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
