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

use crate::network::lobby::SharedLobby;
use crate::network::{GameServer, ServerConfig};

/// Compile-time git short SHA (from `build.rs`). `"unknown"` if `git`
/// was unavailable at build time (e.g. tarball release).
pub const BUILD_SHA: &str = match option_env!("MTG_BUILD_SHA") {
    Some(s) => s,
    None => "unknown",
};

/// Compile-time build timestamp (Unix epoch seconds, as a string).
/// `"0"` if unavailable.
pub const BUILD_TIME_EPOCH: &str = match option_env!("MTG_BUILD_TIME_EPOCH") {
    Some(s) => s,
    None => "0",
};

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
    /// Process start time — `/health` reports `uptime_secs` derived
    /// from this.
    start_time: std::time::Instant,
    /// Shared lobby handle — `/health` reads `active_games` /
    /// `waiting_games` counts from here. Read-only access from the
    /// HTTP side; never mutated.
    lobby: SharedLobby,
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
    // Build the server FIRST (cheap; no I/O) so we can clone its
    // `SharedLobby` handle for the `/health` endpoint, then move the
    // server into the spawned task to run.
    let mut server = GameServer::new(config.lobby_config.clone());
    let lobby_for_health: SharedLobby = server.lobby_handle();
    let lobby_handle = tokio::spawn(async move {
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
        start_time: std::time::Instant::now(),
        lobby: lobby_for_health,
    };

    // ── Tiered cache policy (content-addressing, mtg-571) ─────────────
    //
    // The stale-WASM bug class (mtg-475 / mtg-2indh) is the JS-glue ↔ .wasm
    // desync: an old cached glue paired with a new wasm (or vice versa)
    // yields the cryptic "WebAssembly.instantiate(): Import #N
    // __wbindgen_cast_<hash>: function import requires a callable" error.
    // The structural fix is CONTENT-ADDRESSED filenames — a content change
    // produces a NEW filename, so a browser can never hold a stale-but-
    // mismatched copy under an old name, and such files are safe to mark
    // `immutable, max-age=1y` even behind a CDN that overrides headers
    // (because the only way to get new bytes is a new URL).
    //
    // Tiering (immutable iff the URL is content-addressed):
    //
    //   /data/**/*.bin              → public, max-age=31536000, immutable
    //                                 NOW content-addressed: the exporter
    //                                 writes `<YYYY>-<CODE>.<hash>.bin` and
    //                                 the hashed name lives in index.json's
    //                                 `file`/`cards` fields (mtg-571). The
    //                                 client only ever fetches names it read
    //                                 from index.json, so a content change is
    //                                 a new URL — immutable is correct.
    //   /data/sets/index.json       → no-cache, must-revalidate
    //                                 The MUTABLE pointer to the hashed bins.
    //                                 Small (tens of KB); cheap 304 per hit.
    //   /pkg/*                      → no-cache, must-revalidate
    //                                 NOT yet content-addressed: the game
    //                                 pages still `import init, {…named…}
    //                                 from './pkg/mtg_forge_rs.js'` with a
    //                                 FIXED specifier (trunk's rel="rust"
    //                                 multi-page rewrite is the deferred
    //                                 follow-up — see mtg-571). Until those
    //                                 pages are rewritten to a hashed glue
    //                                 name, /pkg stays no-cache + the
    //                                 deploy-time `?v=<sha>` query-string
    //                                 cache-bust so glue+wasm swap together.
    //   /images/**                  → public, max-age=31536000, immutable
    //                                 (filenames embed scryfall art_id;
    //                                  they never change for a given URL)
    //   HTML, server-config.js, etc → public, max-age=60 (mutable pointers)
    //
    // INVARIANT: a route may be marked `immutable` ONLY if its URL is
    // content-addressed (the bytes uniquely determine the filename). Adding
    // an immutable tier for a fixed-name asset re-opens the desync bug.
    use tower::ServiceBuilder;
    let pkg_dir = config.static_dir.join("pkg");
    let data_dir = config.static_dir.join("data");
    let images_dir = config.static_dir.join("images");
    let general_service = ServeDir::new(&config.static_dir).append_index_html_on_directories(true);

    let no_cache = SetResponseHeaderLayer::overriding(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, must-revalidate"),
    );
    let immutable_year = SetResponseHeaderLayer::overriding(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    let pkg_service = ServiceBuilder::new()
        .layer(no_cache.clone())
        .service(ServeDir::new(&pkg_dir));

    // /data is split: the small index.json manifest is the MUTABLE pointer
    // (no-cache; revalidate every hit, cheap 304 because it's a few KB)
    // while the large per-set .bin / decks.bin / tokens.bin bundles are now
    // CONTENT-ADDRESSED (mtg-571) — `<set>.<hash>.bin` names referenced
    // from index.json — so they are safe to mark `immutable, max-age=1y`.
    // The index.json case wins via a dedicated axum `.route()` (more
    // specific than the nest_service); everything else under /data lands on
    // the immutable bundle service — which is correct ONLY for the
    // content-addressed `<set>.<hash>.bin` files.
    //
    // decks.bin / tokens.bin are FIXED-NAME (referenced by a literal
    // `fetch('./data/decks.bin')` in the pages), so per the immutable
    // INVARIANT above they must NOT be immutable. They get dedicated
    // no-cache routes alongside index.json (mutable pointers). Hashing them
    // too is a tracked mtg-571 follow-up; until then no-cache keeps them
    // honest across a card-DB re-export.
    let data_bins_service = ServiceBuilder::new()
        .layer(immutable_year.clone())
        .service(ServeDir::new(&data_dir));
    let data_index_path = data_dir.join("sets").join("index.json");
    let data_decks_path = data_dir.join("decks.bin");
    let data_tokens_path = data_dir.join("tokens.bin");
    let index_no_cache_header = HeaderValue::from_static("no-cache, must-revalidate");

    let images_service = ServiceBuilder::new()
        .layer(immutable_year.clone())
        .service(ServeDir::new(&images_dir));

    let app = Router::new()
        .route(&config.lobby_path, get(lobby_ws_handler))
        .route("/health", get(health_handler))
        .nest_service("/pkg", pkg_service)
        // The MORE-SPECIFIC route wins in axum's matching: a request to
        // /data/sets/index.json gets the dedicated no-cache handler,
        // anything else under /data goes to data_bins_service (daily).
        .route(
            "/data/sets/index.json",
            get({
                let path = data_index_path.clone();
                let hdr = index_no_cache_header.clone();
                move || serve_static_file_with_header(path.clone(), hdr.clone(), "application/json")
            }),
        )
        // decks.bin / tokens.bin: fixed-name (NOT content-addressed) -> the
        // mutable-pointer no-cache tier, NOT the immutable bin tier below.
        .route(
            "/data/decks.bin",
            get({
                let path = data_decks_path.clone();
                let hdr = index_no_cache_header.clone();
                move || serve_static_file_with_header(path.clone(), hdr.clone(), "application/octet-stream")
            }),
        )
        .route(
            "/data/tokens.bin",
            get({
                let path = data_tokens_path.clone();
                let hdr = index_no_cache_header.clone();
                move || serve_static_file_with_header(path.clone(), hdr.clone(), "application/octet-stream")
            }),
        )
        .nest_service("/data", data_bins_service)
        .nest_service("/images", images_service)
        .fallback_service(general_service)
        // Default cache for HTML / server-config.js / etc. (`if_not_present`
        // so it does not override the per-route headers above).
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

// ─── Status endpoints (ops + deploy probes) ───────────────────────────

/// `GET /health` — JSON liveness + identity probe.
///
/// Returns the build SHA, build timestamp, package version, current
/// uptime, and live lobby counts. Used by:
///   * `scripts/deploy-cloud.sh` post-deploy probe to confirm the
///     freshly-rsynced binary is actually the one running (sha match).
///   * Operators eyeballing "what's deployed" without SSH.
///   * Future external monitors.
///
/// Cheap — touches a single Mutex for the lobby counts and never blocks.
async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let uptime_secs = state.start_time.elapsed().as_secs();
    let (active, waiting) = {
        // Hold the mutex for the minimum span needed to copy two `usize`.
        let l = state.lobby.lock().await;
        (l.active_count(), l.waiting_count())
    };

    let body = serde_json::json!({
        "sha": BUILD_SHA,
        "build_time_epoch": BUILD_TIME_EPOCH,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
        "active_games": active,
        "waiting_games": waiting,
    });

    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        axum::Json(body),
    )
}

/// Serve a single file from disk with a fixed `Cache-Control` header
/// and `Content-Type`. Used for the `/data/sets/index.json` carve-out
/// where we want no-cache semantics on a single file inside an
/// otherwise daily-cached `/data` tree.
///
/// Reads the file fresh per request (small JSON file, < 100 KB, OS
/// page cache makes this near-free). Returns 500 if the read fails.
async fn serve_static_file_with_header(
    path: PathBuf,
    cache_control: HeaderValue,
    content_type: &'static str,
) -> impl IntoResponse {
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (axum::http::header::CACHE_CONTROL, cache_control),
                (axum::http::header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            log::warn!("[web-server] failed to read {path:?}: {e}");
            (StatusCode::NOT_FOUND, format!("not found: {path:?}\n")).into_response()
        }
    }
}
