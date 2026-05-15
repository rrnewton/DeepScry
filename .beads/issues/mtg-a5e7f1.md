---
title: Twin Blades crashes client with 'Only Equipment or Auras can be attached' on resolution
status: open
priority: 2
issue_type: task
created_at: 2026-05-14T14:28:04.644518025+00:00
updated_at: 2026-05-14T14:28:04.644518025+00:00
---

# Description

## Summary

In a network fuzz test, Player 1 (Ryan) cast Twin Blades, which crashed the client process the moment it resolved. The error is from the engine, not network sync:

```
Error: InvalidAction("Game error: Game error: Invalid game action: Only Equipment or Auras can be attached")
```

The crash terminates the client which then triggers a downstream connection_reset on the server side and an unrelated 'Game 1: P1 handler exited unexpectedly' error.

## Reproducer

```bash
cd mtg-forge-rs
./tests/network_vs_local_equivalence_e2e.sh 5 random random
```

Logs preserved at /tmp/qa-fail-equipment.

## Client log excerpt

```
========================================
Turn 17 - Ryan's turn
========================================
  <Choice> Ryan chose 0 - cast Twin Blades
  [GAMELOG Turn17 UP] Ryan casts Twin Blades (7) (putting on stack)
  [GAMELOG Turn17 UP] Tap Mountain for {R}
  [GAMELOG Turn17 UP] Tap Swamp for {B}
  [GAMELOG Turn17 UP] Tap Swamp for {B}
  <Choice> Ryan chose 'p' (pass priority)
  [GAMELOG Turn17 UP] Twin Blades (7) resolves
Error: InvalidAction("Game error: Game error: Invalid game action: Only Equipment or Auras can be attached")
```

## Card

Twin Blades is from the Avatar set (Ryan's deck). Per upstream Forge it creates two Equipment artifact tokens and attaches them. The error suggests the rust engine is calling 'attach' on a non-Equipment object, possibly the source instant itself rather than the spawned token, or the token is being created without the Equipment supertype.

Search `forge-java/forge-gui/res/cardsfolder` for 'twinblades' and compare to AbilityFactory_Token / AttachEffect handling in Rust.

## Discovered by

`bug_finding/network_fuzz_test.py` 45-config pass on `qa-fuzz-testing` @ fe820468, 2026-05-14.
