---
title: 'Puzzle PlayerStateDefinition.mana_pool: parse into typed mana symbols instead of Vec<String>'
status: open
priority: 4
issue_type: task
created_at: 2026-06-05T12:51:44.048585277+00:00
updated_at: 2026-06-05T12:51:44.048585277+00:00
---

# Description

PlayerStateDefinition.mana_pool (and persistent_mana) in mtg-engine/src/puzzle/state.rs are typed Vec<String>, a placeholder from the initial puzzle-loader implementation ('Simplified for Phase 1, will parse properly later'). They should parse into the engine's typed mana-symbol representation rather than raw strings, per the project strong-typing convention (CLAUDE.md: 'PREFER STRONG TYPES ... String makes it very unclear which values are legal'). Low priority; replaces the bare 'Phase 1' work-reference that previously sat in the code comment with a durable issue anchor.
