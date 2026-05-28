---
title: Rename engine crate mtg-forge-rs -> mtg-engine
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T19:46:50.620842838+00:00
updated_at: 2026-05-28T19:46:50.620842838+00:00
---

# Description

Rename the engine crate package from mtg-forge-rs to mtg-engine (matching its directory mtg-engine/). The product stays DeepScry; the GitHub repo is currently named mtg-forge-rs.

Scope (surveyed at depth 2389):
- mtg-engine/Cargo.toml: name = "mtg-forge-rs" -> "mtg-engine".
- 593 lines of mtg_forge_rs (underscore lib path) across 52 .rs files -> mtg_engine.
- -p mtg-forge-rs -> -p mtg-engine in CI (.github/workflows/ci.yml), make targets, validate scripts (4 files).
- Workspace Cargo.toml member/dep refs if any.
- Flakiness canonical names validate.mtg-forge-rs--* become validate.mtg-engine--* (scripts/flakiness_stress.py derives pkg dynamically, so mostly doc/examples in TEST_FLAKINESS.md).

Constraints: do AFTER the desync keystone (mtg-vk4b7) lands, on a clean base (avoids a 593-line .rs rebase conflict + protects desync's stability proof). Full make validate required. Mechanical but wide.

Decision (user 2026-05-28): crate -> mtg-engine; do after desync; MUST land cleanly BEFORE the flakiness baseline run so the DB uses mtg-engine names.
