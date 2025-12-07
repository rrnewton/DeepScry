---
title: 'Cross-cutting codebase issues: APIs, testing, architecture'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-07T20:10:32.780174422+00:00
---

# Description

Track architectural improvements, API design, and testing infrastructure.

**Controller architecture:**
- Current: Unified PlayerController trait (documented in ai_docs/CONTROLLER_DESIGN.md)
  - Single `choose_spell_ability_to_play()` method for lands, spells, and abilities
  - GameStateView provides read-only access with zero-copy patterns
  - Callback-based casting with proper mana timing (step 6 of 8)
- Implementations:
  - RandomController: Random decisions with seeded RNG
  - ZeroController: Always chooses first option (deterministic)
  - HeuristicController: Evaluation-based AI (faithful Java port)
  - FixedScriptController: Script-based decisions for testing
  - InteractiveController: Human player via stdin/stdout
- mtg-40: Migrate game loop from v1 to v2 controller interface (OBSOLETE - already unified)
- mtg-41: Controller API consistency and documentation
- mtg-144: Missing player choices (mulligan, activated abilities)

**UI/TUI Event Architecture:**
- Current: Partially shared event handling between native and WASM
  - Mouse clicks: shared via handle_mouse_click() in fancy_tui_events.rs
  - Keyboard: shared KeyInput enum, but backend-specific handling
  - Event-driven rendering: window.onRenderComplete callback (not polling!)
  - Geometry queries: tui_get_card_positions() exports layout data
- TODO: Extract common InputEventHandler abstraction for full sharing
- TODO: Animation/transition support (future work)
- See ai_docs/UI_ARCHITECTURE.md for complete documentation

**Testing infrastructure:**
- Current: 405 passing tests (nextest, all categories)
- mtg-42: Improve test coverage for edge cases
- mtg-43: Integration test suite expansion
- mtg-44: Determinism testing for more complex scenarios
- mtg-45: Property-based testing with proptest

**Performance & Tree Search (Phase 4):**
- mtg-46: Undo/redo performance testing
- mtg-47: Board state evaluation function
- mtg-48: Tree search using undo log
- mtg-49: MCTS or minimax search implementation
- mtg-50: Measure boardstates-per-second

**Serialization:**
- mtg-51: Fast binary game snapshots (rkyv)
- mtg-52: Parallel game search capabilities
- mtg-53: SIMD optimizations where applicable

---
**Checked up-to-date as of 2025-12-07_#1144**
- Added UI/TUI Event Architecture section
- Documented event-driven rendering callback (no polling!)
- Noted TODO for InputEventHandler abstraction
- Referenced ai_docs/UI_ARCHITECTURE.md
- Previous updates from 2025-11-04:
  - Verified controller architecture (5 implementations, unified interface)
  - Updated test count: 360 → 405 tests
  - Marked mtg-40 as obsolete (v1/v2 already unified)
  - Verified file locations (ai_docs/CONTROLLER_DESIGN.md exists)
  - All controller implementations working correctly
  - Added mtg-144: Missing player choices (mulligan + activated abilities)
