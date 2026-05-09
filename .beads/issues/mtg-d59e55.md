---
title: Migrate interactive_controller / rich_input_controller blocker menus to combat_rules::can_block
status: open
priority: 3
issue_type: task
created_at: 2026-05-09T15:55:06.420240832+00:00
updated_at: 2026-05-09T15:55:06.420240832+00:00
---

# Description

The native CLI interactive_controller (mtg-engine/src/game/interactive_controller.rs)
and rich_input_controller still build ad-hoc blocker selection menus that
do not call combat_rules::can_block. They can present illegal blocker
options, mirroring the WASM bug fixed under mtg-426cf0.

Action: refactor both controllers' choose_blockers implementations to
filter (blocker, attacker) pairs through combat_rules::can_block so the
engine never silently drops a user-picked block.

Related: mtg-426cf0 (root-cause fix for WASM GUI).
