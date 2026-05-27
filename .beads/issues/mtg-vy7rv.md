---
title: 'CI clippy WASM target fails: pinned nightly has no wasm rust-std'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-27T13:42:29.609824718+00:00
updated_at: 2026-05-27T13:42:29.609824718+00:00
---

# Description

The CI step 'Run clippy (WASM target)' fails with E0463 'can't find crate for core' because the pinned nightly toolchain (rust-toolchain.toml -> nightly-2025-11-28) has no wasm32-unknown-unknown rust-std installed.

## Symptom
```
error[E0463]: can't find crate for core
  = note: the wasm32-unknown-unknown target may not be installed
  = help: consider downloading the target with rustup target add wasm32-unknown-unknown
error: could not compile cfg-if (lib) due to 1 previous error
##[error]Process completed with exit code 101.
```

## Root cause
.github/workflows/ci.yml uses dtolnay/rust-toolchain@nightly with targets: wasm32-unknown-unknown, which installs wasm rust-std onto the **latest** nightly. But rust-toolchain.toml pins nightly-2025-11-28, so when cargo clippy runs, rustup auto-switches to the pinned toolchain. The CI log explicitly says: 'note that the toolchain nightly-2025-11-28-x86_64-unknown-linux-gnu is currently in use (overridden by ... rust-toolchain.toml)'. wasm rust-std was installed only for the latest nightly, not the pinned one.

## Recommended fix (one-line)
Add targets = ["wasm32-unknown-unknown"] to rust-toolchain.toml. The pin will then pull wasm rust-std on every rustup install, both in CI and in fresh local clones.

## Alternative
Add a CI step before clippy: 'rustup target add wasm32-unknown-unknown' (targets the active/pinned toolchain).

## Discovery
Found during integration-branch triage 2026-05-27_#2297(b5cbdc85). See ai_docs/integration_triage_20260527.md (F2). Reproduces on CI runs 26482076564 and 26514285939.

## Related
- 7a423c75 (commit that added the clippy-wasm CI step)
- mtg-99og6 (CI status policy)
