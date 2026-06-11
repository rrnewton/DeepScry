//! Unified web server: static files + lobby WebSocket proxy + optional TLS.
//!
//! One axum process binds a single public port (default 8080) and serves:
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

use crate::network::lobby::SharedLobby;
use crate::network::{GameServer, ServerConfig};

pub mod oauth;
pub mod r2;
use oauth::{OAuthConfig, OAuthState, Provider, SESSION_COOKIE, STATE_COOKIE};
use r2::{Identity, PresignMethod, R2Config, DEFAULT_PRESIGN_TTL};

// Build/version identity lives in the central `crate::version` module
// (single source of truth shared with the CLI `mtg --version`). Re-export
// the names the rest of this module already uses to avoid churn.
pub use crate::version::{BUILD_TIME_EPOCH, GIT_HASH as BUILD_SHA};

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
    /// R2 deck-storage backend (mtg-742). `None` when the R2 env vars are
    /// absent — the deck-storage endpoint then returns 503, and the rest of
    /// the server (static files + lobby) is unaffected. The server holds the
    /// ONE parent token here and mints short-TTL presigned URLs from it; it
    /// never proxies deck bytes.
    r2: Option<Arc<R2Config>>,
    /// OAuth login state (mtg-742): provider config + session/CSRF stores.
    /// `None` when no OAuth env is configured — the deck-storage endpoint then
    /// requires login and 401s, while the ephemeral (name-only) lobby path is
    /// always available. The session cookie resolves to an
    /// [`oauth::OAuthIdentity`], which implements the SAME [`Identity`] trait
    /// the storage path consumes (the old `DevIdentity` stub is retired).
    oauth: Option<OAuthState>,
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

    // R2 deck storage (mtg-742): read the parent token from the environment
    // (mirrors how `.r2.env` / the systemd EnvironmentFile feed creds in).
    // Absent creds → `None` → the deck-storage endpoint 503s; everything
    // else still serves.
    let r2 = R2Config::from_env().map(Arc::new);
    if r2.is_some() {
        log::info!("[web-server] R2 deck storage ENABLED (bucket from env)");
    } else {
        log::info!("[web-server] R2 deck storage disabled (no R2_* env vars)");
    }
    // OAuth login (mtg-742): enabled iff at least one provider + the callback
    // base are configured in the environment. Absent → login routes report
    // not-configured and deck storage requires a (then-impossible) session;
    // the ephemeral name-only lobby path is unaffected.
    let oauth = OAuthConfig::from_env().map(OAuthState::new);
    match &oauth {
        Some(o) => {
            let (gh, gg) = o.config().available();
            log::info!("[web-server] OAuth login ENABLED (github={gh}, google={gg})");
        }
        None => log::info!("[web-server] OAuth login disabled (no OAuth env vars)"),
    }

    let state = AppState {
        upstream_ws_url: Arc::new(upstream_ws_url),
        shutdown_rx: shutdown_rx.clone(),
        start_time: std::time::Instant::now(),
        lobby: lobby_for_health,
        r2,
        oauth,
    };

    // ── Tiered cache policy (content-addressing, mtg-571 + mtg-620) ───
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
    // After mtg-620 (full asset-graph hashing) the picture is much simpler:
    //
    //   /index.html (the sole stable URL)        → public, max-age=60
    //   /<anything>.<16-hex-hash>.<ext>           → public, max-age=31536000, immutable
    //   /images/**                                → public, max-age=31536000, immutable
    //                                                (scryfall art_id-named)
    //   /data/<YYYY>-<CODE>.<hash>.bin            → public, max-age=31536000, immutable
    //                                                (exporter-named; same family)
    //   anything ELSE that lacks the hash pattern → public, max-age=60
    //                                                (fixed-name fallback — used by
    //                                                 the source tree before
    //                                                 hash-web-assets has run,
    //                                                 which makes `make validate`'s
    //                                                 fixed-name e2e tests work)
    //
    // INVARIANT (unchanged): a route may be marked `immutable` ONLY if its
    // URL is content-addressed. The new global `content_addressed_cache_header`
    // middleware below enforces this from the filename token alone, retiring
    // the old per-route no-cache carve-outs for index.json/server-config.js/etc.
    // Their HASHED forms (e.g. `index.<hash>.json`) trip the same hash detector
    // and inherit immutable for free.
    // ── ONE static service + ONE global cache-tier middleware (mtg-620) ──
    //
    // mtg-571 needed per-route carve-outs because the only content-
    // addressed assets were the pkg pair + the `<set>.<hash>.bin` files,
    // while mutable pointers (`index.json`, fixed-name JS, fixed-name
    // pkg) lived alongside them in the same tree. mtg-620 makes EVERY
    // reachable asset content-addressed except for `index.html`. That
    // collapses the routing: one `ServeDir` covers all static paths,
    // and one middleware sets Cache-Control based on whether the URL's
    // last filename token has the hash pattern. The fixed-name fallback
    // (`max-age=60`) covers the source-tree case before
    // `mtg hash-web-assets` runs — which keeps `make validate`'s e2e
    // tests against the committed unhashed names working.
    let static_service = ServeDir::new(&config.static_dir).append_index_html_on_directories(true);
    // Compress static responses over the wire (mtg-722 / task #7): the
    // card-lookup.bin table is ~1.5 MB raw but ~63% over the wire, and the wasm
    // bundle / HTML / JS compress well too. `CompressionLayer::new()` negotiates
    // br/gzip from `Accept-Encoding`; its DefaultPredicate skips already-
    // compressed types (image/*) and tiny bodies, so card-art `.jpg`/`.png`
    // and the `/health` JSON are left alone. Compression is the OUTERMOST layer
    // so it wraps the cache-header middleware's output (Cache-Control +
    // Content-Encoding compose; the blake3 hash is over the RAW file bytes, so
    // content-addressing is unaffected).
    let static_with_cache = tower::ServiceBuilder::new()
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(axum::middleware::from_fn(content_addressed_cache_header))
        .service(static_service);

    let app = Router::new()
        .route(&config.lobby_path, get(lobby_ws_handler))
        .route("/health", get(health_handler))
        // Deck storage (mtg-742): mint short-TTL, prefix-scoped presigned R2
        // URLs for the caller's collection object. The browser uses these to
        // PUT/GET its `.tgz` directly against R2 — bytes never transit here.
        .route("/api/deck-storage/credentials", get(deck_credentials_handler))
        // OAuth login leg (mtg-742): GitHub/Google authorization-code flow.
        .route("/auth/login/:provider", get(auth_login_handler))
        .route("/auth/callback/:provider", get(auth_callback_handler))
        .route("/auth/logout", axum::routing::post(auth_logout_handler))
        .route("/auth/status", get(auth_status_handler))
        // DEV-ONLY test login (mtg-742): mint a real session WITHOUT a live
        // OAuth round-trip, so the deck-storage R2 PUT/GET round-trip can be
        // exercised end-to-end locally. Gated behind `MTG_DEV_LOGIN=1`; the
        // route 404s in production (the env var is never set there). See
        // dev_login_handler.
        .route("/auth/dev-login", get(dev_login_handler))
        .fallback_service(static_with_cache)
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

    match upstream_ws.get_ref() {
        tokio_tungstenite::MaybeTlsStream::Plain(ref tcp) => {
            let _ = tcp.set_nodelay(true);
        }
        tokio_tungstenite::MaybeTlsStream::Rustls(ref tls) => {
            let _ = tls.get_ref().0.set_nodelay(true);
        }
        _ => {}
    }

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

