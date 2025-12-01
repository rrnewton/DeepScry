---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-01T22:14:30.959912386+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

## Latest Optimization (2025-12-01_#1102(70b3a07))

✅ **Eliminate Vec<Effect> clone in get_valid_targets_for_spell** - **2.4% allocation reduction, ~0.9% speedup**
- Use index-based iteration instead of cloning effects vector
- Extract primitives upfront, re-fetch effects[i] each iteration
- DHAT: 22,784 fewer bytes, 712 fewer blocks (5.2% reduction)
- Benchmark: -0.87% execution time (p=0.04, statistically significant)
