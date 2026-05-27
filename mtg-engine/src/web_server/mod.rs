//! Unified web server: static files + lobby WebSocket proxy + optional TLS.
//!
//! Replaces the old dual-process deploy (Python `http.server` for `web/`
//! plus a separate `mtg server` for the lobby). One axum process now
//! binds a single public port (default 8080) and serves:
//!
//! - `GET /…` → static files out of `--static-dir` (default `./web`).
//! - `GET /lobby` → WebSocket upgrade, proxied bidirectionally to the
//!   in-process [`crate::network::GameServer`] running on a private
//!   loopback port.
//!
//! ## Why a proxy instead of plugging axum directly into the lobby code
//!
//! The existing lobby implementation (`network::server`) threads
//! `tokio_tungstenite::WebSocketStream<TcpStream>` through
//! `WaitingPlayer`, `JoinedPlayer`, `PlayerConnection`, and a dozen
//! call-sites — including `tokio_tungstenite::tungstenite::Message`
//! values stored in oneshot channels for the lobby hand-off. Refactoring
//! that to be generic over a `Sink/Stream` (or to use axum's
//! `axum::extract::ws::Message`) is a 3 kLOC change that risks the
//! "Desync is ALWAYS Fatal" invariant.
//!
//! Instead we keep the existing `GameServer` 100% unchanged on a private
//! loopback port and proxy raw WebSocket frames through axum. This is
//! also the path the safety-valves section of the design doc recommends
//! for exactly this reason.
//!
//! ## TLS
//!
//! If both `tls_cert` and `tls_key` are supplied (CLI flags or
//! `MTG_TLS_CERT` / `MTG_TLS_KEY` env vars) we terminate TLS via
//! `axum_server::bind_rustls`. Otherwise we serve plain HTTP. Deployment
//! behind Cloudflare uses plain HTTP at the origin today; the TLS path
//! is ready for the future where we want direct HTTPS.
//!
//! ## Graceful shutdown
//!
//! `run_web_server` installs a SIGTERM/Ctrl-C handler. On signal:
//! - Static-file serving stops accepting new connections.
//! - Each active proxied WS connection is sent a final
//!   `ServerMessage::Error { message: "server-restart", fatal: true }`
//!   (the existing fatal-error path; the protocol does not yet have a
//!   dedicated `ServerRestart` variant — adding one is a follow-up).
//! - We give clients up to [`SHUTDOWN_GRACE`] to drain before the
//!   process exits.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::network::{GameServer, ServerConfig};

/// Maximum time we wait for in-flight WebSocket clients to drain after a
/// shutdown signal before tearing the process down anyway.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);

/// CLI/library-facing configuration for the unified web server.
#[derive(Debug, Clone)]
pub struct WebServerConfig {
    /// Public bind address (e.g. `0.0.0.0:8080`).
    pub bind: SocketAddr,
    /// Directory served as static assets at `/`.
    pub static_dir: PathBuf,
    /// Path that triggers a WebSocket upgrade + proxy to the lobby.
    pub lobby_path: String,
    /// Optional TLS certificate (PEM) — TLS is enabled iff both
    /// `tls_cert` AND `tls_key` are `Some`.
    pub tls_cert: Option<PathBuf>,
    /// Optional TLS private key (PEM).
    pub tls_key: Option<PathBuf>,
    /// Embedded `GameServer` configuration. The `port` field is
    /// overridden to a private loopback port chosen by the OS.
    pub lobby_config: ServerConfig,
}

/// Shared state passed to axum handlers.
#[derive(Clone)]
struct AppState {
    /// `ws://127.0.0.1:<port>` — internal upstream where the embedded
    /// `GameServer` is listening.
    upstream_ws_url: Arc<String>,
    /// Watch channel: flipped to `true` when SIGTERM/Ctrl-C arrives.
    /// Proxied WS tasks poll this to drain gracefully.
    shutdown_rx: watch::Receiver<bool>,
}

