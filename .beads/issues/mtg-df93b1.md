---
title: Puzzle file support for controlled starting states
status: open
priority: 1
issue_type: task
created_at: 2026-04-04T01:50:09.411018441+00:00
updated_at: 2026-04-04T01:50:09.411018441+00:00
---

# Description

Files: agentplay/agent_game.py

Action: Add --puzzle flag to start from a puzzle file instead of a normal game:
1. Accept .pzl file path
2. Pass it to the MTG engine (mtg puzzle <file> or similar)
3. The puzzle sets up hands, battlefields, libraries in a desired state
4. Agent plays from that state forward
5. Combined with --max-turns and --goal (Phase 2), enables targeted bug reproduction

The puzzle format is INI-like with [metadata] and [state] sections.
Check how puzzles are invoked via CLI: look at Commands::Puzzle in main.rs.

Verify:
- --puzzle test_puzzles/some_puzzle.pzl starts correctly
- Agent can make choices from puzzle starting state
- Works with both agent and heuristic controllers
