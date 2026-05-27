---
title: Unified axum web+lobby server (replaces python http.server + standalone mtg server)
status: open
priority: 2
issue_type: task
created_at: 2026-05-27T20:52:57.864233049+00:00
updated_at: 2026-05-27T20:53:23.664591502+00:00
---

# Description

Implementation of mtg-uwv3w (unify) + mtg-pm0lz (systemd/graceful shutdown) + mtg-dbypv (TLS). See those for design rationale.

Done in this branch (axum-unified-server):
- New `web-server` cargo feature gating axum + tower-http + axum-server + rustls.
- New `mtg server-web` subcommand (--bind, --static-dir, --lobby-path, --tls-cert/key with env fallback, plus all embedded-lobby options).
- `mtg-engine/src/web_server/` module: boots embedded GameServer on a private 127.0.0.1:<random> port and bidirectionally proxies axum WS frames to a tokio-tungstenite client connection. Avoids refactoring ~3 kLOC of WebSocketStream<TcpStream>-typed lobby code (per the safety-valves section of the brief).
- ServerConfig.bind_host (default 0.0.0.0; web-server overrides to 127.0.0.1) so the embedded lobby is loopback-only.
- Graceful shutdown: SIGTERM/SIGINT triggers watch channel, axum_server::Handle::graceful_shutdown(SHUTDOWN_GRACE=30s) for TLS path, with_graceful_shutdown for plain HTTP path.
- web/server-config.js self-detects ws://vs wss:// from window.location and uses /lobby on the same origin — works for HTTP direct-IP, HTTPS-behind-CF, AND any local dev setup.
- infra/deepscry.service systemd unit; scripts/deploy-cloud.sh rewritten to install/refresh it via passwordless sudo and drop python+tmux entirely.

Verified locally:
- cargo build --release --bin mtg --features network: green.
- cargo check --no-default-features --features native: green (web-server is properly gated).
- cargo fmt --all -- --check: clean.
- cargo clippy --features network --lib --bins: warnings only.
- Smoke test: `GET /` returns 200 (31453 B index.html), `GET /lobby` returns 101 Switching Protocols with proper Sec-WebSocket-Accept header (proxy upgrade works).

Follow-ups:
- Dedicated `ServerMessage::ServerRestart { reason }` variant (today shutdown closes proxied sockets without injecting a final frame; clients see WS close).
- Optional refactor: make `network::server` generic over Sink/Stream so axum can drive the lobby directly without the loopback proxy hop.
