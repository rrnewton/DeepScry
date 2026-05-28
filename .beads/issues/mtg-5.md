---
title: 'Cross-cutting codebase issues: APIs, testing, architecture'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-05-28T13:30:46.309000659+00:00
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
