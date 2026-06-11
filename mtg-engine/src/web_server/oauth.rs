//! OAuth 2.0 authorization-code login for GitHub and Google (mtg-742).
//!
//! ## What this gives us
//!
//! A logged-in user gets a STABLE identity — `<provider>:<subject-id>`
//! (GitHub numeric user id, or Google `sub` claim) — which drives the
//! per-user R2 deck prefix (`decks/<provider>-<sub>/`). That plugs straight
//! into the [`crate::web_server::r2::Identity`] seam the storage leg already
//! uses, so the presigned-credential minting is unchanged: the server still
//! holds only the parent R2 token and never proxies deck bytes.
//!
//! ## Flow (authorization-code, NOT device flow)
//!
//! 1. `GET /auth/login/<provider>` → set a short-lived signed `state` cookie
//!    (CSRF) and 302-redirect to the provider's authorize endpoint.
//! 2. Provider redirects back to `OAUTH_CALLBACK_BASE/<provider>?code&state`.
//! 3. `GET /auth/callback/<provider>` → verify `state`, exchange `code` for an
//!    access token (server-to-server, client secret stays on the server),
//!    fetch the stable subject id, mint a SESSION, set an HttpOnly session
//!    cookie, and redirect home.
//! 4. The deck-storage endpoint resolves the session cookie → identity.
//! 5. `POST /auth/logout` drops the session.
//!
//! ## Identity is still pluggable
//!
//! The session yields an [`OAuthIdentity`] implementing the SAME `Identity`
//! trait as the old `DevIdentity` stub. Nothing in the storage path changed;
//! we just swapped where the prefix comes from.
//!
//! ## Secrets
//!
//! Client ids/secrets come from the environment ONLY (mirrors `.r2.env` /
//! `.deepscry-deploy.env`); never hardcoded, never logged. When the env is
//! absent the OAuth routes report "not configured" and the rest of the
//! server is unaffected — the ephemeral (name-only) lobby path always works.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;

use super::r2::Identity;

/// How long a login `state` (CSRF nonce) is valid before the callback must
/// arrive. Generous enough for a slow human, short enough to bound replay.
const STATE_TTL: Duration = Duration::from_secs(10 * 60);
/// How long a session stays valid without activity.
const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days
/// Session cookie name.
pub const SESSION_COOKIE: &str = "ds_session";
/// CSRF state cookie name.
pub const STATE_COOKIE: &str = "ds_oauth_state";

/// The two supported identity providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
    Google,
}

impl Provider {
    /// Parse the `<provider>` path segment. Unknown → `None`.
    pub fn parse_slug(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Provider::GitHub),
            "google" => Some(Provider::Google),
            _ => None,
        }
    }

    /// Short, URL/prefix-safe slug used in the R2 key and `<provider>:<sub>`.
    pub fn slug(self) -> &'static str {
        match self {
            Provider::GitHub => "github",
            Provider::Google => "google",
        }
    }

    fn authorize_url(self) -> &'static str {
        match self {
            Provider::GitHub => "https://github.com/login/oauth/authorize",
            Provider::Google => "https://accounts.google.com/o/oauth2/v2/auth",
        }
    }

    fn token_url(self) -> &'static str {
        match self {
            Provider::GitHub => "https://github.com/login/oauth/access_token",
            Provider::Google => "https://oauth2.googleapis.com/token",
        }
    }

    /// OAuth scopes we request: just enough to read a stable account id, no
    /// repo / email / Drive access.
    fn scope(self) -> &'static str {
        match self {
            // `read:user` would also give the profile; we only need the id, but
            // GitHub requires a scope to return a useful /user response.
            Provider::GitHub => "read:user",
            // `openid` yields the stable `sub`; `email` adds the email claim to
            // the id_token so we can derive a friendly suggested display name
            // (local-part) for UI pre-fill. Both ride in the id_token — no
            // userinfo round-trip needed.
            Provider::Google => "openid email",
        }
    }
}

/// Per-provider client credentials, from the environment.
#[derive(Clone)]
struct ProviderCreds {
    client_id: String,
    client_secret: String,
}

/// OAuth configuration assembled from env vars at server start.
#[derive(Clone)]
pub struct OAuthConfig {
    github: Option<ProviderCreds>,
    google: Option<ProviderCreds>,
    /// Base URL the provider redirects back to; the per-provider callback is
    /// `<callback_base>/<provider>`. e.g. `https://deepscry.net/auth/callback`.
    callback_base: String,
}

impl std::fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthConfig")
            .field("github", &self.github.as_ref().map(|_| "<configured>"))
            .field("google", &self.google.as_ref().map(|_| "<configured>"))
            .field("callback_base", &self.callback_base)
            .finish()
    }
}

