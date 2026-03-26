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

**Current status as of 2026-03-26_#1995(fff908a7):**
- Tests: 942 unit/integration tests passing
- All 55+ determinism tests passing
- Network multiplayer: Full WebSocket support with deterministic sync
- Performance: 7.44M actions/sec (simple_bolt), 2.40M (mem_logging)
- Random deck compatibility: 100% success rate (200 games, 0 engine errors)
- Keyword warnings: 338 remaining (down from 2672 original)
- AI casts 18+ effect types automatically, counters board wipes/extra turns/steals

**Recent accomplishments (2026-03-25 to 2026-03-26):**
- AI: Counter board wipes, extra turns, steal effects (CounterAi parity)
- TapOrUntap effect (49 cards) - tap/untap target permanent
- MultiplyCounter effect (44 cards) - counter doubling
- AI: Undying/Persist counter-state awareness (Java parity)
- AI: Surveil/Loot/Dig always-beneficial + SacrificeAll board wipe routing
- AI: Mill, GainLife, PumpAllCreatures, MultiplyCounter, PutCounter always-beneficial
- Optimization: trigger boolean flags -14.6%, empty pool fast-path -2.9%, SmallVec SBA -1.9%

**Previous accomplishments (2026-03-14 to 2026-03-21):**
- AddTurn effect (64 cards), SacrificeAll (143), ChangeZoneAll (636), PutCounterAll (264)
- AI: PutCounterAll, ChangeZoneAll, equipment/counter evaluation, bluffing
- Fixes: Aura non-creature enchanting, sacrifice cost infinite loop, CounterSpell/DealDamage fizzle
- Trigger flags refactor, 15+ keyword variants, warnings 706→338
- Optimization: check_triggers Vec (-3-5%), sort_unstable + SBA guard (-5-7.5%)

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
Checked up-to-date as of 2026-03-26_#1995(fff908a7).
