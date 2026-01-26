---
title: Flaky network sync test blocks choose_from_library refactoring
status: open
priority: 2
issue_type: task
labels:
- bug,network,blocked
created_at: 2026-01-26T16:14:55.863430496+00:00
updated_at: 2026-01-26T16:14:55.863430496+00:00
---

# Description

## Summary

The `choose_from_library` interface refactoring (CardId-based → name-based for network compatibility) is blocked by an apparent flaky/timing-sensitive network synchronization bug.

## Observed Behavior

1. **HEAD (without changes)**: Network equivalence test passes consistently (3/3 runs)
2. **With refactoring changes**: Network equivalence test fails consistently (timeout after 180s)
3. **Failure mode**: NETWORK SYNC MISMATCH showing draw step desync:
   - Server: Libs [30, 31], Hand CardIds [14, 35, 37, 38, 39]
   - Client: Libs [31, 31], Hand CardIds [13, 35, 37, 38, 39]

## Why This Is Strange

The refactoring ONLY changes `choose_from_library`:
- Updated trait signature from `valid_cards: &[CardId]` → `valid_card_names: &[&str]`
- Updated return type from `ChoiceResult<Option<CardId>>` → `ChoiceResult<Option<usize>>`
- Updated all controller implementations
- Updated network bridge in `network_choice.rs`

But the **test decks (avatar draft) don't have tutoring effects** - `choose_from_library` should never be called!

The desync shows a **draw step issue** (library size difference of 1), not a library search issue.

## Hypothesis

The refactoring changes cause:
1. Different Rust codegen/monomorphization
2. Different binary layout affecting timing
3. Exposing a latent race condition in network synchronization

## Changes Preserved

The refactoring changes are preserved on branch `choose-from-library-refactor` (commit 58aee7d8):

- `mtg-engine/src/game/controller.rs` - Trait signature
- `mtg-engine/src/game/*_controller.rs` - All controller implementations
- `mtg-engine/src/game/game_loop/network_choice.rs` - Network bridge
- `mtg-engine/src/network/controller.rs` - NetworkController
- `mtg-engine/src/network/local_controller.rs` - LocalController
- `mtg-engine/src/network/remote_controller.rs` - RemoteController
- `mtg-engine/src/game/game_loop/priority.rs` - ReplayChoice update
- Various test files

## To Reproduce

Checkout the branch and run:
```bash
./tests/network_vs_local_equivalence_e2e.sh 3
```

## Resolution Path

1. Debug the underlying network sync issue (may be in reveal/draw synchronization)
2. Once fixed, re-apply the `choose_from_library` refactoring
3. The refactoring itself is necessary for network mode to support tutoring effects

## Timestamp

2026-01-26_#398(f3658a0)
