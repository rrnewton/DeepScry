---
title: 'feat(deploy): replace tmux with systemd unit + graceful WS shutdown'
status: open
priority: 2
issue_type: feature
labels:
- deploy
- systemd
depends_on:
  mtg-dbypv: related
  mtg-uwv3w: blocks
created_at: 2026-05-27T20:40:57.843769430+00:00
updated_at: 2026-05-27T20:40:57.843769430+00:00
---

# Description

## Context

Part of the web-server-unification design. Depends on **mtg-uwv3w** (unified axum process).

## Problem with the current deploy

`scripts/deploy-cloud.sh` runs the static web server and the Rust lobby in **tmux sessions** (`mtg-server`, `mtg-rust-server`). Drawbacks:

- No supervision: if a process crashes, nothing restarts it.
- No log rotation: `~/mtg-forge-rs/rust-server.log` grows unbounded.
- No structured signal handling: a `tmux kill-session` SIGHUPs the child, no chance to drain in-flight WS games.
- No resource limits.
- Boot-time start requires `crontab @reboot tmux ...` hacks.

## Plan: systemd unit + graceful shutdown

### Unit file (`/etc/systemd/system/deepscry.service`)

Coordinator-provided skeleton:

```ini
[Unit]
Description=DeepScry MTG web + lobby server
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
User=newton
Group=newton
WorkingDirectory=/home/newton/mtg-forge-rs
Environment=RUST_LOG=info
Environment=MTG_TLS_CERT=/etc/ssl/deepscry/deepscry.crt
Environment=MTG_TLS_KEY=/etc/ssl/deepscry/deepscry.key
ExecStart=/home/newton/mtg-forge-rs/target/release/mtg server-web \
    --port 8080 --web-root /home/newton/mtg-forge-rs/web \
    --cards /home/newton/mtg-forge-rs/cardsfolder
Restart=on-failure
RestartSec=3
TimeoutStopSec=35
KillSignal=SIGTERM
LimitNOFILE=65536
MemoryMax=8G
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

Notes:
- `CAP_NET_BIND_SERVICE` isn't strictly required for port 8080 but is harmless and future-proofs binding <1024 if we ever move to 443 directly.
- `TimeoutStopSec=35` aligns with the 30s graceful-drain budget inside the process.
- `User=newton` (not root) means the cert/key file group must include `newton` — see #mtg-dbypv.

### Graceful shutdown in axum

Replace the bare `axum::serve(...).await` with:

```rust
axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

where `shutdown_signal()` selects on `tokio::signal::ctrl_c()` and a `SIGTERM` listener (`tokio::signal::unix::signal(SignalKind::terminate())`). The same signal must be propagated into the lobby task supervisor so in-flight `handle_lobby_connection` tasks can:

1. Stop accepting new `CreateGame` / `JoinGame` (return `ServerFull` or a new `ServerShuttingDown` error).
2. Send `{"type":"ServerRestart","reconnect_after_ms":3000}` (NEW protocol message — add to `mtg-engine/src/network/protocol.rs` `ServerMessage` enum) to every active connection.
3. Let in-flight games complete a final choice-roundtrip but cap total drain at ~30s.

Existing `mtg-engine/src/network/server.rs` already uses `tokio::select!` heavily (lines 1548, 1692, 2131) — wire a shared `CancellationToken` (from `tokio-util`) into those select arms.

### Browser-side handling

`web/index.html` (lobby page) and downstream game pages should treat `ServerRestart` as a soft disconnect: show "Server restarting, reconnecting…" and reconnect with exponential backoff. Pure UX — no server cooperation beyond the one message.

### Deploy script changes

`scripts/deploy-cloud.sh`:
- Drop all `tmux new-session` / `tmux kill-session` invocations.
- Add: rsync `scripts/deepscry.service` (new file in repo) to `/etc/systemd/system/` on the VM.
- `sudo systemctl daemon-reload && sudo systemctl enable --now deepscry && sudo systemctl restart deepscry`.
- Update the OPERATIONS comment block: replace tmux commands with `systemctl status/restart/stop deepscry` and `journalctl -u deepscry -f`.

### Open question — game session resume

The current WS protocol does not appear to support session resume after disconnect (clients reconnect as fresh sessions; per-game state lives in the server's `SharedLobby` and is lost when a connection closes). For now, graceful shutdown is **best-effort**: short downtimes (3s restart) won't usually catch a game mid-decision, but a restart during an active game still loses that game. File a follow-up issue if/when reconnect-with-resume becomes a priority. (Sketch: persist game state to disk keyed by `game_name` + `password`, allow `JoinGame` on a game with a matching pending-session token to resume.)

## Acceptance criteria

- `systemctl restart deepscry` causes ≤5s downtime for static requests (handled by retry / browser cache).
- Active WS clients receive `ServerRestart` before disconnection.
- `journalctl -u deepscry -f` shows structured logs replacing `tail -f rust-server.log`.
- `systemctl status deepscry` reports `active (running)` post-deploy.
- Killing the process with `kill -9 $(pidof mtg)` causes `Restart=on-failure` to relaunch within `RestartSec=3`.
- `deploy-cloud.sh` contains zero `tmux` invocations.

## Dependencies

- Depends on **mtg-uwv3w** (axum process to supervise).
- Related to **mtg-dbypv** (TLS cert file permissions must allow service user to read).
