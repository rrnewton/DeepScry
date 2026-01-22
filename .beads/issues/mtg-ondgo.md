---
title: Network heuristic game state divergence
status: open
priority: 3
issue_type: task
created_at: 2026-01-22T18:48:10.009585292+00:00
updated_at: 2026-01-22T18:48:10.009585292+00:00
---

# Description

## Network State Divergence with Heuristic Controllers

**Problem**: With heuristic AI controllers, network games produce different gamelogs than local games, even with identical seeds. The shadow game state in network clients diverges from the server, causing the heuristic AI to make different decisions.

**Status**: 2026-01-22_#1767 - Actively being investigated

**Symptoms**:
- zero controller games: **PASS** (identical gamelogs)
- heuristic controller games: **FAIL** (divergent gamelogs)
- No explicit errors, games complete, but outcomes differ

**Evidence** (2026-01-22):
```
./tests/network_vs_local_equivalence_e2e.sh 3 heuristic

Local: 18 turns, 89 gamelog entries
Network: 16 turns, 75 gamelog entries
First divergence at Turn 13:
  Local:   The Boulder, Ready to Rumble (72) dies
  Network: White Lotus Reinforcements (73) dies
```

**Root Cause Analysis**:
The heuristic AI evaluates game state to make decisions. If the client's shadow game state differs from the local game state (which mirrors the server), the AI will make different choices → different game outcomes.

**Possible causes**:
1. Shadow game state not properly synchronized after certain operations
2. Card abilities or triggered effects updating state differently
3. RNG state divergence between local and network modes
4. Information about opponent's hidden zones differs

**Related Issues**:
- mtg-a33hf (closed): Library search state divergence - added RevealReason::Searched handling
