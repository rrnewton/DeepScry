---
title: Network protocol only supports single-select for attackers/blockers
status: open
priority: 2
issue_type: task
created_at: 2025-12-31T00:57:13.797781179+00:00
updated_at: 2025-12-31T00:57:13.797781179+00:00
---

# Description

## Bug Description

The determinism test comparing local vs networked games revealed that multi-select
choices (attackers, blockers, discard, etc.) only transmit the first selection.

## Root Cause

In `network/controller.rs`:
```rust
// FIXME-UNFINISHED: Support multi-select for attackers (currently single selection)
ChoiceResult::Ok(SmallVec::from_slice(&[available_creatures[creature_idx]]))
```

In `network/local_controller.rs`:
```rust
available_creatures.iter().position(|&a| a == attackers[0]).unwrap_or(0) + 1
```

Only the first attacker's index is transmitted; the rest are ignored.

## Impact

- Random controller choosing 2 attackers only sends 1 to server
- Games diverge when players try to attack with multiple creatures
- Breaks determinism: local game != networked game

## Affected Choice Types

All marked with FIXME-UNFINISHED:
- Attackers (line 527)
- Blockers (line 574)  
- Damage order (line 621)
- Discard (line 662)
- Targets (line 458)
- Mana sources (line 493)

## Fix Required

Modify protocol to support multi-select:
1. Change SubmitChoice to include array of indices
2. Update server to process array
3. Update RemoteController to replay array