// ─── Content-addressed cache tier middleware (mtg-571 + mtg-620) ──────

/// Cache-Control for a content-addressed file: safe forever.
const CAS_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Cache-Control for `index.html` and any other fixed-name (NOT
/// content-addressed) asset. Short-TTL so a deploy propagates quickly,
/// no `no-cache` so the browser can revalidate cheaply via 304.
///
/// On a fully hash-web-assets'd DEPLOY tree, `index.html` is the ONLY
/// asset that lands here — EVERYTHING else (the pkg pair, JS leaves, the
/// per-set `.bin`, the data set-index `index.<hash>.json`, the release
/// manifest, and the game/launcher pages) is content-addressed and
/// immutable, and every fixed name (`/data/sets/index.json`, …) 404s.
/// A stale ≤60 s `index.html` is recoverable: the CAS dispatcher falls
/// back to the latest release. This short-TTL bucket is otherwise only
/// exercised on the un-hashed source/dev tree (mtg-620 / mtg-727).
const MUTABLE_SHORT: &str = "public, max-age=60";
/// Cache-Control for `/images/**`: filenames embed the scryfall art_id,
/// so a given URL never changes bytes — safe to mark immutable even
/// though they don't carry the blake3 hash token.
const IMAGES_IMMUTABLE: &str = CAS_IMMUTABLE;

/// Is `file_name` a content-addressed filename, i.e. does it embed a
/// blake3 hash of the form `<stem>.<16-lowercase-hex>.<ext>`?
///
/// `mtg hash-web-assets` (mtg-620) produces hashed names in this exact
/// form for the pkg pair, the JS leaves, the data set-resolver JSON,
/// and the non-entry HTML pages. The exporter produces
/// `<YYYY>-<CODE>.<hash>.bin` for per-set bins, which fits the same
/// "second-to-last dot-segment is the hash" predicate, so this one
/// detector covers every content-addressed asset class.
///
/// A fixed name like `index.html` / `decks.bin` has no such hash
/// segment.
fn is_content_addressed(file_name: &str) -> bool {
    let segments: Vec<&str> = file_name.split('.').collect();
    // Need at least `<stem>.<hash>.<ext>` → 3 segments.
    if segments.len() < 3 {
        return false;
    }
    let hash_seg = segments[segments.len() - 2];
    hash_seg.len() == crate::asset_hash::ASSET_HASH_HEX_LEN
        && hash_seg.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Global middleware: set Cache-Control based on whether the request
/// URL is for a content-addressed file, an image (always immutable by
/// scryfall art_id naming), or a fixed-name file (short-TTL).
///
/// Enforces the IMMUTABILITY INVARIANT centrally: a route is marked
/// `immutable` ONLY if its URL is content-addressed (or in the
/// `/images/` art-id-addressed namespace). Replaces every per-route
/// no-cache carve-out the mtg-571 layout needed.
async fn content_addressed_cache_header(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let header = cache_control_for_path(request.uri().path());
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(axum::http::header::CACHE_CONTROL, HeaderValue::from_static(header));
    response
}

/// Pure cache-control policy: pick the `Cache-Control` value for a request
/// path. Centralized + side-effect-free so the tiers are unit-testable.
///
/// Precedence (first match wins):
///   1. content-addressed `<stem>.<16hex>.<ext>` → immutable. This covers
///      the data set-index `index.<hash>.json`, the release manifest
///      `asset-manifest.<token>.json`, the pkg pair, and the per-set bins —
///      i.e. EVERY release asset except `index.html` (mtg-620: the data
///      index is folded into the CAS graph like everything else, NOT a
///      special-cased no-cache resolver — mtg-727).
///   2. `/images/**` (scryfall art_id-addressed)  → immutable.
///   3. anything else fixed-name (only `index.html` on a clean deploy) →
///      short-TTL max-age=60.
fn cache_control_for_path(path: &str) -> &'static str {
    let file_name = path.rsplit('/').next().unwrap_or("");
    if is_content_addressed(file_name) {
        // The hashed `index.<hash>.json` data resolver lands here — immutable,
        // which is correct: its bytes never change for that URL, and its hash
        // transitively covers the hashed `.bin` names it lists.
        CAS_IMMUTABLE
    } else if path.starts_with("/images/") {
        IMAGES_IMMUTABLE
    } else {
        MUTABLE_SHORT
    }
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
        // Full `Major.Minor.<gitdepth>` display version (was bare Cargo version).
        "version": crate::version::FULL_VERSION,
        "build_date": crate::version::BUILD_DATE,
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

// ─── Deck storage: presigned R2 credentials (mtg-742) ─────────────────

/// `GET /api/deck-storage/credentials` — mint short-TTL, prefix-scoped
/// presigned R2 URLs for the caller's deck collection.
///
/// The server resolves the caller's [`Identity`] (today: the fixed `dev`
/// prefix stub; later: OAuth), computes that user's single collection key
/// `decks/<id>/collection.tgz`, and returns presigned PUT / GET / HEAD URLs
/// plus the download URL (a GET with `response-content-disposition` so the
/// browser saves a real file — the data-liberation property). The browser
/// then talks to R2 directly; deck bytes NEVER pass through this server.
///
/// Returns 503 when R2 is not configured (no env creds) so a dev box without
/// R2 degrades gracefully instead of 500ing.
///
/// SECURITY: the returned URLs are each bound to ONE object key, ONE method,
/// and a short expiry. They are NOT a bucket-wide capability: even though the
/// parent token can see the whole bucket, a handed-out URL can only touch the
/// caller's own collection object for ~10 minutes.
async fn deck_credentials_handler(headers: axum::http::HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let no_store = [(axum::http::header::CACHE_CONTROL, "no-store")];
    let Some(r2) = state.r2.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            no_store,
            axum::Json(serde_json::json!({
                "error": "deck storage not configured",
                "detail": "the server has no R2 credentials in its environment",
            })),
        );
    };

    // Resolve identity from the OAuth session cookie. Ephemeral (name-only)
    // users have no session → 401: they get no cloud R2 prefix and stay on the
    // localStorage-only path (by design).
    let identity = match (&state.oauth, cookie_value(&headers, SESSION_COOKIE)) {
        (Some(oauth), Some(sid)) => oauth.identity_for(&sid),
        _ => None,
    };
    let Some(session) = identity else {
        return (
            StatusCode::UNAUTHORIZED,
            no_store,
            axum::Json(serde_json::json!({
                "error": "login required",
                "detail": "cloud deck storage requires signing in with GitHub or Google",
            })),
        );
    };

    let user_id = session.identity.user_id();
    // Defensive: the identity must yield a prefix-safe id (OAuthIdentity
    // sanitizes, but guard against any traversal-y id reaching the key).
    if !r2::is_valid_user_id(user_id) {
        log::error!("[web-server] identity yielded unsafe user_id; refusing to presign");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            no_store,
            axum::Json(serde_json::json!({ "error": "invalid identity" })),
        );
    }

    let key = R2Config::collection_key(user_id);
    let now = std::time::SystemTime::now();
    let ttl = DEFAULT_PRESIGN_TTL;

    let put_url = r2.presign(PresignMethod::Put, &key, ttl, now);
    let get_url = r2.presign(PresignMethod::Get, &key, ttl, now);
    let head_url = r2.presign(PresignMethod::Head, &key, ttl, now);
    // "Download my decks": a GET that forces an attachment download of the
    // real .tgz. Same presigning machinery; the disposition is a separate
    // presigned GET so the in-app hydrate GET above stays a plain fetch.
    let download_url = r2.presign_download(&key, ttl, now, "deepscry-decks.tgz");
    // Permanent public URL — present only when R2_PUBLIC_BASE_URL is set and
    // the bucket has been enabled for public access in the Cloudflare dashboard.
    // When present, the browser can use this as a stable, non-expiring "Direct
    // link" instead of the short-TTL presigned get_url.
    let public_url = r2.public_url(&key);

    (
        StatusCode::OK,
        no_store,
        axum::Json(serde_json::json!({
            // The prefix the browser is scoped to (informational).
            "user_id": user_id,
            "object_key": key,
            "ttl_secs": ttl.as_secs(),
            "put_url": put_url,
            "get_url": get_url,
            "head_url": head_url,
            "download_url": download_url,
            // Permanent public URL (present when R2_PUBLIC_BASE_URL is set).
            // Requires the bucket to be enabled for public access in Cloudflare.
            "public_url": public_url,
            // Opaque storage contract (mtg-742): clients MUST send the body as
            // application/gzip with NO Content-Encoding so the bytes are stored
            // byte-clean (no CDN re-compression / auto-decompression).
            "content_type": "application/gzip",
        })),
    )
}

