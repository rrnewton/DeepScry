---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-03T19:55:51.071433791+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

## Latest Optimization (2025-12-03_#1117(95ac82cf))

✅ **Eliminate Vec<Effect> clone in resolve_spell** - **2.5% allocation reduction, 5.5% fewer blocks**
- Changed resolve_spell to use index-based iteration instead of cloning entire Vec<Effect>
- Added resolve_effect_target helper for inline target resolution
- DHAT: 22,784 fewer bytes (928,374 → 905,590), 712 fewer blocks (12,936 → 12,224)
- Vec<Effect> clone hotspot (was #13 at 1.5%) no longer appears in top 20

## Previous Optimization (2025-12-01_#1102(70b3a07))

✅ **Eliminate Vec<Effect> clone in get_valid_targets_for_spell** - **2.4% allocation reduction, ~0.9% speedup**
- Use index-based iteration instead of cloning effects vector
- Extract primitives upfront, re-fetch effects[i] each iteration
- DHAT: 22,784 fewer bytes, 712 fewer blocks (5.2% reduction)
- Benchmark: -0.87% execution time (p=0.04, statistically significant)
