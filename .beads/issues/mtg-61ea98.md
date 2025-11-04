---
title: Optimize GameState clone for parallel MCTS simulations
status: open
priority: 3
issue_type: task
created_at: 2025-11-04T21:44:24.580096373+00:00
updated_at: 2025-11-04T21:44:24.580096373+00:00
---

# Description

Problem: GameState cloning costs 15-20KB per clone (Cards 8KB + Undo log 10KB + other 2KB), creating cache pressure in parallel code. With 8 threads, 120-160KB cloned per iteration. Solution: Create clone_for_simulation() that skips undo_log and logger. Estimated savings: 60% reduction (15-20KB to 5-8KB). This should improve parallel efficiency from 47.4% toward 70-80%. See ai_docs/parallel_contention_analysis.md. Related: mtg-a6ca26, mtg-2