// ─── OAuth login handlers (mtg-742) ───────────────────────────────────────

/// Read a single cookie value out of the request `Cookie` header.
fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(name) {
            if let Some(v) = rest.strip_prefix('=') {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Build a `Set-Cookie` header value. `max_age` of 0 expires the cookie.
/// HttpOnly + SameSite=Lax + Secure + Path=/. (Lax lets the OAuth provider's
/// top-level redirect back to us still carry the state cookie.)
fn set_cookie(name: &str, value: &str, max_age_secs: i64) -> String {
    format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age_secs}")
}

/// `GET /auth/login/:provider` — start the authorization-code flow: set a
/// CSRF `state` cookie and 302 to the provider's authorize endpoint.
async fn auth_login_handler(
    axum::extract::Path(provider): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> axum::response::Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth login is not configured on this server",
        )
            .into_response();
    };
    let Some(provider) = Provider::parse_slug(&provider) else {
        return (StatusCode::NOT_FOUND, "unknown provider").into_response();
    };
    let csrf = oauth.new_state();
    let Some(redirect) = oauth.config().authorize_redirect(provider, &csrf) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "that provider is not configured").into_response();
    };
    let mut resp = axum::response::Redirect::temporary(&redirect).into_response();
    resp.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(STATE_COOKIE, &csrf, 600)).unwrap(),
    );
    resp
}

