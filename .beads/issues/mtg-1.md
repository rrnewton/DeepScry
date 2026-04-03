---
title: Overall MTG Forge Rust development tracking
status: open
priority: 0
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-04-03T15:18:13.625816317+00:00
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
- mtg-4s1lq: Commander format support (FEATURE COMPLETE 2026-04-01)
- mtg-fmm68: Julian Avatar Deck Compatibility

**Current status as of 2026-04-03_#2060(79616d6b):**
- Tests: 924 unit/integration tests passing
- All 55+ determinism tests passing
- Network multiplayer: Full WebSocket support with deterministic sync
- Performance: 7.33M actions/sec (simple_bolt), 2.03M (mem_logging) at commit #2059
- Random deck compatibility: 100% success rate (200 games, 0 engine errors)
- Keyword warnings: 338 remaining (down from 2672 original)
- AI casts 18+ effect types, counters board wipes/extra turns/steals
- mtg-159 (Spiderman draft decks) CLOSED - 20/20 games flawless
- Commander format: fully supported with command zone, tax, planeswalker loyalty
- X-cost spell casting implemented (mtg-113)
- AB$ ChooseColor effect (30 cards, mtg-dxjtq CLOSED)

**Recent accomplishments (2026-04-01 to 2026-04-03):**
- feat: AB$ ChooseColor effect type (30 cards) - AI picks prominent color
- feat: X-cost spell casting (mtg-113) - proper X mana calculation
- Commander format FEATURE COMPLETE (35 commits, 14 bugs fixed)
  - Planeswalker loyalty system, commander tax, zone replacement
  - Token is_token field, ModalChoice for CastFromExile
  - Heuristic AI: CastFromCommand, planeswalker casting, mana rock priority

**Previous accomplishments (2026-03-25 to 2026-03-31):**
- Fix: Guard EntityStore::get() against sentinel CardId values (u32::MAX)
- Fix: Handle sentinel CardId + missing source zone gracefully
- Fix: Gracefully handle missing player zones in scry/surveil/discard
- AI: Keyword-granting and trigger enchantment casting (Levitation, Fervor)
- AI: Counter board wipes, extra turns, steal effects (CounterAi parity)
- TapOrUntap effect (49 cards), MultiplyCounter (44 cards)
- AI: Undying/Persist counter-state, Surveil/Loot/Dig, SacrificeAll board wipe
- Optimization: trigger boolean flags -14.6%, empty pool -2.9%, SmallVec SBA -1.9%

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
Checked up-to-date as of 2026-04-03_#2060(79616d6b).
