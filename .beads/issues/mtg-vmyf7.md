---
title: Network protocol only supports single-select for attackers/blockers
status: closed
priority: 2
issue_type: task
created_at: 2025-12-31T00:57:13.797781179+00:00
updated_at: 2025-12-31T02:06:10.567331083+00:00
---

# Description

## Bug Description

The determinism test comparing local vs networked games revealed that multi-select
choices (attackers, blockers, discard, etc.) only transmit the first selection.

## Root Cause (FIXED)

Changed `SubmitChoice.choice_index: usize` to `choice_indices: Vec<usize>` 
across the entire network protocol stack.

## Fix Applied

**Protocol changes:**
- `protocol.rs`: Changed `SubmitChoice` and `OpponentChoice` to use `Vec<usize>`
- `controller.rs`: `request_choice()` now returns `Vec<usize>`, all choice methods updated
- `server.rs`: `OpponentChoiceInfo` and `PendingChoice` updated to `Vec<usize>`
- `local_controller.rs`: `LocalChoice` and `send_choice()` updated
- `remote_controller.rs`: `RemoteMessage` and `wait_for_choice()` updated
- `client.rs`: Message handling updated

**WASM network module:**
- `wasm/network/client.rs`: `submit_choice()` updated
- `wasm/network/local_controller.rs`: All choice methods updated for multi-select
- `wasm/network/remote_controller.rs`: `try_get_choice()` returns `Vec<usize>`

**Tests:**
- `network_e2e.rs`: Updated test assertions

## Verification

Determinism test: Local game (seed 42) and Network game (same seed) both end at Turn 41 with identical results (Player1: -2 life, Player2: 8 life).

All 691 tests pass.
