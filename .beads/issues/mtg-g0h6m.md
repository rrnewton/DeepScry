---
title: 'DRY: repeated casting-context setup in push_castable_* (engine-cleanup wave2)'
status: open
priority: 4
issue_type: task
created_at: 2026-06-13T16:22:06.683390212+00:00
updated_at: 2026-06-13T16:22:06.683390212+00:00
---

# Description

Engine-cleanup wave2 (branch claude/engine-cleanup-wave2): the five push_castable_* functions in game_loop/actions.rs each had an identical 10-15 line setup block (mana_engine.update_mut, mana_pool read, is_active_player, is_sorcery_speed, stack_is_empty) and a repeated 4-line can_cast_now timing check. Factored into CastingContext struct + make_casting_context() helper + can_cast_now() method. Applied to push_castable_spells, push_castable_from_exile, push_castable_from_command, push_castable_with_fires, push_castable_from_library. Landed on integration via wave2 branch.
