---
title: 'Puzzle inline assertion DSL: phase 1 (final-state assertions in [assertions] section)'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-13T18:02:55.030872108+00:00
updated_at: 2026-06-13T18:55:30.694810005+00:00
---

# Description

Track the puzzle-DSL effort: phase 1 implements inline `[assertions]` parsing and evaluation (final-state queries only) behind the `puzzle-assert` cargo feature. See ai_docs/reference/PUZZLE_ASSERTION_DSL.md for the design doc.

## Scope
Phase 1:
- [x] `[assertions]` section parser -> Vec<Assertion> AST (strong types, reuse CardModifier vocabulary)
- [x] Final-state assertions: life totals, zone contents (hand/graveyard/battlefield/exile/library-top-N), game result, turn number
- [x] Negation (NOT prefix) and player scoping (me / opponent)
- [x] Evaluator wired into puzzle runner (feature-gated, no engine overhead when off)
- [x] Unit tests for parser + evaluator
- [x] Integration: demo puzzle files, wired into make validate
- [x] Zero-overhead proof: engine builds clean with puzzle-assert OFF

## Completion
- Branch: claude/puzzle-format-phase1 (pushed to origin)
- Final SHA: 9ed749cf84964a68981fb09ca3a5cb3a00d4379c
- Validate log: validate_logs/validate_9ed749cf84964a68981fb09ca3a5cb3a00d4379c.log
- All 17 steps passed (1831 unit+integration tests, 108 pytest, clippy clean, fmt clean)
- Validate ran with --no-wasm-e2e (wasm.npm-install pre-existing infra gap in WSL)

## Later phases (NOT in scope here)
- Log-derived (event) assertions: blocked on structured game log (log entries are string messages only; substring matching violates NO HACKY STRING OPERATIONS rule). Requires adding a structured GameEvent enum to the logger first - tracked separately.
- Golden game-log oracle + one-command re-bless
- Bulk parallel puzzle runner
- Rewind-determinism mode
- Migration of existing 668 .pzl external Rust assertions into the DSL