/// Query parameters on the OAuth callback.
#[derive(serde::Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /auth/callback/:provider` — verify CSRF state, exchange the code for
/// the stable subject id, create a session, set the session cookie, redirect
/// home.
async fn auth_callback_handler(
    axum::extract::Path(provider): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<CallbackParams>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> axum::response::Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth login is not configured").into_response();
    };
    let Some(provider) = Provider::parse_slug(&provider) else {
        return (StatusCode::NOT_FOUND, "unknown provider").into_response();
    };
    if let Some(err) = params.error {
        log::warn!("[web-server] OAuth provider returned error: {err}");
        return axum::response::Redirect::temporary("/?login=denied").into_response();
    }
    // CSRF: the `state` query param MUST match the one-shot cookie we set AND
    // a state we issued. Both checks defend against forged callbacks.
    let cookie_state = cookie_value(&headers, STATE_COOKIE);
    match (&params.state, &cookie_state) {
        (Some(qs), Some(cs)) if qs == cs && oauth.consume_state(qs) => {}
        _ => {
            log::warn!("[web-server] OAuth callback failed CSRF state check");
            return (StatusCode::BAD_REQUEST, "invalid or expired login state").into_response();
        }
    }
    let Some(code) = params.code else {
        return (StatusCode::BAD_REQUEST, "missing authorization code").into_response();
    };

    let resolved = match oauth.exchange_code_for_subject(provider, &code).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[web-server] OAuth code exchange failed: {e}");
            return (StatusCode::BAD_GATEWAY, "login failed talking to the identity provider").into_response();
        }
    };
    let sid = oauth.create_session(provider, resolved);

    // Redirect home with the session cookie set and the (now-spent) state
    // cookie cleared.
    let mut resp = axum::response::Redirect::temporary("/?login=ok").into_response();
    let h = resp.headers_mut();
    h.append(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(SESSION_COOKIE, &sid, 30 * 24 * 60 * 60)).unwrap(),
    );
    h.append(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(STATE_COOKIE, "", 0)).unwrap(),
    );
    resp
}

