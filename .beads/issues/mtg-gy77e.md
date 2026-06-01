---
title: 'Standardized + flexible test binary strategy: trusted env signal for prebuilt mtg binary + feature-flag encoding'
status: closed
priority: 3
issue_type: task
created_at: 2026-05-31T20:13:58.073266180+00:00
updated_at: 2026-06-01T13:35:06.726354360+00:00
---

# Description

## Standardized flexible test binary strategy: trusted env signal for prebuilt mtg binary — DONE

Closed 2026-06-01 gardening: DONE. The MTG_REUSE_PREBUILT / MTG_BIN pattern is implemented.

Evidence: tests/lib/test_helpers.sh:42-68:
- ensure_mtg_binary() function
- MTG_BIN env var (default: $WORKSPACE_ROOT/target/release/mtg)
- Fast path: if MTG_REUSE_PREBUILT=1 and binary exists, skip rebuild
- Comments: 'CI builds mtg --release --features network ONCE in the Build release binary step, then runs the whole shell-script test binary; having each of the 26 scripts re-invoke cargo build was the single biggest contributor to the ~1046s serial shell-test time (mtg-578)'

All 74 e2e tests call ensure_mtg_binary(). CI sets MTG_REUSE_PREBUILT=1 to avoid re-building. Local make validate still builds fresh (MTG_REUSE_PREBUILT not set).

Feature flag encoding: MTG_BIN path + MTG_REUSE_PREBUILT=1 pattern; if the binary doesn't exist at the expected path, the test fails loudly.
