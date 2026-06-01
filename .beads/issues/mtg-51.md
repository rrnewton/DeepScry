---
title: Fast binary game snapshots (rkyv)
status: open
priority: 4
issue_type: feature
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-06-01T13:24:15.813472265+00:00
---

# Description

## Fast binary game snapshots (rkyv)

GARDENING (2026-06-01): possibly-stale, needs human/code re-check — the original goal was zero-copy binary snapshots with rkyv. In practice, bincode was implemented instead (mtg-99, now closed). SnapshotFormat::Bincode exists in mtg-engine/src/game/snapshot.rs and is the default. rkyv specifically (true zero-copy/memory-mapped deserialization) is NOT implemented.

Whether rkyv is still desired as an upgrade from bincode is a question for the user. If bincode performance is adequate (binary default, fast serialize/deserialize), rkyv may be premature optimization. If snapshot save/load is still a bottleneck (the original motivation was MCTS transposition tables), rkyv is worth pursuing.

ORIGINAL DESCRIPTION:
Zero-copy binary serialization with rkyv:
- Instant deserialization (no parsing)
- Save/load game states efficiently
- Use for transposition tables (MCTS)
- Network play synchronization

CURRENT STATE (2026-06-01): bincode serves as the binary snapshot format. rkyv not implemented. Priority 4 (low).