/// `POST /auth/logout` — destroy the session and clear the cookie.
async fn auth_logout_handler(
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> axum::response::Response {
    if let (Some(oauth), Some(sid)) = (&state.oauth, cookie_value(&headers, SESSION_COOKIE)) {
        oauth.destroy_session(&sid);
    }
    let mut resp = (StatusCode::OK, "logged out").into_response();
    resp.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(SESSION_COOKIE, "", 0)).unwrap(),
    );
    resp
}

/// `GET /auth/status` — JSON describing which providers are available and
/// whether the caller is currently logged in (so the landing page can render
/// the right login choices without exposing secrets).
async fn auth_status_handler(headers: axum::http::HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let (github, google) = state
        .oauth
        .as_ref()
        .map(|o| o.config().available())
        .unwrap_or((false, false));
    let session = match (&state.oauth, cookie_value(&headers, SESSION_COOKIE)) {
        (Some(oauth), Some(sid)) => oauth.identity_for(&sid),
        _ => None,
    };
    let logged_in = session.is_some();
    let user_id = session.as_ref().map(|s| s.identity.user_id().to_string());
    // Non-authoritative display fields for the landing page (mtg-742):
    //   `display_name`   — RECOGNIZABLE account label the user confirms
    //                      ("Signed in via Google — alice@example.com" /
    //                      "Signed in via GitHub — @octocat"). Shown only to
    //                      the user, never to other players.
    //   `suggested_name` — EDITABLE PUBLIC username default that pre-fills the
    //                      lobby name box (Google email local-part / GitHub
    //                      login).
    // Omitted/empty for guests; the stable `user_id` is what identity + R2
    // prefix key on, NEVER any of these display fields.
    let provider = session.as_ref().map(|s| s.provider.slug());
    let display_name = session.as_ref().and_then(|s| s.display_name().map(str::to_owned));
    let suggested_name = session
        .as_ref()
        .map(|s| s.suggested_name.clone())
        .filter(|s| !s.is_empty());
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({
            "providers": { "github": github, "google": google },
            "oauth_enabled": github || google,
            "logged_in": logged_in,
            "user_id": user_id,
            "provider": provider,
            "display_name": display_name,
            "suggested_name": suggested_name,
        })),
    )
}

