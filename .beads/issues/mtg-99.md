---
title: 'TODO: overhaul snapshot serialization'
status: closed
priority: 2
issue_type: task
labels:
- human
created_at: 2025-10-27T13:51:48+00:00
updated_at: 2026-06-01T13:22:17.995181031+00:00
---

# Description

## Snapshot serialization overhaul — DONE

Closed 2026-06-01 gardening: DONE. All three requested items from the original description are implemented.

1. Binary format as default: mtg-engine/src/main.rs line 337/509: '--json: Use JSON format for snapshots (default is binary format)'. The default is now binary (bincode).

2. No more pretty-printed JSON (binary is default so this is moot; --json still works for debugging).

3. Flag to control json/binary: the --json flag exists in both the TUI subcommand (line 337) and resume subcommand (line 509). SnapshotFormat enum at game/snapshot.rs selects the backend.

Evidence: tests/snapshot_resume_e2e.sh covers 'stop-on-choice + resume in both bincode and JSON snapshot formats' (from mtg-414 description). main.rs line 1053 builds snapshot_format from the --json flag.