impl OAuthConfig {
    /// Build from env. Returns `None` only if NEITHER provider is configured
    /// or the callback base is missing — i.e. OAuth login is simply off and
    /// the ephemeral path still works.
    pub fn from_env() -> Option<Self> {
        let callback_base = non_empty_env("OAUTH_CALLBACK_BASE")?.trim_end_matches('/').to_string();
        let github = creds_from_env("GITHUB_OAUTH_CLIENT_ID", "GITHUB_OAUTH_CLIENT_SECRET");
        let google = creds_from_env("GOOGLE_OAUTH_CLIENT_ID", "GOOGLE_OAUTH_CLIENT_SECRET");
        if github.is_none() && google.is_none() {
            return None;
        }
        Some(Self {
            github,
            google,
            callback_base,
        })
    }

    fn creds(&self, provider: Provider) -> Option<&ProviderCreds> {
        match provider {
            Provider::GitHub => self.github.as_ref(),
            Provider::Google => self.google.as_ref(),
        }
    }

    fn callback_url(&self, provider: Provider) -> String {
        format!("{}/{}", self.callback_base, provider.slug())
    }

    /// Build the provider's authorize-redirect URL for a given CSRF state.
    pub fn authorize_redirect(&self, provider: Provider, state: &str) -> Option<String> {
        let creds = self.creds(provider)?;
        let redirect_uri = self.callback_url(provider);
        let q = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
        let mut url = format!(
            "{}?client_id={}&redirect_uri={}&scope={}&state={}&response_type=code",
            provider.authorize_url(),
            q(&creds.client_id),
            q(&redirect_uri),
            q(provider.scope()),
            q(state),
        );
        // Google needs explicit response_type=code (already set) and benefits
        // from prompt=select_account so a user can switch accounts.
        if provider == Provider::Google {
            url.push_str("&prompt=select_account");
        }
        Some(url)
    }

    /// Which providers are available (for the login UI to show/hide buttons).
    pub fn available(&self) -> (bool, bool) {
        (self.github.is_some(), self.google.is_some())
    }
}

/// A logged-in identity backed by an OAuth subject. Implements the same
/// [`Identity`] trait the storage leg consumes, so the R2 prefix derivation
/// is unchanged.
#[derive(Debug, Clone)]
pub struct OAuthIdentity {
    /// `<provider>-<subject-id>`, sanitized to the prefix-safe charset.
    user_id: String,
}

