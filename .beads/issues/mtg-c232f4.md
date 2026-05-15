---
title: Snapshot resume fails on bincode with deserialize_any error
status: open
priority: 2
issue_type: bug
labels:
- snapshot
- network
created_at: 2026-05-15T17:01:22.520265380+00:00
updated_at: 2026-05-15T17:01:22.520265380+00:00
---

# Description

## Summary

`tests/snapshot_resume_e2e.sh` fails with:

```
Error: InvalidAction("Failed to load snapshot: Failed to deserialize snapshot: Bincode does not support the serde::Deserializer::deserialize_any method")
```

on stops at choice 3, 8, and 25 in the `bincode/stop` mode. The `json/stop`
and `override` phases pass.

## Status

PRE-EXISTING on `origin/integration` baseline (ff1817f7) — verified by
running the test in a clean worktree at that commit. NOT caused by the
recent merges (fix-seismic-sense, fix-cycle-desync, fix-scry-choice-pipeline,
server-lobby) — those are all green for this test on their own.

## Root cause hypothesis

`deserialize_any` is invoked when serde encounters a self-describing
format requirement, typically from:
- `#[serde(untagged)]` enum variants
- `#[serde(flatten)]` on structs containing untagged enums
- `serde_json::Value` / generic `Value` types in the snapshot
- Internally-tagged enums (`#[serde(tag = "...")]`) with struct variants
  in some serde-bincode interactions

## Repro

```sh
git checkout ff1817f7  # or any later integration commit
cargo build --release --features network
CARDSFOLDER=$PWD/cardsfolder bash tests/snapshot_resume_e2e.sh
```

Failures in `[Phase 2] Replay matches: bincode/stop@{3,8,25}`.

## Related

- mtg-cc4837 (FIXED earlier resume-cache bug)
