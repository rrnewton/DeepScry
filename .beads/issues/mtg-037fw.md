---
title: 4-way gamelog equivalence test for NETWORK_MODE
status: open
priority: 2
issue_type: task
created_at: 2025-12-08T11:49:48.576522867+00:00
updated_at: 2025-12-29T23:23:23.624984775+00:00
---

# Description



## Update (2025-12-29) - Auto-pass Bug Fix

**Root Cause 2 Identified: Client auto-passes for opponent**

The client's priority loop auto-passes when no available abilities are computed:

```rust
if available_count == 0 {
    break None;  // Auto-pass
}
```

But for opponent's hand, the client doesn't know the contents, so `get_available_spell_abilities()` returns empty. This means the client **never asks the RemoteController** for opponent's choices and auto-passes instead\!

This causes massive divergence: opponent plays spells on server, but client skips them entirely.

**Fix Applied (Partial)**:

1. Added `ControllerType::Remote` variant
2. Priority loop now checks: `if available_count == 0 && \!is_remote { auto-pass }`
3. RemoteController now always gets asked, even when `available` is empty
4. Added `spell_ability: Option<SpellAbility>` to OpponentChoice protocol
5. RemoteController can use `spell_ability` when provided by server

**Remaining Work**:
- Server needs to populate `spell_ability` in OpponentChoice (currently None)
- Client needs to handle executing abilities for cards not in its game state
- Cards played from hand need to be revealed before execution

Validation passes with partial fix.
