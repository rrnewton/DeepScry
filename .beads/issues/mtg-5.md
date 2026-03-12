---
title: 'Cross-cutting codebase issues: APIs, testing, architecture'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-10T00:57:44.149159574+00:00
---

# Description

Track architectural improvements, API design, and testing infrastructure.

**Testing status as of 2026-03-12_#1920(aa94be5):**
- Current: 938 passing tests (nextest, all categories)
- Includes 55 determinism tests across all major decks

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
  - NetworkController: Remote player over WebSocket (for web GUI)
- mtg-40: Migrate game loop from v1 to v2 controller interface (OBSOLETE - already unified)
- mtg-41: Controller API consistency and documentation
- mtg-144: Missing player choices (mulligan, activated abilities)

**Network Architecture (2026-03):**
- Deterministic sequential simulation model (see docs/NETWORK_ARCHITECTURE.md)
- Server authoritative, clients maintain shadow state
- Zero-desync tolerance: any mismatch is FATAL
- Validation layer in NetworkController checks all moves
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
- mtg-50: Measure boardstates-per-second - tracked in mtg-2 (5.6M actions/sec)

**Serialization:**
- mtg-51: Fast binary game snapshots (rkyv)
- mtg-52: Parallel game search capabilities
- mtg-53: SIMD optimizations where applicable

---
Checked up-to-date as of 2026-03-12_#1920(aa94be5).