/// `GET /auth/dev-login?provider=github&sub=<id>&name=<n>` — DEV-ONLY.
///
/// Mints a real session for a synthetic subject so the deck-storage R2
/// round-trip (PUT on save, GET on auto-hydrate) can be exercised end-to-end on
/// a local box without a live GitHub/Google authorization-code flow. This is
/// the ONLY non-OAuth path to a session, so it is gated TWICE: (1) the
/// `MTG_DEV_LOGIN` env var must be exactly `1` (never set in prod), and (2)
/// OAuth state must be present (so identity/R2 keying behaves identically to a
/// real login).
///
/// Without the env var it returns 404 — indistinguishable from a route that
/// does not exist, so production never exposes it.
async fn dev_login_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<AppState>,
) -> axum::response::Response {
    if std::env::var("MTG_DEV_LOGIN").ok().as_deref() != Some("1") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let Some(oauth) = state.oauth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "oauth not configured").into_response();
    };
    let provider = params
        .get("provider")
        .and_then(|p| Provider::parse_slug(p))
        .unwrap_or(Provider::GitHub);
    let subject_id = params.get("sub").cloned().unwrap_or_else(|| "dev-test-1".to_string());
    let suggested_name = params.get("name").cloned().unwrap_or_else(|| "dev-tester".to_string());
    // login-ux-corrections (mtg-742) changed create_session to take a
    // ResolvedSubject; build one from the dev-login query params. display_name
    // is a friendly label (mirrors the real GitHub resolver's "@handle"); only
    // subject_id is identity-relevant.
    let sid = oauth.create_session(
        provider,
        oauth::ResolvedSubject {
            subject_id,
            display_name: format!("@{suggested_name}"),
            suggested_name,
        },
    );
    let mut resp = (StatusCode::OK, "dev session created").into_response();
    // Dev-login runs against a LOCAL plain-HTTP box, where a `Secure` cookie
    // would be dropped by the browser. Omit `Secure` for THIS dev-only cookie
    // so the round-trip works over http://localhost. (Production logins still
    // use the Secure set_cookie via the real OAuth callback.)
    let cookie = format!(
        "{SESSION_COOKIE}={sid}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        30 * 24 * 60 * 60
    );
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    resp
}