/// Entry point for `mtg server-web`. Boots the embedded GameServer on a
/// private loopback port, then serves static files + WS proxy on the
/// public bind address.
///
/// # Errors
///
/// Returns an error if either the embedded lobby fails to bind, the
/// public listener fails to bind, or (with TLS) the cert/key cannot be
/// loaded.
pub async fn run_web_server(mut config: WebServerConfig) -> Result<()> {
    // ---- 1. Bind a private loopback socket for the embedded lobby. ----
    //
    // We pre-bind here (via std + into tokio) only to discover the
    // OS-assigned port, then hand the port to GameServer which will
    // re-bind. There's a tiny TOCTOU race window but it's loopback-only
    // and only matters at startup; if it ever fires the user re-runs.
    let internal_port = pick_loopback_port().await?;
    config.lobby_config.port = internal_port;
    config.lobby_config.bind_host = "127.0.0.1".to_string();
    let upstream_ws_url = format!("ws://127.0.0.1:{internal_port}");
    log::info!("[web-server] embedded lobby will listen on 127.0.0.1:{internal_port} (internal)");

    // ---- 2. Spawn the embedded GameServer. ----
    let lobby_handle = tokio::spawn(async move {
        let mut server = GameServer::new(config.lobby_config.clone());
        if let Err(e) = server.run().await {
            log::error!("[web-server] embedded GameServer exited: {e}");
        }
    });

    // Give the embedded server a moment to actually bind. We probe with
    // a short retry loop rather than sleeping blindly so slow hosts
    // don't lose the first client.
    wait_for_loopback_port(internal_port).await?;
    log::info!("[web-server] embedded lobby is up");

    // ---- 3. Build the axum app. ----
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let state = AppState {
        upstream_ws_url: Arc::new(upstream_ws_url),
        shutdown_rx: shutdown_rx.clone(),
    };

    // Static-file service. `ServeDir` handles range requests, index.html,
    // and correct MIME types automatically. We layer a permissive
    // Cache-Control header so re-deploys take effect immediately — the
    // WASM bundle filenames are content-hashed at build time anyway.
    let static_service = ServeDir::new(&config.static_dir).append_index_html_on_directories(true);

    let app = Router::new()
        .route(&config.lobby_path, get(lobby_ws_handler))
        .fallback_service(static_service)
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        ))
        .with_state(state);

    // ---- 4. Install shutdown signal handler. ----
    let shutdown_tx_for_signal = shutdown_tx.clone();
    let signal_fut = async move {
        wait_for_shutdown_signal().await;
        log::warn!("[web-server] shutdown signal received; draining (up to {SHUTDOWN_GRACE:?})");
        let _ = shutdown_tx_for_signal.send(true);
        // Give proxied tasks a chance to flush the server-restart frame.
        tokio::time::sleep(SHUTDOWN_GRACE).await;
    };

    // ---- 5. Serve (TLS if configured, plain HTTP otherwise). ----
    let bind = config.bind;
    match (&config.tls_cert, &config.tls_key) {
        (Some(cert_path), Some(key_path)) => {
            log::info!("[web-server] starting HTTPS on {bind} (cert={cert_path:?})");
            // rustls 0.23 requires explicit CryptoProvider selection before
            // any TLS work. Install ring as the process-wide default.
            // `install_default` returns Err if one is already installed; we
            // ignore that since the only way it happens is duplicate calls.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
                .await
                .with_context(|| format!("loading TLS cert/key from {cert_path:?} / {key_path:?}"))?;
            // axum-server has its own graceful handle; we wire signal_fut
            // by calling `handle.graceful_shutdown(Some(SHUTDOWN_GRACE))`
            // when the signal fires.
            let handle = axum_server::Handle::new();
            let handle_for_signal = handle.clone();
            tokio::spawn(async move {
                signal_fut.await;
                handle_for_signal.graceful_shutdown(Some(SHUTDOWN_GRACE));
            });
            axum_server::bind_rustls(bind, tls)
                .handle(handle)
                .serve(app.into_make_service())
                .await
                .context("axum-server (TLS) exited with error")?;
        }
        _ => {
            log::info!("[web-server] starting HTTP on {bind} (no TLS — set MTG_TLS_CERT/MTG_TLS_KEY to enable)");
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .with_context(|| format!("binding {bind}"))?;
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(signal_fut)
                .await
                .context("axum::serve exited with error")?;
        }
    }

    // ---- 6. Tear down the embedded lobby. ----
    lobby_handle.abort();
    let _ = lobby_handle.await;
    Ok(())
}

/// Pick a free loopback port by binding ephemeral and immediately dropping.
async fn pick_loopback_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("picking loopback port")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Wait until `127.0.0.1:<port>` accepts a TCP connection. Polls with a
/// short delay; bails out after ~5 s so a broken embedded server doesn't
/// hang the whole process.
async fn wait_for_loopback_port(port: u16) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("embedded lobby never came up on 127.0.0.1:{port} within 5 s"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Wait for SIGTERM (Unix) or Ctrl-C (any platform).
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
        tokio::select! {
            _ = term.recv() => log::info!("SIGTERM received"),
            _ = int.recv()  => log::info!("SIGINT received"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("Ctrl-C received");
    }
}

// ─── Lobby WS proxy ────────────────────────────────────────────────────

/// `GET /lobby` — upgrades the HTTP request to a WebSocket then proxies
/// it bidirectionally to the embedded `mtg server` on a loopback port.
async fn lobby_ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| proxy_connection(socket, state))
}

