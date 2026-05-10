---
title: 'GUI: Blocker choice menu offered illegal pairs (silently dropped)'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-09T15:54:58.957515666+00:00
updated_at: 2026-05-09T15:55:09.172845228+00:00
---

# Description

## Context (FIXED on branch fix-blockers, May 2026)

When the WASM Fancy TUI presented blocker assignments for a flying attacker
(e.g. Glider Kids 2/3 Flying), the choice menu offered every
(blocker, attacker) pair regardless of MTG legality. A user picking a
non-flying / non-reach blocker would have it silently dropped by the
engine's validate_blocking_restrictions, with no feedback. Glider Kids
would then deal damage directly to the player.

## Root cause

update_choices_from_context in mtg-engine/src/wasm/fancy_tui.rs built the
blocker choice list as the cartesian product of available_blockers x
attackers, without filtering by legality. The engine's
validate_blocking_restrictions then rejected the user's pick.

## Fix (commit on fix-blockers)

- Extracted the per-pair legality predicate into
  mtg-engine/src/game/combat_rules.rs::can_block. Both the GUI and the
  engine validator now call this single predicate (CR 509.1a, 702.9b,
  702.13, 702.16, 702.28, 702.31, 702.36, 702.119, plus CantBeBlocked).
  Landwalk has a can_block_with_view variant.
- WASM Fancy TUI now filters the displayed (blocker, attacker) pairs
  through can_block, so illegal options are never presented.
- Added multi-blocker staging: clicking a blocker pair stages it, the
  prompt shows accumulated assignments, and 'Done' submits all at once.
  Previously, a single click immediately committed to one blocker with
  no way to add more.
- Regression tests in mtg-engine/tests/blocker_legality_test.rs.

## Out of scope

- Menace (CR 702.111b): aggregate check, still in
  validate_blocking_restrictions.
- The native CLI interactive_controller and rich_input_controller still
  use their own ad-hoc menus. They should be migrated to call can_block
  too. Filed as follow-up.
