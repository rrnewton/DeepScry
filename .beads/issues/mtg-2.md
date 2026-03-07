---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-07T19:56:10.488074768+00:00
---

# Description



## Latest Optimization (2026-03-07_#1865(cdb8c74))

✅ **Eliminate Vec allocation+clone in targeting callback** - **-3.9% bytes/game (simple_bolt), -4.4% allocation blocks**
- Changed cast_spell_8_step callback from FnMut→FnOnce, Vec<CardId>→SmallVec<[CardId; 2]>
- Eliminated 2 heap allocations per spell cast (Vec creation + Vec::clone)
- SmallVec<[CardId; 4]> for tapped_sources avoids heap for spells costing ≤4 mana
- DHAT: 1,258,103→1,245,431 bytes (-1.0%), 18,006→17,214 blocks (-4.4%)
- simple_bolt: 2,863→2,751 bytes/game (-3.9%), +0.7% actions/sec (p=0.00)