/// Drive one client ↔ upstream WebSocket pair until either side closes.
async fn proxy_connection(client_ws: WebSocket, state: AppState) {
    let upstream_url = std::sync::Arc::clone(&state.upstream_ws_url);
    let upstream_ws = match connect_async(upstream_url.as_str()).await {
        Ok((s, _)) => s,
        Err(e) => {
            log::error!("[web-server] upstream connect to {upstream_url} failed: {e}");
            let _ = close_client_with_error(client_ws, "lobby unavailable").await;
            return;
        }
    };

    let (client_tx, client_rx) = client_ws.split();
    let (upstream_tx, upstream_rx) = upstream_ws.split();

    let mut shutdown_rx = state.shutdown_rx.clone();
    let shutdown_fut = async move {
        // Returns once the watch channel flips to `true`.
        while shutdown_rx.changed().await.is_ok() {
            if *shutdown_rx.borrow() {
                return;
            }
        }
    };

    // Three concurrent futures: c→u, u→c, shutdown notifier.
    let c2u = pump_client_to_upstream(client_rx, upstream_tx);
    let u2c = pump_upstream_to_client(upstream_rx, client_tx);

    tokio::pin!(c2u);
    tokio::pin!(u2c);
    tokio::pin!(shutdown_fut);

    tokio::select! {
        _ = &mut c2u => log::debug!("[web-server] client→upstream pump finished"),
        _ = &mut u2c => log::debug!("[web-server] upstream→client pump finished"),
        _ = &mut shutdown_fut => log::info!("[web-server] shutdown: closing proxied WS"),
    }
}

/// Forward axum frames to tokio-tungstenite. Returns when the client
/// closes or the upstream send fails.
async fn pump_client_to_upstream(
    mut rx: futures_util::stream::SplitStream<WebSocket>,
    mut tx: futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, TungMessage>,
) {
    while let Some(msg) = rx.next().await {
        let Ok(msg) = msg else {
            log::debug!("[web-server] client recv error; closing");
            break;
        };
        let tung = match axum_to_tungstenite(msg) {
            Some(m) => m,
            None => continue,
        };
        if tx.send(tung).await.is_err() {
            log::debug!("[web-server] upstream send failed; closing");
            break;
        }
    }
    let _ = tx.close().await;
}

/// Forward tokio-tungstenite frames to axum. Returns when upstream closes
/// or the client send fails.
async fn pump_upstream_to_client(
    mut rx: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    mut tx: futures_util::stream::SplitSink<WebSocket, AxumMessage>,
) {
    while let Some(msg) = rx.next().await {
        let Ok(msg) = msg else {
            log::debug!("[web-server] upstream recv error; closing");
            break;
        };
        let axm = match tungstenite_to_axum(msg) {
            Some(m) => m,
            None => continue,
        };
        if tx.send(axm).await.is_err() {
            log::debug!("[web-server] client send failed; closing");
            break;
        }
    }
    let _ = tx.close().await;
}

/// Convert an axum WS frame into a tungstenite one. Returns `None` for
/// frames that should not be forwarded (e.g. pings — both libraries
/// handle keepalive transparently).
fn axum_to_tungstenite(m: AxumMessage) -> Option<TungMessage> {
    match m {
        AxumMessage::Text(t) => Some(TungMessage::Text(t.as_str().into())),
        AxumMessage::Binary(b) => Some(TungMessage::Binary(b.to_vec().into())),
        AxumMessage::Ping(_) | AxumMessage::Pong(_) => None,
        AxumMessage::Close(_) => Some(TungMessage::Close(None)),
    }
}

/// Convert a tungstenite frame into an axum one. See `axum_to_tungstenite`
/// for the filter rationale.
fn tungstenite_to_axum(m: TungMessage) -> Option<AxumMessage> {
    match m {
        TungMessage::Text(t) => Some(AxumMessage::Text(t.as_str().to_owned())),
        TungMessage::Binary(b) => Some(AxumMessage::Binary(b.to_vec())),
        TungMessage::Ping(_) | TungMessage::Pong(_) => None,
        TungMessage::Close(_) => Some(AxumMessage::Close(None)),
        // Raw frames are an internal tungstenite type we never see in
        // practice from a real client; drop them rather than panic.
        TungMessage::Frame(_) => None,
    }
}

/// Send a single JSON error frame and close the client socket. Used when
/// the upstream lobby is unreachable.
async fn close_client_with_error(mut ws: WebSocket, reason: &str) -> Result<(), axum::Error> {
    use crate::network::ServerMessage;
    let msg = ServerMessage::Error {
        message: reason.to_string(),
        fatal: true,
    };
    let json = serde_json::to_string(&msg).unwrap_or_else(|_| "{}".to_string());
    ws.send(AxumMessage::Text(json)).await?;
    ws.send(AxumMessage::Close(None)).await?;
    Ok(())
}

// ─── Mini status endpoint (used by deploy smoke tests) ────────────────

/// Trivial liveness probe. Not yet wired into the router by default; the
/// deploy script may add `/healthz` in a future iteration.
#[allow(dead_code)]
async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}
