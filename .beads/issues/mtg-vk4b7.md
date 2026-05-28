---
title: 'Network desync: rogerbrand seed=3 P2 state-hash mismatch at choice_seq=216 (Demonic Tutor / library-search shadow sync)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T16:39:35.293229311+00:00
updated_at: 2026-05-28T16:39:35.293229311+00:00
---

# Description

## Summary

Deterministic network desync reproducible with the old_school rogerbrand decks at seed=3, random/random controllers. PRE-EXISTING on integration (confirmed by stashing the fix-network-desync changes and re-running the baseline binary: identical fail point). NOT fixed by the fix-network-desync mana-cache/winner-race work — distinct class (library-search / hidden-info shadow sync, mtg-212 / mtg-259 family).

## Repro (deterministic, fails 2/2)

```bash
cd mtg-forge-rs
PORT=26123
./target/release/mtg server --port $PORT --seed 3 --network-debug --no-color-logs &
sleep 2
./target/release/mtg connect decks/old_school/01_rogue_rogerbrand.dck --server localhost:$PORT --controller random --seed-player 3 -n Roger &
sleep 1
./target/release/mtg connect decks/old_school/05_mono_black_rogerbrand.dck --server localhost:$PORT --controller random --seed-player 3 -n Brand &
wait
```

## Fail point

FATAL: P2 state hash mismatch! server=faacb39002b9b14b client=5b8cf425286d9a72 at choice_seq=216 action_count=938.

## Divergence context (server + P2 client gamelog right before fatal)

- Brand activates Library of Alexandria (draw a card)
- Brand casts Demonic Tutor -> 'Demonic Tutor searches Brand's library for a Card card and puts it into Hand'
- Brand casts Royal Assassin -> then immediately the P2 state-hash mismatch fires.
- Earlier in the game: Chaos Orb (flip-destroy), All Hallow's Eve, Sinkhole (destroy land) all resolved without an earlier mismatch.

Hash diverges right around the Demonic Tutor library search + subsequent cast. Strongly suggests the searched CardId / library order in the P2 shadow state diverges from the server (the same class as mtg-212 reveal-ordering and mtg-259 mid-game shadow divergence), NOT a mana-cache or winner issue.

## Next steps

1. Add RUST_LOG=debug capture of the undo_log around action 938 on both server and P2 client; find the first diverging GameAction (likely a MoveCard with a different CardId for the tutor result, or a reveal not propagated to the P2 shadow).
2. Compare with the LibraryReordered / library_search_result sync path (ChoiceAccepted carries library_search_result; verify Demonic Tutor's NamedCard/Any search routes through it for the shadow).
3. Regression: once fixed, add an old_school rogerbrand seed=3 e2e (and consider a minimal puzzle reproducer).

Related: mtg-212, mtg-259, mtg-429 (fuzz tracker). Found during fix-network-desync stability sweep 2026-05-28.
