---
title: Extend agentplay for web GUI playtesting with Playwright
status: closed
priority: 0
issue_type: task
created_at: 2026-04-04T01:50:09.418401682+00:00
updated_at: 2026-05-12T13:59:24.903097834+00:00
closed_at: 2026-05-12T13:59:24.903097764+00:00
---

# Description

CLI playtesting COMPLETE (18+ bugs found). Web GUI playtesting blocked on: (1) wasm32-unknown-unknown target not installed, (2) forge-java submodule can't be cloned. Unblock by running: rustup target add wasm32-unknown-unknown && git submodule update --init forge-java
