---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-01-03T03:49:30.783877618+00:00
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
- mtg-6n8rl: Avatar set mechanics (Waterbend, Airbend) support

**Current status as of 2026-01-03_#1477(54b4e2c):**
- Tests: 732 passing (nextest, all categories)
- Parser warnings: 772 (71% reduction from initial 2,672)
- Dynamic Affected$ selector parsing expanded
- Avatar deck games complete without errors (200+ seeds tested)

**Recent accomplishments (2026-01-03_#1477):**
- Treasure tokens selector: Card.Treasure+YouCtrl
- wasCast state selector: Card.YouCtrl+wasCast
- Self TopLibrary: Card.Self+TopLibrary
- Color-based spell selectors: Instant.COLOR+YouCtrl, Sorcery.COLOR+YouCtrl
- Top of library subtype: Card.TopLibrary+YouCtrl+SUBTYPE
- Parser warning reduction: 792 → 772 (-20)

**Previous accomplishments (2026-01-03_#1475):**
- Dynamic subtype.YouOwn parsing (Merfolk.YouOwn, Druid.YouOwn, etc.)
- CardType.TopLibrary+YouCtrl patterns (Instant, Sorcery top of library)
- Permanent.Subtype+YouCtrl patterns (Servo, Thopter buffs)
- Card.EquippedBy+TYPE patterns (Human, Angel equipment bonuses)
- Parser warning reduction: 854 → 792 (-62)

**Earlier accomplishments (2026-01-02):**
- Fixed Ba Sing Se (non-basic land) mana production
- Fixed Foggy Swamp Vinebender incorrectly marked as mana source
- CardTypeYouOwn/SubtypeYouOwn selectors for graveyard casting
- Don't offer unimplemented instants/sorceries as castable

**Conventions:**
- Tracking issues (priority 1) reference granular issues
- Granular issues have priority 3-4 unless critical bugs (priority 2)
- Human-created issues have priority 0
- Reference issues in code: // TODO(mtg-N): description
- Transient info includes timestamp: YYYY-MM-DD_#depth(hash)

Checked up-to-date as of 2026-01-03.
