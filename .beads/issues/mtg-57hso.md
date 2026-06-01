---
title: 'mtg server / server-web: fix misleading --help, document the real distinction, purge retired-python mentions'
status: open
priority: 3
issue_type: task
created_at: 2026-05-31T21:37:21.293301614+00:00
updated_at: 2026-05-31T21:37:21.293301614+00:00
---

# Description

USER (2026-05-31): server-web --help should be extensive and must NOT reference old/retired state ('used to be a separate python server' — irrelevant to a finished product).

Findings: (1) 'mtg server' about text says 'dedicated game server (TUI watches games...)' but there is NO TUI — it is the bare multiplayer WebSocket GameServer (clients connect via 'mtg connect' / self-hosted 'Custom Network Game'), no HTTP/static serving. (2) 'mtg server-web' about text 'dedicated game server (no TUI...)' is near-identical and doesn't convey that it is the FULL browser product: unified axum server that serves web/ (HTML+WASM+card bins) AND embeds the GameServer lobby on a private port. (3) web_server.rs:48,53 docstrings still describe the retired 'standalone Python script (serve_web.py)' / 'old two-process (Python proxy + Rust WS) deployment' — delete.

DELIVER: rewrite both subcommands' about/long_about so each is self-contained and the server-vs-server-web distinction is obvious; give server-web a thorough long_about (what it serves, ports, static-dir, lobby model, TLS, bug-report password); strip all retired-python/legacy mentions from help + docstrings. Decide+document whether 'mtg server' stays a separate command (headless WS server for CLI/custom) or folds into 'server-web --no-web'. main.rs Commands::Server@643 / ServerWeb@717; web_server.rs.
