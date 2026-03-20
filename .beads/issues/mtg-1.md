---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-12T02:07:15.887412443+00:00
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

**Current status as of 2026-03-14_#1953(42ab0056):**
- Tests: 942 unit/integration tests passing
- All 55+ determinism tests passing
- Network multiplayer: Full WebSocket support with deterministic sync
- Performance: 7.0M actions/sec (simple_bolt benchmark)
- Keyword warnings: 338 remaining (down from 706)

**Recent accomplishments (2026-03-14):**
- SacrificeAll effect (143 cards) - mass sacrifice (All is Dust, Archfiend of Depravity)
- ChangeZoneAll effect (636 cards) - mass zone changes (Aetherize, Tormod's Crypt)
- PutCounterAll effect (264 cards) - mass counter placement (Ajani, Arcbound Overseer)
- PutCounterAll AI evaluation (CountersPutAllAi port)
- 15 new keyword variants: combat restrictions, damage prevention, alternate costs
- Keyword warnings reduced from 706 → 346 (51% reduction)
- AI: Land drop bluffing + instant-speed spell timing bluffing
- Fix: Sacrifice cost checking prevents infinite loop on activated abilities
- Fix: Aura enchanting now supports Land, Artifact, Enchantment, Permanent targets
- Optimization: check_triggers Vec elimination (-3-5%), sort_unstable + SBA guard (-5-7.5%)
- Optimization: check_phase_triggers String allocation elimination (-3.2%)

**Previous accomplishments (2026-03 early):**
- DealsCombatDamage trigger firing at runtime
- AB$ GainControl (steal target creature) effect
- Removal timing AI, bounds_check_payment optimization (+6.6%)
- MtgError boxing optimization (+10%)

**Conventions:**
- Tracking issues (priority 1) reference granular issues
- Granular issues have priority 3-4 unless critical bugs (priority 2)
- Human-created issues have priority 0
- Reference issues in code: // TODO(mtg-N): description
- Transient info includes timestamp: YYYY-MM-DD_#depth(hash)

---
Checked up-to-date as of 2026-03-14_#1953(42ab0056).
