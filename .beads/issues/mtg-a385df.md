---
title: Sandbenders' Storm Earthbend mode resolves without targeting a land
status: closed
priority: 3
issue_type: task
labels:
- bug
- earthbend
created_at: 2026-04-20T19:59:02.876778856+00:00
updated_at: 2026-05-12T13:57:46.439946992+00:00
closed_at: 2026-05-12T13:57:46.439946912+00:00
---

# Description

## Summary

Sandbenders' Storm with Earthbend 3 mode resolves without targeting a land — no creature is created from earthbend. The spell just resolves with no visible effect.

## Reproduction

Observed in game.html human play, gabriel_avatar_draft vs eric_avatar_draft decks.

## Root Cause

In `targeting.rs`, `get_valid_targets_for_spell()` listed `Effect::Earthbend { .. }` in the "no targeting requirements" catch-all (line 561). This meant when the Charm mode was applied and Earthbend became the spell's effect, the targeting pass returned no targets. The Earthbend target stayed as placeholder (CardId 0), and at resolution (mod.rs:3557) the placeholder check silently returned Ok(()) — effectively fizzling.

The same bug existed in `effect_has_valid_targets()` (line 1266), where Earthbend was always reported as having valid targets (true), bypassing the check for whether the player actually controls a land.

Note: `get_valid_targets_for_ability()` already had the correct Earthbend handler (line 960) for activated/triggered abilities (e.g., Ba Sing Se, Avatar Kyoshi's triggered earthbend). Only SPELL-based earthbend (Sandbenders' Storm, Cracked Earth Technique) was broken.

## Fix

1. Added `Effect::Earthbend { target, .. } if target.is_placeholder()` handler in `get_valid_targets_for_spell()` — targets lands you control (matching the ability handler)
2. Added same handler in `effect_has_valid_targets()` — checks if player controls any land
3. Moved `Earthbend { .. }` from "no targeting" catch-all to "target already specified" catch-all in both functions
4. Added unit test `test_modal_spell_earthbend_mode_targets_land` verifying the complete flow

## Files Changed

- `mtg-engine/src/game/actions/targeting.rs` — 4 edits
- `mtg-engine/src/game/actions/tests/spell_casting.rs` — new test
