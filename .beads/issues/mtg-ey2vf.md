---
title: 'Refactor: two parallel undo implementations (GameAction::undo + GameState::undo) — DRY footgun'
status: open
priority: 3
issue_type: task
created_at: 2026-06-03T05:14:53.160703042+00:00
updated_at: 2026-06-03T05:14:53.160703042+00:00
---

# Description

DRY debt found while fixing mtg-ba6uq #1 (ETB choice-field undo holes).

There are TWO parallel reversal implementations for GameActions:
1. undo.rs `impl GameAction { fn undo(&self, game: &mut GameState) }` (the canonical reversal, used by rewind_to_turn_start's catch-all `_ => action.undo(game)`).
2. state.rs `GameState::undo(&mut self)` — a SECOND exhaustive `match action { ... }` that REIMPLEMENTS the reversal per-variant inline (per-action undo path: human undo / MCTS).

FOOTGUN: a new GameAction variant (or a fix to an existing one) must be handled in BOTH, or the two undo paths diverge silently. E.g. mtg-ba6uq #1 had to add SetChosenColor/SetChosenPlayer arms to undo.rs:691, state.rs:3283 (GameState::undo), AND the Display arm at undo.rs:449 — three sites for one logical reversal. The compiler catches MISSING variants (exhaustive match) but NOT a SUBTLY-DIFFERENT reversal between the two impls.

FIX: make GameState::undo delegate to GameAction::undo (single source of truth), or extract a shared reversal so there is exactly ONE per-variant reversal. Audit for any existing divergence between the two impls while consolidating (some arms may already differ — e.g. ModifyLife has_lost re-derivation, mana_version handling). Add a test that asserts the two paths produce byte-identical state for a representative action sequence.

Relates mtg-ba6uq (undo-log completeness), mtg-610 (netarch).
