---
title: 'Fancy TUI: Add library count to status bar'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T16:34:45.638090976+00:00
updated_at: 2025-11-03T16:46:54.239664899+00:00
---

# Description

Part of: mtg-dba689

Add library card count to the player info status bars.

Current display:
```
You: 20 life | Hand: 7 | GY: 0
```

Should become:
```
You: 20 life | Hand: 7 | GY: 0 | Lib: 53
```

## Implementation

- Modify `draw_player_info` method in `fancy_tui_controller.rs`
- Use `view.player_library(player_id).len()` to get library count
- Update format string to include library count

Related to turn counter/phase indicator work (will share the same status line).
