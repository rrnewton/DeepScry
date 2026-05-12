---
title: Add BUG_REPORT section to agent prompts and exit-on-bug logic
status: closed
priority: 1
issue_type: task
created_at: 2026-04-04T02:16:36.519241187+00:00
updated_at: 2026-05-12T13:58:19.019063898+00:00
closed_at: 2026-05-12T13:58:19.019063827+00:00
---

# Description

Files: agentplay/agent_game.py (modify)

Action: Extend agent_game.py prompt to instruct the agent to:
1. Give a choice (existing)
2. Give an explanation (existing)
3. Print a BUG_REPORT section when the game engine deviates from MTG rules
4. Exit the game on first BUG_REPORT unless --continue-past-bug-reports flag
5. Log BUG_REPORT sections to a separate file for easy review

Why: Enables opportunistic bug finding during any agent game session.

Verify: Agent prompts include BUG_REPORT instructions; game exits on bug report; --continue-past-bug-reports works.
