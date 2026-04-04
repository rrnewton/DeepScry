---
title: Add friendly error when mtg binary is not built
status: open
priority: 1
issue_type: task
created_at: 2026-04-04T01:50:09.414290573+00:00
updated_at: 2026-04-04T01:50:09.414290573+00:00
---

# Description

Files: agentplay/agent_game.py, agentplay/engine.py

Action: At startup (before running any game), check if the mtg binary exists at the expected path (target/release/mtg). If not found, print a clear error message like:

"Error: MTG engine binary not found at target/release/mtg
Build it with: cargo build --release
(from the mtg-engine directory)"

Currently it fails with a cryptic: "IoError(Os { code: 2, kind: NotFound, message: "No such file or directory" })"

The check should go in engine.py's GameEngine.__init__ or start method, before the first subprocess call.

Why: Users get a confusing Rust IoError when the binary isn't built.

Verify: Run agent_game.py without the binary built and see a helpful error message.