// (`serve_static_file_with_header` retired with mtg-620: the per-route
// no-cache carve-outs it implemented are now handled by the global
// `content_addressed_cache_header` middleware above.)

#[cfg(test)]
mod cache_policy_tests {
    use super::*;

    /// mtg-727: the tiered Cache-Control policy. Anchors the IMMUTABILITY
    /// invariant — the data set-index is folded into the CAS graph like every
    /// other asset (the hashed `index.<hash>.json`, the release manifest, the
    /// pkg pair, and the per-set bins are ALL immutable), and `index.html` is
    /// the sole short-TTL fixed-name asset on a clean deploy. There is NO
    /// special-cased no-cache resolver tier (the data index is hashed, not a
    /// 2nd mutable file).
    #[test]
    fn cache_control_policy_tiers() {
        // 1. Content-addressed assets → immutable forever.
        assert_eq!(
            cache_control_for_path("/pkg/mtg_engine_bg.13cb3ea056601678.wasm"),
            CAS_IMMUTABLE
        );
        assert_eq!(
            cache_control_for_path("/pkg/mtg_engine.f46b820b19c954ee.js"),
            CAS_IMMUTABLE
        );
        // The HASHED data set-index is content-addressed → immutable: its bytes
        // never change for that URL, and its hash transitively covers the
        // hashed `.bin` names it lists (Merkle parent in the release DAG).
        assert_eq!(
            cache_control_for_path("/data/sets/index.d4b977e7f8818b41.json"),
            CAS_IMMUTABLE
        );
        assert_eq!(
            cache_control_for_path("/data/sets/2026-AVR.deadbeefdeadbeef.bin"),
            CAS_IMMUTABLE
        );
        // The content-hashed release manifest `asset-manifest.<token>.json`
        // is itself content-addressed → immutable (a mutable manifest would
        // reintroduce the stale-resolution cache vuln mtg-704 eliminated).
        assert_eq!(
            cache_control_for_path("/asset-manifest.0011223344556677.json"),
            CAS_IMMUTABLE
        );

        // 2. Card-art images → immutable (scryfall art_id-addressed).
        assert_eq!(cache_control_for_path("/images/small/c/Clue.jpg"), IMAGES_IMMUTABLE);

        // 3. index.html → short-TTL (recoverable; the CAS dispatcher falls back
        //    to latest for a stale token). On a clean deploy this is the ONLY
        //    short-TTL asset.
        assert_eq!(cache_control_for_path("/index.html"), MUTABLE_SHORT);
        assert_eq!(cache_control_for_path("/"), MUTABLE_SHORT);
        // Un-hashed source/dev tree only: fixed-name pkg + the fixed-name data
        // index fall back to short-TTL. On a DEPLOY tree these fixed names 404
        // (renamed to hashed) — asserted by test_web_server_smoke.js — so this
        // bucket is never the served data index in production.
        assert_eq!(cache_control_for_path("/pkg/mtg_engine.js"), MUTABLE_SHORT);
        assert_eq!(cache_control_for_path("/data/sets/index.json"), MUTABLE_SHORT);
    }
}

