---
title: 'Cross-cutting codebase issues: APIs, testing, architecture'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-06-13T21:03:23.418408785+00:00
---

# Description

Track architectural improvements, API design, and testing infrastructure.


**Testing status as of 2026-04-03_#2060(79616d6b):**
- Current: 924 passing tests (nextest, all categories)
- Includes 55+ determinism tests across all major decks
- Performance: 7.33M actions/sec (simple_bolt), 2.03M (mem_logging) at commit #2059
- Random deck compatibility: 100% success rate (200 games, 0 engine errors)


**Controller architecture:**
- Core: Unified PlayerController trait (defined in game/controller.rs:1214, documented in ai_docs/CONTROLLER_DESIGN.md)
  - Single `choose_spell_ability_to_play()` method for lands, spells, and abilities
  - GameStateView provides read-only access with zero-copy patterns
  - Callback-based casting with proper mana timing (step 6 of 8)
- 13 controller implementations across 3 subsystems:
  - Core (game/): ZeroController, RandomController, HeuristicController, FixedScriptController, ReplayController, InteractiveController, RichInputController, FancyTuiController, FancyFixedController
  - Network (network/): NetworkLocalController (generic wrapper), RemoteController (client-side opponent)
  - WASM (wasm/): WasmHumanController, WasmRichInputController
- ControllerType enum (game/snapshot.rs:30): Zero, Random, Tui, Heuristic, Fixed, FancyFixed, Remote, Network
- mtg-144: Missing player choices (mulligan, activated abilities)


**Game format support:**
- Standard 60-card constructed: fully supported
- Commander (EDH): fully supported (mtg-274 FEATURE COMPLETE 2026-04-01)
  - Command zone, commander tax, zone replacement, commander damage tracking
  - Planeswalker loyalty system: costs, counters, 0-loyalty death SBA, once-per-turn rule


**Network Architecture (2026-03):**
- Deterministic sequential simulation model (see docs/NETWORK_ARCHITECTURE.md)
- Server authoritative, clients maintain shadow state
- Zero-desync tolerance: any mismatch is FATAL
- Validation layer in NetworkLocalController checks all moves
- RevealCard mechanism for hidden information disclosure


**UI/TUI Event Architecture:**
- Current: Fully shared event handling between native and WASM (fancy_tui_events.rs)
  - Mouse clicks: handle_mouse_click() with card position detection
  - Keyboard: KeyInput enum with unified handling
  - Event-driven rendering: window.onRenderComplete callback (not polling!)
  - Geometry queries: tui_get_card_positions() exports layout data
- See ai_docs/UI_ARCHITECTURE.md for complete documentation


**Testing infrastructure:**
- mtg-42: Improve test coverage for edge cases
- mtg-43: Integration test suite expansion
- mtg-44: Determinism testing - now comprehensive (55 deck tests)
- mtg-45: Property-based testing with proptest


**Performance & Tree Search (Phase 4):**
- mtg-46: Undo/redo performance testing
- mtg-47: Board state evaluation function - IMPLEMENTED (GameStateEvaluator)
- mtg-48: Tree search using undo log
- mtg-49: MCTS or minimax search implementation
- mtg-50: Measure boardstates-per-second - tracked in mtg-2


**Serialization:**
- mtg-51: Fast binary game snapshots (rkyv)
- mtg-52: Parallel game search capabilities
- mtg-53: SIMD optimizations where applicable


---
Checked up-to-date as of 2026-04-03_#2060(79616d6b).

**Engine cleanup wave series (DRY / modular refactors, 2026-06):**
- Wave 1 (landed @engine-cleanup-wave1): heuristic_controller split into per-module files
- Wave 2 (landed @engine-cleanup-wave2): CastingContext DRY in game_loop/actions.rs + TODO linking
- Wave 3 (landed @engine-cleanup-wave3): consume_next_target helper in actions/mod.rs (also fixed Power Sink)
- Wave 4 (branch claude/engine-cleanup-wave4, commit 18f29f02): extract apply_pump_bonus_and_log shared helper in effects/pump.rs; fixes variable-pump missing gamelog (Berserk was silent)
**Engine-cleanup waves (structural refactor, zero behavior change):**
- wave1: pump.rs split / DRY (landed integration, pre-wave)
- wave2: CastingContext extraction from push_castable_* (mtg-g0h6m, landed integration @2026-05-xx)
- wave3: consume_next_target helper extraction, resolve_effect_target DRY (landed integration @2d1d6af)
- wave4: (see wave3 branch for details)
- wave5 (2026-06-13_#3391(eef8eab22)): effects.rs 6280→3788 lines split into 4 submodules
  (effects/mod.rs + triggers.rs + static_abilities.rs + activated_ability.rs);
  CopyPermanent.add_types Vec<String> → add_subtypes SmallVec<[Subtype; 2]> (strong type);
  infra issues filed: mtg-2b0d7 (Playwright XDG isolation), mtg-9ohle (WASM RefCell borrow).
  Branch claude/engine-cleanup-wave5, validate 35/35.

---
Checked up-to-date as of 2026-04-03_#2060(79616d6b) (structure); wave5 appended 2026-06-13_#3391(eef8eab22).

- wave6 (2026-06-13_#3402(125e3c570)): DRY card-type filters + strengthen Effect::Balance types.
  (1) Card::has_card_type_str added to core/card.rs; routed 8+ duplicate match blocks across
  game/actions/mod.rs, effects/tapping.rs, effects/zones.rs, game_loop/actions.rs, priority.rs.
  (2) Effect::Balance::card_type String → Option<CardType>; zone String → Zone enum.
  Branch claude/engine-cleanup-wave6. 1228 tests pass. Pure refactor, no gameplay change.
