---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-10T00:58:09.001410482+00:00
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
- mtg-0iad2: Ryan Avatar Deck compatibility testing
- mtg-5hvly: Gabriel Avatar Deck compatibility testing

**Current status as of 2026-03-10_#1894:**
- Tests: 918 unit/integration tests passing
- All 53 determinism tests passing
- Network multiplayer: Full WebSocket support with deterministic sync
- Performance: 5.6M actions/sec (simple_bolt benchmark)

**Recent accomplishments (2026-03):**
- AB$ GainControl (steal target creature) effect
- Removal timing AI for smart spell usage
- bounds_check_payment optimization (+6.6% perf)
- MtgError boxing optimization (+10% perf)
- Network reveal logic centralization (desync prevention)

**Previous accomplishments (2026-01-17):**
- PlayerTurn$ True parsing for activated abilities
- Fixed Waterbend cost affordability/payment
- SMART multi-blocker damage assignment
- Combat death and blocker logging improvements

**Conventions:**
- Tracking issues (priority 1) reference granular issues
- Granular issues have priority 3-4 unless critical bugs (priority 2)
- Human-created issues have priority 0
- Reference issues in code: // TODO(mtg-N): description
- Transient info includes timestamp: YYYY-MM-DD_#depth(hash)

---
Checked up-to-date as of 2026-03-10.