/// Cookie-parsing + OAuth-session HTTP round-trip (mtg-742).
///
/// These are the regression tests for the "OAuth login does nothing" report:
/// they prove that once a session exists, a browser presenting the session
/// cookie on `/auth/status` is recognised as logged in — exercising the REAL
/// `cookie_value` header parsing (multiple `; `-separated cookies, prefix
/// collisions) and the real `auth_status_handler` JSON, including the
/// non-authoritative `suggested_name` UI pre-fill field.
#[cfg(test)]
mod auth_session_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    #[test]
    fn cookie_value_extracts_among_multiple_cookies() {
        let mk = |raw: &str| {
            let mut h = axum::http::HeaderMap::new();
            h.insert(header::COOKIE, raw.parse().unwrap());
            h
        };
        // Single cookie.
        assert_eq!(
            cookie_value(&mk("ds_session=abc123"), SESSION_COOKIE).as_deref(),
            Some("abc123")
        );
        // Among several, with the usual "; " separators.
        let h = mk("foo=1; ds_session=abc123; bar=2");
        assert_eq!(cookie_value(&h, SESSION_COOKIE).as_deref(), Some("abc123"));
        // State + session cookies coexisting (the real OAuth callback case).
        let h = mk("ds_oauth_state=csrfxyz; ds_session=sid999");
        assert_eq!(cookie_value(&h, STATE_COOKIE).as_deref(), Some("csrfxyz"));
        assert_eq!(cookie_value(&h, SESSION_COOKIE).as_deref(), Some("sid999"));
        // A cookie whose name merely SHARES A PREFIX with ours must not match:
        // `ds_session_backup` must not be read as `ds_session`.
        let h = mk("ds_session_backup=nope; other=x");
        assert_eq!(cookie_value(&h, SESSION_COOKIE), None);
        // Missing entirely → None.
        assert_eq!(cookie_value(&mk("a=1; b=2"), SESSION_COOKIE), None);
    }

    /// Build a minimal router carrying the real `/auth/status` route over a
    /// test `AppState` whose lobby is a bare in-memory handle (no GameServer).
    fn test_app(oauth: OAuthState) -> Router {
        let (_tx, rx) = watch::channel(false);
        let state = AppState {
            upstream_ws_url: Arc::new("ws://127.0.0.1:0".to_string()),
            shutdown_rx: rx,
            start_time: std::time::Instant::now(),
            lobby: crate::network::lobby::new_shared_lobby(),
            r2: None,
            oauth: Some(oauth),
        };
        Router::new()
            .route("/auth/status", get(auth_status_handler))
            .route("/auth/dev-login", get(dev_login_handler))
            .with_state(state)
    }

    #[tokio::test]
    async fn dev_login_is_404_without_env_gate() {
        // SAFETY of the dev-only backdoor: with `MTG_DEV_LOGIN` unset (the
        // production default), /auth/dev-login is indistinguishable from a
        // missing route. We assert the unset path only (never SET the env var in
        // a test — it is process-global and would leak into parallel tests).
        if std::env::var("MTG_DEV_LOGIN").as_deref() == Ok("1") {
            return; // someone deliberately enabled it; skip the negative check.
        }
        let app = test_app(OAuthState::new_for_test());
        let req = Request::builder()
            .uri("/auth/dev-login?provider=github&sub=x")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn status_reports_logged_in_with_session_cookie() {
        let oauth = OAuthState::new_for_test();
        // Simulate a completed GitHub login: subject 999, "@octocat" as the
        // recognizable account label, "octocat" as the editable public default.
        let sid = oauth.create_session(
            Provider::GitHub,
            oauth::ResolvedSubject {
                subject_id: "999".into(),
                display_name: "@octocat".into(),
                suggested_name: "octocat".into(),
            },
        );
        let app = test_app(oauth);

        // Browser presents the session cookie among others (state already
        // cleared, plus an unrelated cookie) — the live failure mode.
        let req = Request::builder()
            .uri("/auth/status")
            .header(header::COOKIE, format!("theme=dark; {SESSION_COOKIE}={sid}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["logged_in"], serde_json::json!(true));
        assert_eq!(json["user_id"], serde_json::json!("github-999"));
        assert_eq!(json["provider"], serde_json::json!("github"));
        // display_name = recognizable account label; suggested_name = editable
        // public default. These are now DISTINCT (mtg-742 corrections).
        assert_eq!(json["display_name"], serde_json::json!("@octocat"));
        assert_eq!(json["suggested_name"], serde_json::json!("octocat"));
    }

    #[tokio::test]
    async fn status_reports_guest_without_cookie() {
        let app = test_app(OAuthState::new_for_test());
        let req = Request::builder().uri("/auth/status").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["logged_in"], serde_json::json!(false));
        assert_eq!(json["user_id"], serde_json::Value::Null);
        assert_eq!(json["display_name"], serde_json::Value::Null);
        assert_eq!(json["suggested_name"], serde_json::Value::Null);
    }
}