impl OAuthIdentity {
    fn new(provider: Provider, subject_id: &str) -> Self {
        // Compose `<provider>-<sub>` and sanitize to [a-z0-9_-] so it is a
        // valid R2 key prefix (super::r2::is_valid_user_id). Provider subject
        // ids are numeric (GitHub) or an opaque `sub` (Google) — both map
        // cleanly; any stray char becomes '_'.
        let raw = format!("{}-{}", provider.slug(), subject_id);
        let user_id: String = raw
            .chars()
            .map(|c| {
                let c = c.to_ascii_lowercase();
                if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Self { user_id }
    }
}

impl Identity for OAuthIdentity {
    fn user_id(&self) -> &str {
        &self.user_id
    }
}

/// A live session: the identity plus an expiry.
#[derive(Clone)]
struct Session {
    provider: Provider,
    subject_id: String,
    /// Friendly provider handle (GitHub `login`, or the local-part of the
    /// Google email / the OIDC `name`) used ONLY to pre-fill the lobby
    /// display-name box. NON-AUTHORITATIVE: nothing security-relevant keys on
    /// it — the stable `subject_id` drives identity + the R2 prefix. May be
    /// empty when the provider gave us nothing usable.
    suggested_name: String,
    expires: Instant,
}

/// The resolved-session view the web layer needs: the stable identity plus the
/// non-authoritative suggested display name for UI pre-fill.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub identity: OAuthIdentity,
    /// Friendly handle for pre-filling the lobby name box (may be empty).
    pub suggested_name: String,
}

/// What `exchange_code_for_subject` resolves: the provider's stable subject id
/// plus a friendly handle for UI pre-fill (empty if the provider gave none).
#[derive(Debug, Clone)]
pub struct ResolvedSubject {
    pub subject_id: String,
    pub suggested_name: String,
}

/// In-memory session + CSRF-state store. Sessions are intentionally
/// process-local (a restart logs everyone out, which is acceptable and
/// avoids a persistence dependency); deck data itself is durable in R2.
#[derive(Clone)]
pub struct OAuthState {
    config: Arc<OAuthConfig>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    states: Arc<Mutex<HashMap<String, Instant>>>, // CSRF nonces → expiry
    rng: Arc<SystemRandom>,
}

impl OAuthState {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config: Arc::new(config),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            states: Arc::new(Mutex::new(HashMap::new())),
            rng: Arc::new(SystemRandom::new()),
        }
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    /// Test-only: build a fully-configured `OAuthState` (both providers, dummy
    /// creds) without touching the environment, so the web layer's HTTP-level
    /// session/cookie round-trip can be exercised hermetically.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self::new(OAuthConfig {
            github: Some(ProviderCreds {
                client_id: "test-id".into(),
                client_secret: "test-secret".into(),
            }),
            google: None,
            callback_base: "https://example.com/auth/callback".into(),
        })
    }

    /// Generate a fresh random URL-safe token (CSRF state or session id).
    fn random_token(&self) -> String {
        let mut buf = [0u8; 32];
        // SystemRandom::fill only errors if the OS RNG is unavailable, which
        // is fatal for an auth system — fall back to a clearly-invalid token
        // rather than panic in a request handler.
        if self.rng.fill(&mut buf).is_err() {
            return String::new();
        }
        hex32(&buf)
    }

    /// Mint a CSRF state nonce, remembering it for [`STATE_TTL`].
    pub fn new_state(&self) -> String {
        let token = self.random_token();
        if token.is_empty() {
            return token;
        }
        let mut states = self.states.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_expired_states(&mut states);
        states.insert(token.clone(), Instant::now() + STATE_TTL);
        token
    }

    /// Consume (one-shot) a CSRF state nonce. Returns true iff it was present
    /// and unexpired.
    pub fn consume_state(&self, state: &str) -> bool {
        let mut states = self.states.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_expired_states(&mut states);
        match states.remove(state) {
            Some(exp) => exp > Instant::now(),
            None => false,
        }
    }

    /// Create a session for a verified subject; returns the session id to set
    /// as a cookie. `suggested_name` is the non-authoritative friendly handle
    /// for UI pre-fill (pass an empty string when none is available).
    pub fn create_session(&self, provider: Provider, subject_id: String, suggested_name: String) -> String {
        let sid = self.random_token();
        if sid.is_empty() {
            return sid;
        }
        let mut sessions = self.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_expired_sessions(&mut sessions);
        sessions.insert(
            sid.clone(),
            Session {
                provider,
                subject_id,
                suggested_name,
                expires: Instant::now() + SESSION_TTL,
            },
        );
        sid
    }

    /// Resolve a session id to its [`SessionInfo`] (stable identity + the
    /// non-authoritative suggested display name), if valid + unexpired.
    pub fn identity_for(&self, session_id: &str) -> Option<SessionInfo> {
        let sessions = self.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let s = sessions.get(session_id)?;
        if s.expires <= Instant::now() {
            return None;
        }
        Some(SessionInfo {
            identity: OAuthIdentity::new(s.provider, &s.subject_id),
            suggested_name: s.suggested_name.clone(),
        })
    }

    /// Drop a session (logout).
    pub fn destroy_session(&self, session_id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session_id);
    }

    /// Exchange an authorization `code` for the provider's stable subject id.
    /// Server-to-server; the client secret never reaches the browser.
    ///
    /// # Errors
    ///
    /// Returns `Err(message)` if the provider is not configured, the network
    /// request fails, the provider returns a non-success status, or the
    /// response cannot be decoded into a usable subject id.
    pub async fn exchange_code_for_subject(&self, provider: Provider, code: &str) -> Result<ResolvedSubject, String> {
        let creds = self
            .config
            .creds(provider)
            .ok_or_else(|| "provider not configured".to_string())?;
        let redirect_uri = self.config.callback_url(provider);
        let client = reqwest::Client::new();

        // --- 1. code → access token ---
        let token_resp = client
            .post(provider.token_url())
            .header("Accept", "application/json")
            .form(&[
                ("client_id", creds.client_id.as_str()),
                ("client_secret", creds.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri.as_str()),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .map_err(|e| format!("token request failed: {e}"))?;
        if !token_resp.status().is_success() {
            return Err(format!("token endpoint returned {}", token_resp.status()));
        }
        let token: TokenResponse = token_resp
            .json()
            .await
            .map_err(|e| format!("token decode failed: {e}"))?;

        // --- 2. token → stable subject id ---
        match provider {
            Provider::GitHub => {
                let access = token
                    .access_token
                    .ok_or_else(|| "no access_token from GitHub".to_string())?;
                let user: GitHubUser = client
                    .get("https://api.github.com/user")
                    .header("Authorization", format!("Bearer {access}"))
                    .header("Accept", "application/vnd.github+json")
                    .header("User-Agent", "deepscry")
                    .send()
                    .await
                    .map_err(|e| format!("userinfo request failed: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("userinfo decode failed: {e}"))?;
                // `login` is the @handle — friendly, but the user can rename it,
                // so it is UI sugar only; the numeric `id` is the stable key.
                Ok(ResolvedSubject {
                    subject_id: user.id.to_string(),
                    suggested_name: user.login.unwrap_or_default(),
                })
            }
            Provider::Google => {
                // Google returns an OIDC id_token (a JWT) whose `sub` claim is
                // the stable subject. We requested only `openid`, so the
                // id_token is present and we read its payload WITHOUT needing a
                // userinfo round-trip. (We trust it because it arrived over the
                // server-to-server TLS token exchange we just authenticated.)
                let id_token = token.id_token.ok_or_else(|| "no id_token from Google".to_string())?;
                let claims = jwt_claims(&id_token).ok_or_else(|| "no sub in id_token".to_string())?;
                Ok(ResolvedSubject {
                    subject_id: claims.subject_id,
                    suggested_name: claims.suggested_name,
                })
            }
        }
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct GitHubUser {
    id: u64,
    /// The mutable @handle. Friendly for UI pre-fill; never an identity key.
    login: Option<String>,
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn creds_from_env(id_key: &str, secret_key: &str) -> Option<ProviderCreds> {
    Some(ProviderCreds {
        client_id: non_empty_env(id_key)?,
        client_secret: non_empty_env(secret_key)?,
    })
}

fn hex32(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit(u32::from(b >> 4), 16).unwrap());
        s.push(char::from_digit(u32::from(b & 0xf), 16).unwrap());
    }
    s
}

fn prune_expired_states(states: &mut HashMap<String, Instant>) {
    let now = Instant::now();
    states.retain(|_, exp| *exp > now);
}

fn prune_expired_sessions(sessions: &mut HashMap<String, Session>) {
    let now = Instant::now();
    sessions.retain(|_, s| s.expires > now);
}

/// The claims we read out of a Google OIDC id_token: the stable `sub` plus a
/// non-authoritative friendly handle for UI pre-fill.
struct JwtClaims {
    subject_id: String,
    suggested_name: String,
}

/// Extract the `sub` claim (and a friendly handle) from a JWT's payload WITHOUT
/// verifying the signature. Safe here ONLY because the JWT came directly from
/// Google over the authenticated server-to-server token exchange (not from the
/// browser), so its integrity is already established by TLS + the client
/// secret. We are not using it as a bearer credential, only reading the stable
/// subject id (`sub`) and a friendly display name. Returns `None` when there is
/// no usable `sub`.
fn jwt_claims(jwt: &str) -> Option<JwtClaims> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = base64url_decode(payload_b64)?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let subject_id = json.get("sub").and_then(|v| v.as_str())?.to_owned();
    // Prefer the email local-part (before '@') as the friendly handle; fall
    // back to the OIDC `name` claim; empty string if neither is present.
    let suggested_name = json
        .get("email")
        .and_then(|v| v.as_str())
        .and_then(|e| e.split('@').next())
        .filter(|s| !s.is_empty())
        .or_else(|| json.get("name").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_owned();
    Some(JwtClaims {
        subject_id,
        suggested_name,
    })
}

/// Minimal base64url decoder (no padding) for the JWT payload segment.
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    const fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        let v = val(c)?;
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_parse_and_slug() {
        assert_eq!(Provider::parse_slug("github"), Some(Provider::GitHub));
        assert_eq!(Provider::parse_slug("google"), Some(Provider::Google));
        assert_eq!(Provider::parse_slug("facebook"), None);
        assert_eq!(Provider::GitHub.slug(), "github");
    }

    #[test]
    fn oauth_identity_prefix_is_r2_safe() {
        let id = OAuthIdentity::new(Provider::GitHub, "12345");
        assert_eq!(id.user_id(), "github-12345");
        assert!(super::super::r2::is_valid_user_id(id.user_id()));
        // Google `sub` can contain only digits in practice, but verify stray
        // characters are sanitized to keep the prefix valid.
        let g = OAuthIdentity::new(Provider::Google, "10769150350006150700719253");
        assert!(super::super::r2::is_valid_user_id(g.user_id()));
        let weird = OAuthIdentity::new(Provider::Google, "ab.cd/ef");
        assert_eq!(weird.user_id(), "google-ab_cd_ef");
        assert!(super::super::r2::is_valid_user_id(weird.user_id()));
    }

    #[test]
    fn base64url_decodes_jwt_payload() {
        // {"sub":"42"} → base64url (no padding).
        let payload = "eyJzdWIiOiI0MiJ9";
        let bytes = base64url_decode(payload).unwrap();
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "{\"sub\":\"42\"}");
        let fake_jwt = format!("header.{payload}.sig");
        let claims = jwt_claims(&fake_jwt).expect("sub present");
        assert_eq!(claims.subject_id, "42");
        // No email/name claim → empty suggested handle.
        assert_eq!(claims.suggested_name, "");
    }

    #[test]
    fn jwt_claims_derive_suggested_name() {
        // {"sub":"77","email":"alice@example.com"} → handle = local-part.
        let payload = b64url(r#"{"sub":"77","email":"alice@example.com"}"#);
        let jwt = format!("h.{payload}.s");
        let claims = jwt_claims(&jwt).expect("sub present");
        assert_eq!(claims.subject_id, "77");
        assert_eq!(claims.suggested_name, "alice");

        // No email, but a `name` claim → fall back to name.
        let payload = b64url(r#"{"sub":"77","name":"Bob Builder"}"#);
        let jwt = format!("h.{payload}.s");
        assert_eq!(jwt_claims(&jwt).unwrap().suggested_name, "Bob Builder");
    }

    /// Tiny base64url (no padding) encoder for building test JWT payloads.
    fn b64url(s: &str) -> String {
        const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = s.as_bytes();
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
            let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(TBL[(n >> 18) as usize & 63] as char);
            out.push(TBL[(n >> 12) as usize & 63] as char);
            if chunk.len() > 1 {
                out.push(TBL[(n >> 6) as usize & 63] as char);
            }
            if chunk.len() > 2 {
                out.push(TBL[n as usize & 63] as char);
            }
        }
        out
    }

    #[test]
    fn state_is_one_shot_and_consumed() {
        let cfg = OAuthConfig {
            github: Some(ProviderCreds {
                client_id: "id".into(),
                client_secret: "secret".into(),
            }),
            google: None,
            callback_base: "https://example.com/auth/callback".into(),
        };
        let st = OAuthState::new(cfg);
        let s = st.new_state();
        assert!(!s.is_empty());
        assert!(st.consume_state(&s), "first consume succeeds");
        assert!(!st.consume_state(&s), "second consume fails (one-shot)");
        assert!(!st.consume_state("never-issued"));
    }

    #[test]
    fn session_round_trip_and_logout() {
        let cfg = OAuthConfig {
            github: Some(ProviderCreds {
                client_id: "id".into(),
                client_secret: "secret".into(),
            }),
            google: None,
            callback_base: "https://example.com/auth/callback".into(),
        };
        let st = OAuthState::new(cfg);
        let sid = st.create_session(Provider::GitHub, "999".into(), "octocat".into());
        let info = st.identity_for(&sid).expect("session resolves");
        assert_eq!(info.identity.user_id(), "github-999");
        // The friendly handle rides alongside the stable identity for UI sugar.
        assert_eq!(info.suggested_name, "octocat");
        st.destroy_session(&sid);
        assert!(st.identity_for(&sid).is_none(), "logout drops session");
    }

    #[test]
    fn debug_redacts_secrets() {
        let cfg = OAuthConfig {
            github: Some(ProviderCreds {
                client_id: "myid".into(),
                client_secret: "supersecret".into(),
            }),
            google: None,
            callback_base: "https://example.com/auth/callback".into(),
        };
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("supersecret"));
        assert!(!dbg.contains("myid"));
        assert!(dbg.contains("<configured>"));
    }

    #[test]
    fn authorize_redirect_has_required_params() {
        let cfg = OAuthConfig {
            github: Some(ProviderCreds {
                client_id: "gh-id".into(),
                client_secret: "x".into(),
            }),
            google: None,
            callback_base: "https://deepscry.net/auth/callback".into(),
        };
        let url = cfg.authorize_redirect(Provider::GitHub, "csrf123").unwrap();
        assert!(url.starts_with("https://github.com/login/oauth/authorize?"));
        // client_id is percent-encoded (the '-' becomes %2D).
        assert!(url.contains("client_id=gh%2Did"));
        assert!(url.contains("state=csrf123"));
        assert!(url.contains("response_type=code"));
        // callback url is percent-encoded (NON_ALPHANUMERIC encodes '.' too).
        assert!(url.contains("redirect_uri=https%3A%2F%2Fdeepscry%2Enet%2Fauth%2Fcallback%2Fgithub"));
        // Google not configured → None.
        assert!(cfg.authorize_redirect(Provider::Google, "x").is_none());
    }
}
